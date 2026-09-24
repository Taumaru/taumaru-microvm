use std::ffi::OsString;
use std::path::PathBuf;

use inquire::{Confirm, Password, PasswordDisplayMode, Select, Text};
use taumaru_microvm::{
    MicroVmSummary, SdkError, SnapshotAddressPolicy, SnapshotCancellation, SnapshotResult,
};

use crate::cli::{SnapshotAddressPolicyArg, SnapshotArgs};
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;
use crate::output::human::SnapshotProgressRenderer;

#[derive(Clone, Debug)]
struct MachineOption {
    name: String,
    label: String,
}

impl std::fmt::Display for MachineOption {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(crate) fn escalated_child_command(
    name: Option<&str>,
    output_path: Option<&std::path::Path>,
    password: Option<&str>,
    address_policy: Option<SnapshotAddressPolicyArg>,
) -> Vec<OsString> {
    let mut command = vec![OsString::from("snapshot")];
    if let Some(name) = name {
        command.push(OsString::from(name));
    }
    if let Some(path) = output_path {
        command.push(path.as_os_str().to_owned());
    }
    if let Some(password) = password {
        command.push(OsString::from("--password"));
        command.push(OsString::from(password));
    }
    if let Some(policy) = address_policy {
        command.push(OsString::from("--address-policy"));
        command.push(OsString::from(match policy {
            SnapshotAddressPolicyArg::Preserve => "preserve",
            SnapshotAddressPolicyArg::Regenerate => "regenerate",
        }));
    }
    command
}

fn prompt_error(error: inquire::InquireError, label: &str) -> CliError {
    let normalized = error.to_string().to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::snapshot_cancelled()
    } else {
        CliError::creation(
            format!("{label} could not be completed"),
            "the interactive prompt did not return a value",
            "Check terminal input and try again",
        )
    }
}

fn validate_name(name: &str) -> Result<String, CliError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(CliError::creation(
            "MicroVM name is invalid",
            "names must contain 1-64 ASCII letters, numbers, underscores, or hyphens",
            "Choose a valid name and retry",
        ));
    }
    Ok(name.to_owned())
}

async fn select_machine(
    machines: &[MicroVmSummary],
    terminal: TerminalCapabilities,
) -> Result<String, CliError> {
    let options = machines
        .iter()
        .map(|machine| MachineOption {
            name: machine.name.clone(),
            label: format!("{} [{}]", machine.name, machine.state),
        })
        .collect::<Vec<_>>();
    Select::new("Choose a MicroVM to snapshot", options)
        .with_help_message("↑↓ move  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(super::download::prompt_render_config(terminal.color))
        .prompt()
        .map(|selected| validate_name(&selected.name))
        .map_err(|error| prompt_error(error, "MicroVM snapshot selection"))?
}

fn prompt_output_directory(
    vm_name: &str,
    terminal: TerminalCapabilities,
) -> Result<PathBuf, CliError> {
    let default_directory = std::env::current_dir().map_err(CliError::from)?;
    let default_value = default_directory.to_string_lossy().into_owned();
    let help = format!("The archive will be named {vm_name}.tmvmsnap in this folder");
    let answer = Text::new("Snapshot output folder")
        .with_default(&default_value)
        .with_help_message(&help)
        .with_render_config(super::download::prompt_render_config(terminal.color))
        .prompt()
        .map_err(|error| prompt_error(error, "Snapshot output folder prompt"))?;
    let directory = PathBuf::from(answer);
    let canonical_directory = std::fs::canonicalize(&directory).map_err(|error| {
        CliError::creation(
            "Snapshot output folder is unavailable",
            format!("{}: {error}", directory.display()),
            "Enter an existing directory path and try again",
        )
    })?;
    if !canonical_directory.is_dir() {
        return Err(CliError::creation(
            "Snapshot output folder is invalid",
            format!("{} is not a directory", canonical_directory.display()),
            "Enter an existing directory path and try again",
        ));
    }
    Ok(canonical_directory.join(format!("{vm_name}.tmvmsnap")))
}

fn prompt_address_policy(
    terminal: TerminalCapabilities,
) -> Result<SnapshotAddressPolicyArg, CliError> {
    Confirm::new("Include the source IPv4 assignments in this snapshot?")
        .with_default(false)
        .with_help_message(snapshot_policy_help())
        .with_render_config(super::download::prompt_render_config(terminal.color))
        .prompt()
        .map(snapshot_address_policy)
        .map_err(|error| prompt_error(error, "Snapshot IPv4 policy prompt"))
}

fn snapshot_policy_help() -> &'static str {
    "Yes preserves the source IPv4 addresses and can conflict on restore if they are occupied. No allocates IPv4 addresses on the restore host."
}

fn snapshot_address_policy(include_source_addresses: bool) -> SnapshotAddressPolicyArg {
    if include_source_addresses {
        SnapshotAddressPolicyArg::Preserve
    } else {
        SnapshotAddressPolicyArg::Regenerate
    }
}

fn sdk_address_policy(policy: SnapshotAddressPolicyArg) -> SnapshotAddressPolicy {
    match policy {
        SnapshotAddressPolicyArg::Preserve => SnapshotAddressPolicy::PreserveIpv4,
        SnapshotAddressPolicyArg::Regenerate => SnapshotAddressPolicy::RegenerateIpv4,
    }
}

fn prompt_password(terminal: TerminalCapabilities) -> Result<String, CliError> {
    Password::new("Snapshot password")
        .with_display_mode(PasswordDisplayMode::Masked)
        .with_help_message("Characters are masked as you type")
        .with_custom_confirmation_message("Confirm snapshot password")
        .with_custom_confirmation_error_message("Passwords do not match. Try again.")
        .with_render_config(super::download::prompt_render_config(terminal.color))
        .prompt()
        .map_err(|error| prompt_error(error, "Snapshot password prompt"))
}

fn snapshot_success_message(result: &SnapshotResult) -> String {
    format!(
        "✓ Snapshot created\n  Path: {}\n  Encrypted size: {} bytes",
        result.output_path.display(),
        result.encrypted_size_bytes
    )
}

fn map_snapshot_error(error: SdkError) -> CliError {
    match error {
        SdkError::SnapshotCancelled => CliError::snapshot_cancelled(),
        SdkError::SnapshotOutputExists { path } => CliError::conflict(
            "Snapshot destination already exists",
            format!("{} will not be overwritten", path.display()),
            "Choose a different output path and retry",
        ),
        other => CliError::creation(
            "MicroVM snapshot failed",
            other.to_string(),
            "Check the reported cause, then retry the snapshot",
        ),
    }
}

async fn require_privileged(
    context: &CliContext,
    name: Option<&str>,
    output_path: Option<&std::path::Path>,
    password: Option<&str>,
    address_policy: Option<SnapshotAddressPolicyArg>,
) -> Result<Option<u8>, CliError> {
    let command = escalated_child_command(name, output_path, password, address_policy);
    crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        false,
        crate::context::resolve_home().ok(),
        &[],
        command,
        "Run the same command with sudo or as root",
    )
    .await
}

async fn run_sdk_snapshot(
    context: &CliContext,
    name: &str,
    output_path: &std::path::Path,
    password: &str,
    address_policy: SnapshotAddressPolicy,
) -> Result<SnapshotResult, CliError> {
    let cancellation = SnapshotCancellation::new();
    let progress = SnapshotProgressRenderer::new(name, context.terminal);
    let callback_progress = progress.clone();
    let operation = context.sdk.create_snapshot_with_cancellation_and_progress(
        name,
        output_path,
        password,
        address_policy,
        cancellation.clone(),
        move |event| callback_progress.on_progress(event),
    );
    tokio::pin!(operation);
    let result = tokio::select! {
        result = &mut operation => result,
        signal = tokio::signal::ctrl_c() => {
            if signal.is_ok() { cancellation.cancel(); }
            operation.await
        }
    };
    progress.finish();
    result.map_err(map_snapshot_error)
}

pub(crate) async fn run(context: &CliContext, arguments: SnapshotArgs) -> Result<u8, CliError> {
    let mut name = match arguments.name.as_deref() {
        Some(name) => Some(validate_name(name)?),
        None => None,
    };
    if name.is_none() && !context.terminal.interactive {
        return Err(CliError::missing_value(
            "machine name",
            "<NAME>",
            "Run `microvm snapshot <NAME> --password <PASSWORD>`",
        ));
    }

    if name.is_none() {
        if let Some(exit) = require_privileged(
            context,
            None,
            None,
            arguments.password.as_deref(),
            arguments.address_policy,
        )
        .await?
        {
            return Ok(exit);
        }
        let machines = context.sdk.list_microvms().await?;
        if machines.is_empty() {
            return Err(CliError::creation(
                "No MicroVMs to snapshot",
                "this SDK home contains no created machines",
                "Run `microvm new` to create one, then retry",
            ));
        }
        name = Some(select_machine(&machines, context.terminal).await?);
    }
    let name = name.ok_or_else(|| {
        CliError::missing_value("machine name", "<NAME>", "Run `microvm snapshot <NAME>`")
    })?;
    if arguments.password.as_deref() == Some("") {
        return Err(CliError::creation(
            "Snapshot password is empty",
            "an encryption password is required",
            "Supply a non-empty `--password` or use the hidden prompt",
        ));
    }
    if arguments.password.is_none() && !context.terminal.interactive {
        return Err(CliError::missing_value(
            "snapshot password",
            "--password <PASSWORD>",
            "Run `microvm snapshot <NAME> --password <PASSWORD>`",
        ));
    }
    let address_policy = match arguments.address_policy {
        Some(policy) => policy,
        None if context.terminal.interactive => prompt_address_policy(context.terminal)?,
        None => {
            return Err(CliError::missing_value(
                "snapshot IPv4 policy",
                "--address-policy <preserve|regenerate>",
                "Run `microvm snapshot <NAME> --address-policy regenerate --password <PASSWORD>`",
            ));
        }
    };

    if let Some(exit) = require_privileged(
        context,
        Some(&name),
        arguments.output_path.as_deref(),
        arguments.password.as_deref(),
        Some(address_policy),
    )
    .await?
    {
        return Ok(exit);
    }

    let output_path = match arguments.output_path {
        Some(path) => path,
        None if context.terminal.interactive => prompt_output_directory(&name, context.terminal)?,
        None => PathBuf::from(format!("{name}.tmvmsnap")),
    };

    let password = match arguments.password {
        Some(password) => password,
        None if context.terminal.interactive => prompt_password(context.terminal)?,
        None => {
            return Err(CliError::missing_value(
                "snapshot password",
                "--password <PASSWORD>",
                "Run `microvm snapshot <NAME> --password <PASSWORD>`",
            ));
        }
    };

    eprintln!("·  Creating encrypted snapshot for {name}");
    let result = run_sdk_snapshot(
        context,
        &name,
        &output_path,
        &password,
        sdk_address_policy(address_policy),
    )
    .await?;
    println!("{}", snapshot_success_message(&result));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{
        SnapshotAddressPolicyArg, escalated_child_command, map_snapshot_error,
        snapshot_address_policy, snapshot_policy_help, snapshot_success_message, validate_name,
    };
    use std::path::PathBuf;
    use taumaru_microvm::{SdkError, SnapshotResult};

    #[test]
    fn elevated_command_preserves_name_path_and_password() {
        let command = escalated_child_command(
            Some("web-01"),
            Some(Path::new("/tmp/export.tmvmsnap")),
            Some("private"),
            Some(SnapshotAddressPolicyArg::Preserve),
        );
        assert_eq!(
            command,
            [
                OsString::from("snapshot"),
                OsString::from("web-01"),
                OsString::from("/tmp/export.tmvmsnap"),
                OsString::from("--password"),
                OsString::from("private"),
                OsString::from("--address-policy"),
                OsString::from("preserve"),
            ]
        );
    }

    #[test]
    fn forwards_password_when_elevating_an_interactive_selection() {
        let command = escalated_child_command(None, None, Some("private"), None);
        assert_eq!(
            command,
            [
                OsString::from("snapshot"),
                OsString::from("--password"),
                OsString::from("private"),
            ]
        );
    }

    #[test]
    fn success_report_contains_the_final_path_and_size_without_the_password() {
        let result = SnapshotResult {
            vm_name: "web-01".to_owned(),
            output_path: PathBuf::from("/tmp/web-01.tmvmsnap"),
            encrypted_size_bytes: 4096,
            source_was_running: true,
        };

        let message = snapshot_success_message(&result);

        assert!(message.contains("/tmp/web-01.tmvmsnap"));
        assert!(message.contains("4096 bytes"));
        assert!(!message.contains("private-passphrase"));
    }

    #[test]
    fn destination_conflict_reports_no_overwrite_and_does_not_expose_a_password() {
        let error = map_snapshot_error(SdkError::SnapshotOutputExists {
            path: PathBuf::from("/tmp/existing.tmvmsnap"),
        });
        let message = error.user_message(false);

        assert!(message.contains("will not be overwritten"));
        assert!(message.contains("/tmp/existing.tmvmsnap"));
        assert!(!message.contains("private-passphrase"));
    }

    #[test]
    fn address_policy_prompt_warns_about_restore_conflicts_and_maps_both_answers() {
        let help = snapshot_policy_help();
        assert!(help.contains("can conflict on restore"));
        assert!(help.contains("allocates IPv4 addresses on the restore host"));
        assert_eq!(
            snapshot_address_policy(true),
            SnapshotAddressPolicyArg::Preserve
        );
        assert_eq!(
            snapshot_address_policy(false),
            SnapshotAddressPolicyArg::Regenerate
        );
    }

    #[test]
    fn validates_vm_names_before_constructing_output_paths() {
        assert_eq!(validate_name("web-01").ok().as_deref(), Some("web-01"));
        assert!(validate_name("../private").is_err());
        assert!(validate_name("").is_err());
    }
}
