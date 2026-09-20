use std::fmt;

use inquire::Select;
use taumaru_microvm::{MicroVmSummary, SdkError};

use super::download::prompt_render_config;
use super::new::{resolve_name, validate_name};
use crate::cli::StartArgs;
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

pub(crate) fn escalated_child_command(name: &str) -> Vec<std::ffi::OsString> {
    vec![
        std::ffi::OsString::from("start"),
        std::ffi::OsString::from(name),
        std::ffi::OsString::from("--non-interactive"),
    ]
}

fn start_prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::start_cancelled()
    } else {
        CliError::start_failed(
            "MicroVM start selection could not be completed",
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
    let selected = Select::new("Choose a MicroVM to start", options)
        .with_help_message("↑↓ move  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(start_prompt_error)?;
    validate_name(&selected.id)
}

async fn resolve_interactive_name(
    context: &CliContext,
    arguments: &StartArgs,
) -> Result<String, CliError> {
    if arguments.name.is_some() || arguments.explicit_name.is_some() {
        return resolve_name(
            arguments.name.as_deref(),
            arguments.explicit_name.as_deref(),
        );
    }
    if !context.terminal.interactive {
        return Err(CliError::creation(
            "An interactive terminal is required",
            "no machine name was supplied and input is not interactive",
            "Run microvm start <NAME> --non-interactive with root access",
        ));
    }
    let machines = context.sdk.list_microvms().await?;
    if machines.is_empty() {
        return Err(CliError::start_empty_inventory());
    }
    prompt_machine(&machines, context.terminal).await
}

fn ssh_prefix_for(escalated: bool) -> &'static str {
    if escalated { "sudo " } else { "" }
}

fn map_start_error(name: &str, error: SdkError) -> CliError {
    match &error {
        SdkError::NotFound { .. } => CliError::start_not_found(name),
        SdkError::InvalidRequest { .. } => CliError::start_failed(
            "MicroVM name is invalid",
            &SdkError::Migration(error.to_string()),
        ),
        SdkError::LifecycleConflict { .. } => {
            CliError::start_failed("MicroVM cannot be started in its current state", &error)
        }
        _ => CliError::start_failed("MicroVM start failed", &error),
    }
}

async fn execute_start(context: &CliContext, name: &str, ssh_prefix: &str) -> Result<u8, CliError> {
    let spinner = crate::output::human::StartSpinner::new(name, context.terminal);
    let result = context.sdk.start_microvm(name).await;
    spinner.finish();
    match result {
        Ok(started) => {
            crate::output::human::write_start_result(&started, ssh_prefix, context.terminal)
                .map_err(CliError::from)?;
            Ok(0)
        }
        Err(error) => Err(map_start_error(name, error)),
    }
}

pub(crate) async fn run(context: &CliContext, arguments: StartArgs) -> Result<u8, CliError> {
    if arguments.non_interactive {
        let name = resolve_name(
            arguments.name.as_deref(),
            arguments.explicit_name.as_deref(),
        )
        .map_err(|error| match error {
            CliError::MissingValue(_) => CliError::missing_value(
                "machine name",
                "--name <NAME>",
                "Run microvm start web-01 --non-interactive",
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
        return execute_start(context, &name, "").await;
    }

    let name = resolve_interactive_name(context, &arguments).await?;
    let home = crate::context::resolve_home().ok();
    let command = escalated_child_command(&name);
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
    let escalated =
        crate::privilege::Privilege::escalated_marker(&crate::privilege::SystemPrivilege);
    execute_start(context, &name, ssh_prefix_for(escalated)).await
}

#[cfg(test)]
mod tests {
    use super::escalated_child_command;
    use super::resolve_name;

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
        let command = escalated_child_command("web-01");
        let rendered: Vec<String> = command
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["start", "web-01", "--non-interactive"]);
    }
}
