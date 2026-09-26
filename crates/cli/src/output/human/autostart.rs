use std::io::{self, Write};

use taumaru_microvm::{
    AutostartDeleteResult, AutostartPolicy, AutostartResult, AutostartRunReport, MicroVmState,
};

use super::{ANSI_BOLD, ANSI_DIM, ANSI_GREEN, ANSI_RED, ANSI_YELLOW, divider, paint};
use crate::boot::BootState;
use crate::context::TerminalCapabilities;

pub(crate) struct AutostartRow {
    pub(crate) policy: AutostartPolicy,
    pub(crate) state: Option<MicroVmState>,
}

pub(crate) fn status_text(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "paused" }
}

fn boot_line(boot: &BootState) -> String {
    match boot {
        BootState::Enabled { unit } => format!("{unit} enabled"),
        BootState::Disabled { unit } => {
            format!("{unit} disabled (no enabled autostart machines)")
        }
    }
}

pub(crate) fn format_policy_result(
    heading: &str,
    policy: &AutostartPolicy,
    boot: &BootState,
    capabilities: TerminalCapabilities,
) -> String {
    let check = paint("✓", ANSI_GREEN, capabilities.color);
    let title = paint(
        format!("{heading} for {}", policy.name),
        ANSI_BOLD,
        capabilities.color,
    );
    let rule = divider(capabilities);
    let status_label = paint("Status:", ANSI_DIM, capabilities.color);
    let attempts_label = paint("Attempts:", ANSI_DIM, capabilities.color);
    let boot_label = paint("Boot:", ANSI_DIM, capabilities.color);
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!(
        "  {status_label}   {}\n",
        status_text(policy.enabled)
    ));
    output.push_str(&format!(
        "  {attempts_label} {} per boot\n",
        policy.max_start_attempts
    ));
    output.push_str(&format!("  {boot_label}     {}\n", boot_line(boot)));
    output
}

pub(crate) fn format_removed(
    result: &AutostartDeleteResult,
    boot: &BootState,
    capabilities: TerminalCapabilities,
) -> String {
    let check = paint("✓", ANSI_GREEN, capabilities.color);
    let heading = if result.removed {
        format!("Autostart removed for {}", result.name)
    } else {
        format!("Autostart was not configured for {}", result.name)
    };
    let title = paint(heading, ANSI_BOLD, capabilities.color);
    let rule = divider(capabilities);
    let machine_label = paint("Machine:", ANSI_DIM, capabilities.color);
    let boot_label = paint("Boot:", ANSI_DIM, capabilities.color);
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!(
        "  {machine_label} unchanged; it no longer starts at boot\n"
    ));
    output.push_str(&format!("  {boot_label}    {}\n", boot_line(boot)));
    output
}

pub(crate) fn format_table(rows: &[AutostartRow], capabilities: TerminalCapabilities) -> String {
    let headers = ["NAME", "AUTOSTART", "ATTEMPTS", "STATE"];
    let cells: Vec<[String; 4]> = rows
        .iter()
        .map(|row| {
            [
                row.policy.name.clone(),
                status_text(row.policy.enabled).to_owned(),
                row.policy.max_start_attempts.to_string(),
                row.state
                    .map_or_else(|| "-".to_owned(), |state| state.to_string()),
            ]
        })
        .collect();
    let mut widths = headers.map(str::len);
    for row in &cells {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }
    let pad = |index: usize, text: &str| format!("{text:<width$}", width = widths[index]);
    let mut output = String::from("\n");
    let header: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| pad(index, header))
        .collect();
    output.push_str(&format!(
        "{}\n",
        paint(header.join("  "), ANSI_DIM, capabilities.color)
    ));
    for (row, cells) in rows.iter().zip(&cells) {
        let mut padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| pad(index, cell))
            .collect();
        let status_color = if row.policy.enabled {
            ANSI_GREEN
        } else {
            ANSI_YELLOW
        };
        padded[1] = paint(&padded[1], status_color, capabilities.color);
        output.push_str(&format!("{}\n", padded.join("  ").trim_end()));
    }
    output
}

pub(crate) fn format_empty(capabilities: TerminalCapabilities) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n",
        paint("○", ANSI_DIM, capabilities.color),
        paint("No MicroVMs start at boot", ANSI_BOLD, capabilities.color)
    ));
    output.push_str(&format!(
        "  {} Run `microvm autostart add` to start a machine automatically.\n",
        paint("Next:", ANSI_BOLD, capabilities.color)
    ));
    output
}

pub(crate) fn format_run_report(
    report: &AutostartRunReport,
    capabilities: TerminalCapabilities,
) -> String {
    if report.outcomes.is_empty() {
        return "No MicroVMs are configured to start at boot\n".to_owned();
    }
    let mut output = String::new();
    for outcome in &report.outcomes {
        let line = match &outcome.result {
            AutostartResult::Started(_) => format!(
                "{} {} started (attempt {})",
                paint("✓", ANSI_GREEN, capabilities.color),
                outcome.name,
                outcome.attempts
            ),
            AutostartResult::Paused => format!(
                "{} {} paused, not started",
                paint("○", ANSI_DIM, capabilities.color),
                outcome.name
            ),
            AutostartResult::Failed(error) => format!(
                "{} {} failed after {} attempts: {error}",
                paint("✗", ANSI_RED, capabilities.color),
                outcome.name,
                outcome.attempts
            ),
        };
        output.push_str(&line);
        output.push('\n');
    }
    output
}

pub(crate) fn write(text: &str) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{text}")
}

#[cfg(test)]
mod tests {
    use taumaru_microvm::{
        AutostartDeleteResult, AutostartOutcome, AutostartPolicy, AutostartResult,
        AutostartRunReport, MicroVmState, SdkError,
    };

    use super::{
        AutostartRow, format_empty, format_policy_result, format_removed, format_run_report,
        format_table,
    };
    use crate::boot::BootState;
    use crate::context::TerminalCapabilities;

    fn plain() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(60),
        }
    }

    fn policy(name: &str, enabled: bool) -> AutostartPolicy {
        AutostartPolicy {
            name: name.to_owned(),
            enabled,
            max_start_attempts: 3,
        }
    }

    #[test]
    fn table_labels_status_in_text_not_only_color() {
        let rows = [
            AutostartRow {
                policy: policy("db-01", false),
                state: Some(MicroVmState::Stopped),
            },
            AutostartRow {
                policy: policy("web-01", true),
                state: Some(MicroVmState::Running),
            },
        ];
        let table = format_table(&rows, plain());
        assert!(table.contains("NAME    AUTOSTART  ATTEMPTS  STATE"));
        assert!(table.contains("db-01   paused     3         stopped"));
        assert!(table.contains("web-01  enabled    3         running"));
        assert!(!table.contains("\u{1b}["));
    }

    #[test]
    fn policy_result_names_the_boot_unit() {
        let boot = BootState::Enabled {
            unit: "taumaru-microvm-autostart@x.service".to_owned(),
        };
        let output =
            format_policy_result("Autostart enabled", &policy("web-01", true), &boot, plain());
        assert!(output.contains("Autostart enabled for web-01"));
        assert!(output.contains("enabled"));
        assert!(output.contains("3 per boot"));
        assert!(output.contains("taumaru-microvm-autostart@x.service enabled"));
    }

    #[test]
    fn removal_reports_idempotent_results_calmly() {
        let boot = BootState::Disabled {
            unit: "taumaru-microvm-autostart@x.service".to_owned(),
        };
        let output = format_removed(
            &AutostartDeleteResult {
                name: "web-01".to_owned(),
                removed: false,
            },
            &boot,
            plain(),
        );
        assert!(output.contains("Autostart was not configured for web-01"));
        assert!(output.contains("disabled"));
    }

    #[test]
    fn run_report_lists_each_machine_with_a_text_label() {
        let report = AutostartRunReport {
            outcomes: vec![
                AutostartOutcome {
                    name: "db-01".to_owned(),
                    attempts: 0,
                    result: AutostartResult::Paused,
                },
                AutostartOutcome {
                    name: "web-01".to_owned(),
                    attempts: 3,
                    result: AutostartResult::Failed(SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "web-01".to_owned(),
                    }),
                },
            ],
        };
        let output = format_run_report(&report, plain());
        assert!(output.contains("db-01 paused, not started"));
        assert!(output.contains("web-01 failed after 3 attempts"));
    }

    #[test]
    fn empty_listing_points_to_the_add_command() {
        assert!(format_empty(plain()).contains("microvm autostart add"));
    }
}
