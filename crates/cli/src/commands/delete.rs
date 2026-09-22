use std::ffi::OsString;
use std::fmt;

use inquire::{Confirm, Select};
use taumaru_microvm::{MicroVmSummary, SdkError};

use super::download::prompt_render_config;
use super::new::{resolve_name, validate_name};
use crate::cli::DeleteArgs;
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;

#[derive(Clone, Debug)]
struct MachineOption {
    id: String,
    label: String,
}

impl fmt::Display for MachineOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(crate) fn escalated_child_command(name: &str) -> Vec<OsString> {
    vec![
        OsString::from("delete"),
        OsString::from(name),
        OsString::from("--non-interactive"),
    ]
}

pub(crate) fn bare_escalated_command() -> Vec<OsString> {
    vec![OsString::from("delete")]
}

fn delete_prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::delete_cancelled()
    } else {
        CliError::delete_failed(
            "MicroVM delete selection could not be completed",
            &SdkError::Migration(message),
        )
    }
}

async fn prompt_machine(
    machines: &[MicroVmSummary],
    terminal: TerminalCapabilities,
) -> Result<String, CliError> {
    let options: Vec<MachineOption> = machines
        .iter()
        .map(|machine| MachineOption {
            id: machine.name.clone(),
            label: format!("{} [{}]", machine.name, machine.state),
        })
        .collect();
    let selected = Select::new("Choose a MicroVM to delete", options)
        .with_help_message("↑↓ move  ·  enter confirm  ·  Deletion is permanent; a running choice reports the stop step instead of deleting")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(delete_prompt_error)?;
    validate_name(&selected.id)
}

fn confirm_deletion(name: &str, terminal: TerminalCapabilities) -> Result<bool, CliError> {
    Confirm::new(&format!(
        "Delete {name}? This permanently removes its disk, credentials, and network attachment"
    ))
    .with_default(false)
    .with_help_message("Enter deletes the machine  ·  Ctrl-C cancels")
    .with_render_config(prompt_render_config(terminal.color))
    .prompt()
    .map_err(delete_prompt_error)
}

fn map_delete_error(name: &str, error: SdkError) -> CliError {
    match &error {
        SdkError::NotFound { .. } => CliError::delete_not_found(name),
        SdkError::InvalidRequest { .. } => CliError::delete_failed(
            "MicroVM name is invalid",
            &SdkError::Migration(error.to_string()),
        ),
        SdkError::LifecycleConflict { .. } => CliError::delete_running(name),
        _ => CliError::delete_failed("MicroVM delete failed", &error),
    }
}

async fn execute_delete(context: &CliContext, name: &str) -> Result<u8, CliError> {
    let spinner = crate::output::human::DeleteSpinner::new(name, context.terminal);
    let result = context.sdk.delete_microvm(name).await;
    spinner.finish();
    match result {
        Ok(deleted) => {
            crate::output::human::write_delete_result(&deleted, context.terminal)
                .map_err(CliError::from)?;
            Ok(0)
        }
        Err(error) => Err(map_delete_error(name, error)),
    }
}

async fn delete_resolved(context: &CliContext, name: &str) -> Result<u8, CliError> {
    let home = crate::context::resolve_home().ok();
    let command = escalated_child_command(name);
    if let Some(exit) = crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        false,
        home,
        &[],
        command,
        "Run the same command with sudo or as root",
    )
    .await?
    {
        return Ok(exit);
    }
    if context.terminal.interactive && !confirm_deletion(name, context.terminal)? {
        return Err(CliError::delete_cancelled());
    }
    execute_delete(context, name).await
}

pub(crate) async fn run(context: &CliContext, arguments: DeleteArgs) -> Result<u8, CliError> {
    if arguments.non_interactive {
        let name = resolve_name(
            arguments.name.as_deref(),
            arguments.explicit_name.as_deref(),
        )
        .map_err(|error| match error {
            CliError::MissingValue(_) => CliError::missing_value(
                "machine name",
                "--name <NAME>",
                "Run microvm delete web-01 --non-interactive",
            ),
            other => other,
        })?;
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            true,
            crate::context::resolve_home().ok(),
            &[],
            Vec::new(),
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
        }
        return execute_delete(context, &name).await;
    }

    if arguments.name.is_none() && arguments.explicit_name.is_none() {
        let home = crate::context::resolve_home().ok();
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            false,
            home,
            &[],
            bare_escalated_command(),
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
        }
        if !context.terminal.interactive {
            return Err(CliError::creation(
                "An interactive terminal is required",
                "no machine name was supplied and input is not interactive",
                "Run microvm delete <NAME> --non-interactive with root access",
            ));
        }
        let machines: Vec<MicroVmSummary> = context.sdk.list_microvms().await?;
        if machines.is_empty() {
            return Err(CliError::delete_empty());
        }
        let name = prompt_machine(&machines, context.terminal).await?;
        return delete_resolved(context, &name).await;
    }

    let name = resolve_name(
        arguments.name.as_deref(),
        arguments.explicit_name.as_deref(),
    )?;
    delete_resolved(context, &name).await
}

#[cfg(test)]
mod tests {
    use super::{bare_escalated_command, escalated_child_command, map_delete_error, resolve_name};

    #[test]
    fn positional_and_flag_names_agree_or_abort() {
        assert_eq!(
            resolve_name(Some("web-01"), None).expect("positional name should resolve"),
            "web-01"
        );
        assert_eq!(
            resolve_name(None, Some("web-01")).expect("flag name should resolve"),
            "web-01"
        );
        assert!(resolve_name(Some("web-01"), Some("db-01")).is_err());
    }

    #[test]
    fn invalid_names_abort_with_the_rule() {
        let error = resolve_name(Some("not path friendly"), None).expect_err("invalid name aborts");
        assert!(error.user_message(false).contains("1-64 ASCII"));
    }

    #[test]
    fn escalated_child_reruns_non_interactive() {
        let rendered: Vec<String> = escalated_child_command("web-01")
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["delete", "web-01", "--non-interactive"]);
    }

    #[test]
    fn bare_escalated_child_lists_privileged() {
        let rendered: Vec<String> = bare_escalated_command()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["delete"]);
    }

    #[test]
    fn unknown_machine_maps_to_creation_pointer() {
        use taumaru_microvm::SdkError;
        let error = map_delete_error(
            "web-01",
            SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: "web-01".to_owned(),
            },
        );
        let message = error.user_message(false);
        assert!(message.contains("web-01"));
        assert!(message.contains("microvm new"));
    }

    #[test]
    fn running_machine_maps_to_stop_pointer() {
        use taumaru_microvm::SdkError;
        let error = map_delete_error(
            "web-01",
            SdkError::LifecycleConflict {
                name: "web-01".to_owned(),
                state: "running".to_owned(),
                operation: "delete MicroVM (stop the machine first)".to_owned(),
            },
        );
        let message = error.user_message(false);
        assert!(message.contains("web-01"));
        assert!(message.contains("microvm stop web-01"));
    }

    #[test]
    fn other_failures_map_to_delete_failure() {
        use taumaru_microvm::SdkError;
        let error = map_delete_error(
            "web-01",
            SdkError::TemporaryRuntime {
                component: "firecracker".to_owned(),
                reason: "the record could not be removed".to_owned(),
                stopped: false,
            },
        );
        assert!(error.user_message(false).contains("MicroVM delete failed"));
    }

    #[test]
    fn confirmation_decline_reports_exit_130_with_nothing_deleted() {
        let error = crate::error::CliError::delete_cancelled();
        assert_eq!(error.exit_code(), 130);
        let message = error.user_message(false);
        assert!(message.contains("No machine was deleted"));
        assert!(message.contains("microvm delete"));
    }

    #[test]
    fn retryable_failure_points_at_fix_and_retry() {
        use taumaru_microvm::SdkError;
        let error = map_delete_error(
            "web-01",
            SdkError::Cleanup {
                primary: "network cleanup failed".to_owned(),
                failures: vec!["tap device busy".to_owned()],
            },
        );
        let message = error.user_message(false);
        assert!(message.contains("MicroVM delete failed"));
        assert!(message.contains("retry the delete"));
    }

    #[test]
    fn delete_cancellation_reports_exit_130() {
        let error = crate::error::CliError::delete_cancelled();
        assert_eq!(error.exit_code(), 130);
        assert!(error.user_message(false).contains("No machine was deleted"));
    }

    #[test]
    fn empty_stored_set_points_at_creation() {
        let error = crate::error::CliError::delete_empty();
        let message = error.user_message(false);
        assert!(message.contains("No MicroVMs to delete"));
        assert!(message.contains("microvm new"));
    }
}
