use std::ffi::OsString;
use std::fmt;

use inquire::{Confirm, Select, Text};
use taumaru_microvm::{MicroVmSummary, SdkError, UpdateMicroVmRequest};

use super::download::{SdkArtifactClient, load_catalog, prompt_render_config};
use super::new::{
    check_memory_minimum, check_vcpu_minimum, format_gb, format_gb_flag, format_mb_gb,
    format_memory_flag, parse_disk_gb, parse_memory, parse_vcpus, resolve_name, validate_name,
};
use crate::cli::EditArgs;
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

pub(crate) fn escalated_child_command(
    name: &str,
    disk_gb: Option<&str>,
    memory: Option<&str>,
    vcpus: Option<&str>,
) -> Vec<OsString> {
    let mut command = vec![
        OsString::from("edit"),
        OsString::from(name),
        OsString::from("--non-interactive"),
    ];
    if let Some(disk) = disk_gb {
        command.push(OsString::from("--disk-gb"));
        command.push(OsString::from(disk));
    }
    if let Some(memory) = memory {
        command.push(OsString::from("--memory"));
        command.push(OsString::from(memory));
    }
    if let Some(vcpus) = vcpus {
        command.push(OsString::from("--vcpus"));
        command.push(OsString::from(vcpus));
    }
    command
}

pub(crate) fn bare_escalated_command(
    disk_gb: Option<&str>,
    memory: Option<&str>,
    vcpus: Option<&str>,
) -> Vec<OsString> {
    let mut command = vec![OsString::from("edit")];
    if let Some(disk) = disk_gb {
        command.push(OsString::from("--disk-gb"));
        command.push(OsString::from(disk));
    }
    if let Some(memory) = memory {
        command.push(OsString::from("--memory"));
        command.push(OsString::from(memory));
    }
    if let Some(vcpus) = vcpus {
        command.push(OsString::from("--vcpus"));
        command.push(OsString::from(vcpus));
    }
    command
}

fn edit_prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::edit_cancelled()
    } else {
        CliError::edit_failed(
            "MicroVM edit selection could not be completed",
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
    let selected = Select::new("Choose a MicroVM to edit", options)
        .with_help_message("↑↓ move  ·  enter confirm  ·  Only stopped MicroVMs can be edited; a running choice reports the stop step instead")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(edit_prompt_error)?;
    validate_name(&selected.id)
}

pub(crate) fn check_disk_minimum_used(
    disk_size_bytes: u64,
    used_bytes: u64,
) -> Result<(), CliError> {
    if disk_size_bytes < used_bytes {
        return Err(CliError::creation(
            "Disk size is too small",
            format!(
                "requested {} is smaller than the used minimum of {}",
                format_gb(disk_size_bytes),
                format_mb_gb(used_bytes)
            ),
            format!("choose --disk-gb of at least {}", format_mb_gb(used_bytes)),
        ));
    }
    Ok(())
}

async fn prompt_disk(
    current_bytes: u64,
    used_bytes: u64,
    preset: Option<&str>,
) -> Result<(String, u64), CliError> {
    if let Some(value) = preset {
        let bytes = parse_disk_gb(value)?;
        check_disk_minimum_used(bytes, used_bytes)?;
        return Ok((value.trim().to_owned(), bytes));
    }
    let message = format!(
        "Disk size in GB (current {}, used minimum {})",
        format_gb(current_bytes),
        format_mb_gb(used_bytes)
    );
    let answer = Text::new(&message)
        .with_initial_value(&format_gb_flag(current_bytes))
        .with_help_message(
            "positive number in GB, for example 20 or 20.5; empty keeps the current value",
        )
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(edit_prompt_error)?;
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Ok((format_gb_flag(current_bytes), current_bytes));
    }
    let bytes = parse_disk_gb(trimmed)?;
    check_disk_minimum_used(bytes, used_bytes)?;
    Ok((trimmed.to_owned(), bytes))
}

async fn prompt_memory(
    current_bytes: u64,
    minimum_bytes: u64,
    preset: Option<&str>,
) -> Result<(String, u64), CliError> {
    if let Some(value) = preset {
        let bytes = parse_memory(value)?;
        check_memory_minimum(bytes, minimum_bytes)?;
        return Ok((value.trim().to_owned(), bytes));
    }
    let message = format!(
        "Memory size (current {}, minimum {})",
        format_mb_gb(current_bytes),
        format_mb_gb(minimum_bytes)
    );
    let answer = Text::new(&message)
        .with_initial_value(&format_memory_flag(current_bytes))
        .with_help_message("xMB or xGB, for example 512MB or 1.5GB; empty keeps the current value")
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(edit_prompt_error)?;
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Ok((format_memory_flag(current_bytes), current_bytes));
    }
    let bytes = parse_memory(trimmed)?;
    check_memory_minimum(bytes, minimum_bytes)?;
    Ok((trimmed.to_owned(), bytes))
}

async fn prompt_vcpus(
    current: u32,
    minimum: u32,
    distribution_id: &str,
    preset: Option<&str>,
) -> Result<u32, CliError> {
    if let Some(value) = preset {
        let vcpus = parse_vcpus(value)?;
        check_vcpu_minimum(vcpus, minimum, distribution_id)?;
        return Ok(vcpus);
    }
    let message = format!("vCPU count (current {current}, minimum {minimum})");
    let answer = Text::new(&message)
        .with_initial_value(&current.to_string())
        .with_help_message("positive integer count; empty keeps the current value")
        .with_render_config(prompt_render_config(TerminalCapabilities::detect().color))
        .prompt()
        .map_err(edit_prompt_error)?;
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Ok(current);
    }
    parse_vcpus(trimmed)
}

fn map_edit_error(name: &str, error: SdkError) -> CliError {
    match &error {
        SdkError::NotFound { .. } => CliError::edit_not_found(name),
        SdkError::InvalidRequest { .. } => CliError::edit_failed(
            "MicroVM edit is invalid",
            &SdkError::Migration(error.to_string()),
        ),
        SdkError::LifecycleConflict { .. } => CliError::edit_running(name),
        _ => CliError::edit_failed("MicroVM edit failed", &error),
    }
}

fn map_usage_error(name: &str, error: SdkError) -> CliError {
    match &error {
        SdkError::NotFound { .. } => CliError::edit_not_found(name),
        SdkError::LifecycleConflict { .. } => CliError::edit_running(name),
        _ => CliError::edit_failed("MicroVM disk usage could not be inspected", &error),
    }
}

async fn execute_edit(
    context: &CliContext,
    name: &str,
    disk_size_bytes: Option<u64>,
    memory_bytes: Option<u64>,
    vcpu_count: Option<u32>,
) -> Result<u8, CliError> {
    let spinner = crate::output::human::EditSpinner::new(name, context.terminal);
    let result = context
        .sdk
        .update_microvm(UpdateMicroVmRequest {
            name: name.to_owned(),
            disk_size_bytes,
            memory_bytes,
            vcpu_count,
        })
        .await;
    spinner.finish();
    match result {
        Ok(updated) => {
            crate::output::human::write_edit_result(&updated, context.terminal)
                .map_err(CliError::from)?;
            Ok(0)
        }
        Err(error) => Err(map_edit_error(name, error)),
    }
}

struct EditTargets {
    summary: MicroVmSummary,
    minimum_memory: u64,
    minimum_vcpus: u32,
    used_disk: u64,
}

async fn load_targets(context: &CliContext, name: &str) -> Result<EditTargets, CliError> {
    let machines = context.sdk.list_microvms().await?;
    let summary = machines
        .into_iter()
        .find(|machine| machine.name == name)
        .ok_or_else(|| CliError::edit_not_found(name))?;
    if summary.state != taumaru_microvm::MicroVmState::Stopped {
        return Err(CliError::edit_running(name));
    }
    let client = SdkArtifactClient::new(&context.sdk);
    let catalog_spinner = crate::output::human::CatalogSpinner::new(context.terminal);
    let catalog_result = load_catalog(&client).await;
    catalog_spinner.finish();
    let catalog = catalog_result?;
    let distribution = catalog
        .distribution(&summary.distribution_id)
        .ok_or_else(|| {
            CliError::creation(
                "Distribution requirements are unavailable",
                format!(
                    "distribution {} is not published for this host",
                    summary.distribution_id
                ),
                "Check registry access and try again",
            )
        })?;
    let minimum_memory = distribution
        .requirements
        .min_memory_mb
        .checked_mul(1024 * 1024)
        .ok_or_else(|| {
            CliError::creation(
                "Distribution requirements are invalid",
                "minimum memory exceeds the supported range",
                "Retry after the registry publishes valid requirements",
            )
        })?;
    let used_disk = context
        .sdk
        .microvm_disk_used_bytes(name)
        .await
        .map_err(|error| map_usage_error(name, error))?;
    Ok(EditTargets {
        summary,
        minimum_memory,
        minimum_vcpus: distribution.requirements.min_vcpus,
        used_disk,
    })
}

async fn edit_resolved(
    context: &CliContext,
    name: &str,
    disk_preset: Option<&str>,
    memory_preset: Option<&str>,
    vcpu_preset: Option<&str>,
) -> Result<u8, CliError> {
    let targets = load_targets(context, name).await?;
    let current = &targets.summary;

    if let Some(value) = disk_preset {
        let bytes = parse_disk_gb(value)?;
        check_disk_minimum_used(bytes, targets.used_disk)?;
    }
    if let Some(value) = memory_preset {
        let bytes = parse_memory(value)?;
        check_memory_minimum(bytes, targets.minimum_memory)?;
    }
    if let Some(value) = vcpu_preset {
        let vcpus = parse_vcpus(value)?;
        check_vcpu_minimum(vcpus, targets.minimum_vcpus, &current.distribution_id)?;
    }

    if !context.terminal.interactive {
        return Err(CliError::creation(
            "An interactive terminal is required",
            "editing a machine prompts for capacities and confirmation",
            "Run microvm edit <NAME> --non-interactive with root access",
        ));
    }
    let (disk_text, disk_bytes) =
        prompt_disk(current.disk_size_bytes, targets.used_disk, disk_preset).await?;
    let (memory_text, memory_bytes) =
        prompt_memory(current.memory_bytes, targets.minimum_memory, memory_preset).await?;
    let vcpu_count = prompt_vcpus(
        current.vcpu_count,
        targets.minimum_vcpus,
        &current.distribution_id,
        vcpu_preset,
    )
    .await?;
    check_vcpu_minimum(vcpu_count, targets.minimum_vcpus, &current.distribution_id)?;

    let disk_changed = disk_bytes != current.disk_size_bytes;
    let memory_changed = memory_bytes != current.memory_bytes;
    let vcpu_changed = vcpu_count != current.vcpu_count;
    if !disk_changed && !memory_changed && !vcpu_changed {
        return Err(CliError::edit_no_changes(name));
    }

    let changes = [
        crate::output::human::EditCapacityChange {
            label: "Disk",
            old_value: format_gb(current.disk_size_bytes),
            new_value: format_gb(disk_bytes),
            changed: disk_changed,
        },
        crate::output::human::EditCapacityChange {
            label: "Memory",
            old_value: format_mb_gb(current.memory_bytes),
            new_value: format_mb_gb(memory_bytes),
            changed: memory_changed,
        },
        crate::output::human::EditCapacityChange {
            label: "vCPUs",
            old_value: current.vcpu_count.to_string(),
            new_value: vcpu_count.to_string(),
            changed: vcpu_changed,
        },
    ];
    crate::output::human::write_edit_review(name, &changes, context.terminal)
        .map_err(CliError::from)?;
    let confirmed = Confirm::new("Apply these changes?")
        .with_default(false)
        .with_help_message("Enter applies the changes  ·  Ctrl-C cancels")
        .with_render_config(prompt_render_config(context.terminal.color))
        .prompt()
        .map_err(edit_prompt_error)?;
    if !confirmed {
        return Err(CliError::edit_cancelled());
    }

    let disk_flag = disk_changed.then(|| disk_text.clone());
    let memory_flag = memory_changed.then(|| memory_text.clone());
    let vcpu_flag = vcpu_changed.then(|| vcpu_count.to_string());
    let home = crate::context::resolve_home().ok();
    let command = escalated_child_command(
        name,
        disk_flag.as_deref(),
        memory_flag.as_deref(),
        vcpu_flag.as_deref(),
    );
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
    execute_edit(
        context,
        name,
        disk_changed.then_some(disk_bytes),
        memory_changed.then_some(memory_bytes),
        vcpu_changed.then_some(vcpu_count),
    )
    .await
}

pub(crate) async fn run(context: &CliContext, arguments: EditArgs) -> Result<u8, CliError> {
    if arguments.non_interactive {
        let name = resolve_name(
            arguments.name.as_deref(),
            arguments.explicit_name.as_deref(),
        )
        .map_err(|error| match error {
            CliError::MissingValue(_) => CliError::missing_value(
                "machine name",
                "--name <NAME>",
                "Run microvm edit web-01 --non-interactive --vcpus 4",
            ),
            other => other,
        })?;
        if arguments.disk_gb.is_none() && arguments.memory.is_none() && arguments.vcpus.is_none() {
            return Err(CliError::missing_value(
                "edit change",
                "--disk-gb <GB> | --memory <xMB|xGB> | --vcpus <N>",
                "Run microvm edit web-01 --non-interactive --vcpus 4",
            ));
        }
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
        let targets = load_targets(context, &name).await?;
        let current = &targets.summary;
        let disk_bytes = match arguments.disk_gb.as_deref() {
            Some(value) => {
                let bytes = parse_disk_gb(value)?;
                check_disk_minimum_used(bytes, targets.used_disk)?;
                Some(bytes)
            }
            None => None,
        };
        let memory_bytes = match arguments.memory.as_deref() {
            Some(value) => {
                let bytes = parse_memory(value)?;
                check_memory_minimum(bytes, targets.minimum_memory)?;
                Some(bytes)
            }
            None => None,
        };
        let vcpu_count = match arguments.vcpus.as_deref() {
            Some(value) => {
                let vcpus = parse_vcpus(value)?;
                check_vcpu_minimum(vcpus, targets.minimum_vcpus, &current.distribution_id)?;
                Some(vcpus)
            }
            None => None,
        };
        if disk_bytes.is_none_or(|value| value == current.disk_size_bytes)
            && memory_bytes.is_none_or(|value| value == current.memory_bytes)
            && vcpu_count.is_none_or(|value| value == current.vcpu_count)
        {
            return Err(CliError::edit_no_changes(&name));
        }
        return execute_edit(context, &name, disk_bytes, memory_bytes, vcpu_count).await;
    }

    if arguments.name.is_none() && arguments.explicit_name.is_none() {
        let home = crate::context::resolve_home().ok();
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            false,
            home,
            &[],
            bare_escalated_command(
                arguments.disk_gb.as_deref(),
                arguments.memory.as_deref(),
                arguments.vcpus.as_deref(),
            ),
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
                "Run microvm edit <NAME> --non-interactive with root access",
            ));
        }
        let machines: Vec<MicroVmSummary> = context.sdk.list_microvms().await?;
        if machines.is_empty() {
            return Err(CliError::edit_empty());
        }
        let name = prompt_machine(&machines, context.terminal).await?;
        return edit_resolved(
            context,
            &name,
            arguments.disk_gb.as_deref(),
            arguments.memory.as_deref(),
            arguments.vcpus.as_deref(),
        )
        .await;
    }

    let name = resolve_name(
        arguments.name.as_deref(),
        arguments.explicit_name.as_deref(),
    )?;
    edit_resolved(
        context,
        &name,
        arguments.disk_gb.as_deref(),
        arguments.memory.as_deref(),
        arguments.vcpus.as_deref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        bare_escalated_command, check_disk_minimum_used, escalated_child_command, map_edit_error,
        resolve_name,
    };

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
    fn escalated_child_carries_only_changed_capacities() {
        let rendered: Vec<String> = escalated_child_command("web-01", Some("30"), None, Some("4"))
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            [
                "edit",
                "web-01",
                "--non-interactive",
                "--disk-gb",
                "30",
                "--vcpus",
                "4"
            ]
        );
    }

    #[test]
    fn bare_escalated_child_forwards_capacity_presets() {
        let rendered: Vec<String> = bare_escalated_command(Some("30"), None, Some("4"))
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["edit", "--disk-gb", "30", "--vcpus", "4"]);
        let rendered: Vec<String> = bare_escalated_command(None, None, None)
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["edit"]);
    }

    #[test]
    fn disk_minimum_is_shown_as_used() {
        let error =
            check_disk_minimum_used(1, 20 * 1024 * 1024 * 1024).expect_err("too small aborts");
        let message = error.user_message(false);
        assert!(message.contains("20 GB"));
        assert!(message.contains("used"));
    }

    #[test]
    fn unknown_machine_maps_to_creation_pointer() {
        use taumaru_microvm::SdkError;
        let error = map_edit_error(
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
        let error = map_edit_error(
            "web-01",
            SdkError::LifecycleConflict {
                name: "web-01".to_owned(),
                state: "running".to_owned(),
                operation: "edit MicroVM (stop the machine first)".to_owned(),
            },
        );
        let message = error.user_message(false);
        assert!(message.contains("web-01"));
        assert!(message.contains("microvm stop web-01"));
    }

    #[test]
    fn empty_stored_set_points_at_creation() {
        let error = crate::error::CliError::edit_empty();
        let message = error.user_message(false);
        assert!(message.contains("No MicroVMs to edit"));
        assert!(message.contains("microvm new"));
    }

    #[test]
    fn edit_cancellation_reports_exit_130() {
        let error = crate::error::CliError::edit_cancelled();
        assert_eq!(error.exit_code(), 130);
        assert!(error.user_message(false).contains("No machine was changed"));
    }
}
