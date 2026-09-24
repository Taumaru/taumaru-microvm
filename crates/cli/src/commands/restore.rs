use std::ffi::OsString;
use std::path::{Path, PathBuf};

use inquire::Text;
use taumaru_microvm::{RestoreCancellation, RestoreRequest, RestoreResult, SdkError};

use crate::cli::RestoreArgs;
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;
use crate::output::human::RestoreProgressRenderer;

pub(crate) fn escalated_child_command(
    archive_path: Option<&Path>,
    non_interactive: bool,
) -> Vec<OsString> {
    let mut command = vec![OsString::from("restore")];
    if let Some(path) = archive_path {
        command.push(path.as_os_str().to_owned());
    }
    if non_interactive {
        command.push(OsString::from("--non-interactive"));
    }
    command
}

fn prompt_error(error: inquire::InquireError, label: &str) -> CliError {
    let normalized = error.to_string().to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::creation(
            "Snapshot restore cancelled",
            "the restore did not publish a MicroVM",
            "Run `microvm restore` again when you are ready",
        )
    } else {
        CliError::creation(
            format!("{label} could not be completed"),
            "the interactive prompt did not return a value",
            "Check terminal input and try again",
        )
    }
}

fn prompt_archive_path(terminal: TerminalCapabilities) -> Result<PathBuf, CliError> {
    let value = Text::new("Snapshot archive path")
        .with_help_message("Enter the path to the .tmvmsnap file")
        .with_render_config(super::download::prompt_render_config(terminal.color))
        .prompt()
        .map_err(|error| prompt_error(error, "Snapshot archive path prompt"))?;
    let path = PathBuf::from(value.trim());
    if path.as_os_str().is_empty() {
        return Err(CliError::creation(
            "Snapshot archive path is empty",
            "restore requires an archive file",
            "Enter the path to a `.tmvmsnap` file and retry",
        ));
    }
    Ok(path)
}

fn map_restore_error(error: SdkError) -> CliError {
    match error {
        SdkError::RestoreCancelled => CliError::creation(
            "Snapshot restore cancelled",
            "the operation-owned files and network resources were cleaned up",
            "Run `microvm restore` again when you are ready",
        ),
        SdkError::RestoreConflict { name, .. } => CliError::conflict(
            "A MicroVM with this name already exists",
            format!("the archive contains the name {name:?}; existing inventory was preserved"),
            "Remove or rename the existing MicroVM before restoring this archive",
        ),
        SdkError::SnapshotNetworkConflict { .. } => CliError::conflict(
            "Snapshot network values are unavailable",
            error.to_string(),
            "Use an archive with regenerated IPv4 values or free the conflicting addresses",
        ),
        other => CliError::creation(
            "MicroVM restore failed",
            other.to_string(),
            "Check the reported cause, then retry the restore",
        ),
    }
}

async fn run_sdk_restore(
    context: &CliContext,
    archive_path: PathBuf,
) -> Result<RestoreResult, CliError> {
    let cancellation = RestoreCancellation::new();
    let progress = RestoreProgressRenderer::new(context.terminal);
    let callback_progress = progress.clone();
    let operation = context.sdk.restore_snapshot_with_cancellation_and_progress(
        RestoreRequest { archive_path },
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
    result.map_err(map_restore_error)
}

pub(crate) async fn run(context: &CliContext, arguments: RestoreArgs) -> Result<u8, CliError> {
    let non_interactive = arguments.non_interactive || !context.terminal.interactive;
    if non_interactive && arguments.archive_path.is_none() {
        return Err(CliError::missing_value(
            "snapshot archive path",
            "<ARCHIVE>",
            "Run `microvm restore ./machine.tmvmsnap --non-interactive`",
        ));
    }

    let command =
        escalated_child_command(arguments.archive_path.as_deref(), arguments.non_interactive);
    if let Some(exit) = crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        non_interactive,
        crate::context::resolve_home().ok(),
        &[],
        command,
        "Run the same command with sudo or as root",
    )
    .await?
    {
        return Ok(exit);
    }

    let archive_path = match arguments.archive_path {
        Some(path) => path,
        None => prompt_archive_path(context.terminal)?,
    };
    let result = run_sdk_restore(context, archive_path).await?;
    crate::output::human::write_restore_result(&result, context.terminal)
        .map_err(CliError::from)?;
    Ok(0)
}
