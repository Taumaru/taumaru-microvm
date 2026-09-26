use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;

use inquire::validator::Validation;
use inquire::{Confirm, CustomType, Select};
use taumaru_microvm::{
    AutostartPolicy, AutostartPolicyUpdate, AutostartSettings, DEFAULT_AUTOSTART_ATTEMPTS,
    MAX_AUTOSTART_ATTEMPTS, MIN_AUTOSTART_ATTEMPTS, SdkError,
};

use super::download::prompt_render_config;
use super::new::{resolve_name, validate_name};
use crate::boot::{BootState, SystemdAutostart};
use crate::cli::{
    AutostartAddArgs, AutostartArgs, AutostartCommand, AutostartEditArgs, AutostartLsArgs,
    AutostartRmArgs,
};
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;
use crate::output::human::autostart::{self as output, AutostartRow};

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

pub(crate) async fn run(context: &CliContext, arguments: AutostartArgs) -> Result<u8, CliError> {
    match arguments.command {
        AutostartCommand::Add(arguments) => add(context, arguments).await,
        AutostartCommand::Edit(arguments) => edit(context, arguments).await,
        AutostartCommand::Rm(arguments) => remove(context, arguments).await,
        AutostartCommand::Ls(arguments) => list(context, arguments).await,
        AutostartCommand::Run => run_at_boot(context).await,
    }
}

async fn add(context: &CliContext, arguments: AutostartAddArgs) -> Result<u8, CliError> {
    let name = supplied_name(
        arguments.name.as_deref(),
        arguments.explicit_name.as_deref(),
        arguments.non_interactive,
        "Run microvm autostart add web-01 --non-interactive",
    )?;
    if let Some(exit) = ensure_root(context, arguments.non_interactive).await? {
        return Ok(exit);
    }
    let boot = SystemdAutostart::default();
    boot.ensure_available()?;
    let name = match name {
        Some(name) => name,
        None => select_unconfigured_machine(context).await?,
    };
    let max_start_attempts = match arguments.max_attempts {
        Some(attempts) => attempts,
        None if prompts_allowed(context, arguments.non_interactive) => {
            prompt_attempts(DEFAULT_AUTOSTART_ATTEMPTS, context.terminal)?
        }
        None => DEFAULT_AUTOSTART_ATTEMPTS,
    };
    let settings = AutostartSettings {
        enabled: !arguments.paused,
        max_start_attempts,
    };
    let policy = context
        .sdk
        .create_autostart_policy(&name, settings)
        .await
        .map_err(|error| map_autostart_error(&name, error))?;
    let boot_state = sync_boot(context, &boot).await?;
    let heading = if policy.enabled {
        "Autostart enabled"
    } else {
        "Autostart saved paused"
    };
    output::write(&output::format_policy_result(
        heading,
        &policy,
        &boot_state,
        context.terminal,
    ))?;
    Ok(0)
}

async fn edit(context: &CliContext, arguments: AutostartEditArgs) -> Result<u8, CliError> {
    let example = "Run microvm autostart edit web-01 --pause --non-interactive";
    let name = supplied_name(
        arguments.name.as_deref(),
        arguments.explicit_name.as_deref(),
        arguments.non_interactive,
        example,
    )?;
    let flagged = flagged_update(&arguments);
    if flagged.is_empty() && !prompts_allowed(context, arguments.non_interactive) {
        return Err(CliError::missing_value(
            "autostart change",
            "--enable, --pause, or --max-attempts <N>",
            example,
        ));
    }
    if let Some(exit) = ensure_root(context, arguments.non_interactive).await? {
        return Ok(exit);
    }
    let boot = SystemdAutostart::default();
    boot.ensure_available()?;
    let name = match name {
        Some(name) => name,
        None => select_configured_machine(context, "Choose a MicroVM to edit").await?,
    };
    let update = if flagged.is_empty() {
        let current = context
            .sdk
            .autostart_policy(&name)
            .await
            .map_err(|error| map_autostart_error(&name, error))?
            .ok_or_else(|| CliError::autostart_not_configured(&name))?;
        prompt_update(&current, context.terminal)?
    } else {
        flagged
    };
    let policy = context
        .sdk
        .update_autostart_policy(&name, update)
        .await
        .map_err(|error| map_autostart_error(&name, error))?;
    let boot_state = sync_boot(context, &boot).await?;
    output::write(&output::format_policy_result(
        "Autostart updated",
        &policy,
        &boot_state,
        context.terminal,
    ))?;
    Ok(0)
}

async fn remove(context: &CliContext, arguments: AutostartRmArgs) -> Result<u8, CliError> {
    let name = supplied_name(
        arguments.name.as_deref(),
        arguments.explicit_name.as_deref(),
        arguments.non_interactive,
        "Run microvm autostart rm web-01 --non-interactive",
    )?;
    if let Some(exit) = ensure_root(context, arguments.non_interactive).await? {
        return Ok(exit);
    }
    let boot = SystemdAutostart::default();
    boot.ensure_available()?;
    let name = match name {
        Some(name) => name,
        None => {
            select_configured_machine(context, "Choose a MicroVM to stop starting at boot").await?
        }
    };
    let removed = context
        .sdk
        .delete_autostart_policy(&name)
        .await
        .map_err(|error| map_autostart_error(&name, error))?;
    let boot_state = sync_boot(context, &boot).await?;
    output::write(&output::format_removed(
        &removed,
        &boot_state,
        context.terminal,
    ))?;
    Ok(0)
}

async fn list(context: &CliContext, arguments: AutostartLsArgs) -> Result<u8, CliError> {
    if let Some(exit) = ensure_root(context, arguments.non_interactive).await? {
        return Ok(exit);
    }
    let policies = context
        .sdk
        .list_autostart_policies()
        .await
        .map_err(|error| CliError::autostart_failed("Autostart listing failed", &error))?;
    if policies.is_empty() {
        output::write(&output::format_empty(context.terminal))?;
        return Ok(0);
    }
    let states: HashMap<String, _> = context
        .sdk
        .list_microvms()
        .await
        .map_err(|error| CliError::autostart_failed("Autostart listing failed", &error))?
        .into_iter()
        .map(|machine| (machine.name, machine.state))
        .collect();
    let rows: Vec<AutostartRow> = policies
        .into_iter()
        .map(|policy| AutostartRow {
            state: states.get(&policy.name).copied(),
            policy,
        })
        .collect();
    output::write(&output::format_table(&rows, context.terminal))?;
    Ok(0)
}

async fn run_at_boot(context: &CliContext) -> Result<u8, CliError> {
    if let Some(exit) = ensure_root(context, true).await? {
        return Ok(exit);
    }
    let report = context
        .sdk
        .start_autostart_microvms()
        .await
        .map_err(|error| CliError::autostart_failed("Autostart run failed", &error))?;
    output::write(&output::format_run_report(&report, context.terminal))?;
    Ok(if report.has_failures() { 1 } else { 0 })
}

fn supplied_name(
    positional: Option<&str>,
    explicit: Option<&str>,
    non_interactive: bool,
    example: &str,
) -> Result<Option<String>, CliError> {
    if positional.is_some() || explicit.is_some() {
        return resolve_name(positional, explicit).map(Some);
    }
    if non_interactive {
        return Err(CliError::missing_value(
            "machine name",
            "--name <NAME>",
            example,
        ));
    }
    Ok(None)
}

fn prompts_allowed(context: &CliContext, non_interactive: bool) -> bool {
    !non_interactive && context.terminal.interactive
}

async fn ensure_root(context: &CliContext, non_interactive: bool) -> Result<Option<u8>, CliError> {
    let command = if non_interactive {
        Vec::new()
    } else {
        std::env::args_os().skip(1).collect::<Vec<OsString>>()
    };
    crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        non_interactive,
        crate::context::resolve_home().ok(),
        &[],
        command,
        "Run the same command with sudo or as root",
    )
    .await
}

async fn sync_boot(context: &CliContext, boot: &SystemdAutostart) -> Result<BootState, CliError> {
    let policies = context
        .sdk
        .list_autostart_policies()
        .await
        .map_err(|error| {
            CliError::autostart_failed("Autostart policies could not be read", &error)
        })?;
    let any_enabled = policies.iter().any(|policy| policy.enabled);
    let home = crate::context::resolve_home()?;
    let executable = std::env::current_exe()?;
    boot.sync(&home, &executable, any_enabled).await
}

fn require_interactive_selection(context: &CliContext) -> Result<(), CliError> {
    if context.terminal.interactive {
        return Ok(());
    }
    Err(CliError::creation(
        "An interactive terminal is required",
        "no machine name was supplied and input is not interactive",
        "Pass the machine name and --non-interactive, and run with root access",
    ))
}

async fn select_unconfigured_machine(context: &CliContext) -> Result<String, CliError> {
    require_interactive_selection(context)?;
    let machines = context.sdk.list_microvms().await?;
    if machines.is_empty() {
        return Err(CliError::creation(
            "No MicroVMs to configure",
            "no created machines exist in this home",
            "Run `microvm new` to create one, then try again",
        ));
    }
    let configured: Vec<String> = context
        .sdk
        .list_autostart_policies()
        .await?
        .into_iter()
        .map(|policy| policy.name)
        .collect();
    let options: Vec<MachineOption> = machines
        .into_iter()
        .filter(|machine| !configured.contains(&machine.name))
        .map(|machine| MachineOption {
            label: format!("{} [{}]", machine.name, machine.state),
            id: machine.name,
        })
        .collect();
    if options.is_empty() {
        return Err(CliError::creation(
            "Every MicroVM already starts at boot",
            "all created machines have an autostart policy",
            "Run `microvm autostart edit` to change one",
        ));
    }
    prompt_machine(
        "Choose a MicroVM to start at boot",
        options,
        context.terminal,
    )
}

async fn select_configured_machine(
    context: &CliContext,
    message: &str,
) -> Result<String, CliError> {
    require_interactive_selection(context)?;
    let policies = context.sdk.list_autostart_policies().await?;
    if policies.is_empty() {
        return Err(CliError::creation(
            "No MicroVMs start at boot",
            "no machine has an autostart policy",
            "Run `microvm autostart add` to configure one",
        ));
    }
    let options = policies.iter().map(policy_option).collect();
    prompt_machine(message, options, context.terminal)
}

fn policy_option(policy: &AutostartPolicy) -> MachineOption {
    MachineOption {
        id: policy.name.clone(),
        label: format!(
            "{} [{} · {} attempts]",
            policy.name,
            output::status_text(policy.enabled),
            policy.max_start_attempts
        ),
    }
}

fn prompt_machine(
    message: &str,
    options: Vec<MachineOption>,
    terminal: TerminalCapabilities,
) -> Result<String, CliError> {
    let selected = Select::new(message, options)
        .with_help_message("↑↓ move  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(prompt_error)?;
    validate_name(&selected.id)
}

fn prompt_attempts(default: u32, terminal: TerminalCapabilities) -> Result<u32, CliError> {
    CustomType::<u32>::new("Start attempts at boot")
        .with_default(default)
        .with_help_message("Retries a failed start before reporting it  ·  1-10")
        .with_error_message("Enter a whole number")
        .with_validator(|value: &u32| {
            Ok(
                if (MIN_AUTOSTART_ATTEMPTS..=MAX_AUTOSTART_ATTEMPTS).contains(value) {
                    Validation::Valid
                } else {
                    Validation::Invalid(
                        format!(
                            "Choose between {MIN_AUTOSTART_ATTEMPTS} and {MAX_AUTOSTART_ATTEMPTS}"
                        )
                        .into(),
                    )
                },
            )
        })
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(prompt_error)
}

fn prompt_update(
    current: &AutostartPolicy,
    terminal: TerminalCapabilities,
) -> Result<AutostartPolicyUpdate, CliError> {
    let enabled = Confirm::new(&format!("Start {} at boot?", current.name))
        .with_default(current.enabled)
        .with_help_message("No keeps the policy but pauses it")
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(prompt_error)?;
    let max_start_attempts = prompt_attempts(current.max_start_attempts, terminal)?;
    Ok(AutostartPolicyUpdate {
        enabled: Some(enabled),
        max_start_attempts: Some(max_start_attempts),
    })
}

fn flagged_update(arguments: &AutostartEditArgs) -> AutostartPolicyUpdate {
    let enabled = match (arguments.enable, arguments.pause) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    AutostartPolicyUpdate {
        enabled,
        max_start_attempts: arguments.max_attempts,
    }
}

fn prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::autostart_cancelled()
    } else {
        CliError::autostart_failed(
            "Autostart selection could not be completed",
            &SdkError::Migration(message),
        )
    }
}

fn map_autostart_error(name: &str, error: SdkError) -> CliError {
    match &error {
        SdkError::NotFound { kind, .. } if kind == "autostart policy" => {
            CliError::autostart_not_configured(name)
        }
        SdkError::NotFound { .. } => CliError::autostart_machine_not_found(name),
        SdkError::ConfigurationConflict { .. } => CliError::autostart_already_configured(name),
        _ => CliError::autostart_failed("Autostart settings could not be saved", &error),
    }
}

#[cfg(test)]
mod tests {
    use taumaru_microvm::SdkError;

    use super::{flagged_update, map_autostart_error, supplied_name};
    use crate::cli::AutostartEditArgs;

    fn edit_arguments(enable: bool, pause: bool, max_attempts: Option<u32>) -> AutostartEditArgs {
        AutostartEditArgs {
            name: Some("web-01".to_owned()),
            explicit_name: None,
            max_attempts,
            enable,
            pause,
            non_interactive: true,
        }
    }

    #[test]
    fn non_interactive_mode_requires_a_name() {
        let error = supplied_name(None, None, true, "Run microvm autostart add web-01")
            .expect_err("missing name should fail");
        assert!(error.user_message(false).contains("machine name"));
        assert_eq!(
            supplied_name(None, None, false, "example").expect("selector is allowed"),
            None
        );
        assert_eq!(
            supplied_name(None, Some("web-01"), true, "example").expect("flag name resolves"),
            Some("web-01".to_owned())
        );
    }

    #[test]
    fn edit_flags_map_to_partial_updates() {
        assert!(flagged_update(&edit_arguments(false, false, None)).is_empty());
        assert_eq!(
            flagged_update(&edit_arguments(false, true, None)).enabled,
            Some(false)
        );
        let resumed = flagged_update(&edit_arguments(true, false, Some(5)));
        assert_eq!(resumed.enabled, Some(true));
        assert_eq!(resumed.max_start_attempts, Some(5));
    }

    #[test]
    fn sdk_errors_map_to_actionable_messages() {
        let missing_policy = map_autostart_error(
            "web-01",
            SdkError::NotFound {
                kind: "autostart policy".to_owned(),
                id: "web-01".to_owned(),
            },
        );
        assert!(
            missing_policy
                .user_message(false)
                .contains("microvm autostart add web-01")
        );
        let missing_machine = map_autostart_error(
            "web-01",
            SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: "web-01".to_owned(),
            },
        );
        assert!(
            missing_machine
                .user_message(false)
                .contains("was not found")
        );
        let conflict = map_autostart_error(
            "web-01",
            SdkError::ConfigurationConflict {
                name: "web-01".to_owned(),
                field: "autostart.enabled".to_owned(),
                existing: "true".to_owned(),
                requested: "false".to_owned(),
            },
        );
        assert!(
            conflict
                .user_message(false)
                .contains("microvm autostart edit web-01")
        );
    }
}
