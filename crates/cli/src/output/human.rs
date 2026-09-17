use std::io::{self, Write};
use std::time::Duration;

use crate::commands::download::{
    Availability, DownloadOutcome, DownloadPlan, MemberOutcome, PlanMember,
};
use crate::context::TerminalCapabilities;
use crate::output::ProgressSink;
use indicatif::{ProgressBar, ProgressStyle};
use taumaru_microvm::DownloadPhase;

const ANSI_BOLD: &str = "\u{1b}[1m";
const ANSI_DIM: &str = "\u{1b}[2m";
const ANSI_BLUE: &str = "\u{1b}[34m";
const ANSI_GREEN: &str = "\u{1b}[32m";
const ANSI_YELLOW: &str = "\u{1b}[33m";
const ANSI_RED: &str = "\u{1b}[31m";
const ANSI_RESET: &str = "\u{1b}[0m";

pub(crate) struct CatalogSpinner {
    bar: ProgressBar,
    interactive: bool,
}

impl CatalogSpinner {
    pub(crate) fn new(capabilities: TerminalCapabilities) -> Self {
        let bar = if capabilities.interactive {
            let bar = ProgressBar::new_spinner();
            let template = if capabilities.color {
                "{spinner:.dim} {msg}"
            } else {
                "{spinner} {msg}"
            };
            let style = match ProgressStyle::with_template(template) {
                Ok(style) => {
                    style.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                }
                Err(_) => ProgressStyle::default_spinner(),
            };
            bar.set_style(style);
            bar.set_message("Checking artifact registry");
            bar.enable_steady_tick(Duration::from_millis(90));
            bar
        } else {
            eprintln!("·  Checking artifact registry");
            ProgressBar::hidden()
        };

        Self {
            bar,
            interactive: capabilities.interactive,
        }
    }

    pub(crate) fn finish(self) {
        if self.interactive {
            self.bar.finish_and_clear();
        }
    }
}

pub(crate) fn write_catalog_ready(
    capabilities: TerminalCapabilities,
    distribution_count: usize,
    kernel_count: usize,
    binary_count: usize,
) -> Result<(), io::Error> {
    let mut stderr = io::stderr().lock();
    writeln!(
        stderr,
        "{}  {}  {} distributions · {} kernels · {} runtime packages",
        paint("✓", ANSI_GREEN, capabilities.color),
        paint("Registry ready", ANSI_BOLD, capabilities.color),
        distribution_count,
        kernel_count,
        binary_count,
    )?;
    Ok(())
}

pub(crate) struct ProgressRenderer {
    bar: ProgressBar,
    capabilities: TerminalCapabilities,
}

impl ProgressRenderer {
    pub(crate) fn new(total_bytes: u64, capabilities: TerminalCapabilities) -> Self {
        let bar = if capabilities.interactive {
            ProgressBar::new(total_bytes)
        } else {
            ProgressBar::hidden()
        };
        let template = if capabilities.color {
            "{spinner:.dim} {bar:28} {bytes}/{total_bytes} {msg}"
        } else {
            "{spinner} {bar:28} {bytes}/{total_bytes} {msg}"
        };
        let style = match ProgressStyle::with_template(template) {
            Ok(style) => style
                .progress_chars("━╸─")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            Err(_) => ProgressStyle::default_bar().progress_chars("=>-"),
        };
        bar.set_style(style);
        Self { bar, capabilities }
    }

    pub(crate) fn finish(self) {
        if self.capabilities.interactive {
            self.bar.finish_and_clear();
        }
    }
}

impl ProgressSink for ProgressRenderer {
    fn on_progress(&mut self, progress: crate::commands::download::DownloadProgressView) {
        let position = progress
            .aggregate_current_bytes
            .min(progress.plan_total_bytes);
        self.bar.set_position(position);
        self.bar.set_message(format!(
            "{} · {}",
            artifact_label(&progress),
            phase_label(&progress.phase)
        ));
        if !self.capabilities.interactive {
            eprintln!("{}", format_progress_line(&progress, self.capabilities));
        }
    }
}

fn artifact_label(progress: &crate::commands::download::DownloadProgressView) -> String {
    let kind = match &progress.artifact_kind {
        taumaru_microvm::ArtifactKind::Kernel => "kernel",
        taumaru_microvm::ArtifactKind::Binary => "runtime",
        taumaru_microvm::ArtifactKind::DistributionImage => "distribution",
    };
    match &progress.member_name {
        Some(member) => format!("{kind}/{}/{}", progress.artifact_id, member),
        None => format!("{kind}/{}", progress.artifact_id),
    }
}

pub(crate) fn format_progress_line(
    progress: &crate::commands::download::DownloadProgressView,
    capabilities: TerminalCapabilities,
) -> String {
    let current = format_bytes(progress.current_bytes);
    let expected = format_bytes(progress.expected_bytes);
    let aggregate_current = format_bytes(progress.aggregate_current_bytes);
    let plan_total = format_bytes(progress.plan_total_bytes);
    let identity = artifact_label(progress);
    let phase = phase_label(&progress.phase);
    let marker = phase_marker(&progress.phase, capabilities.color);
    if capabilities.width.is_some_and(|width| width < 72) {
        format!("{marker}  {identity}  {phase}  {current} / {expected}")
    } else {
        format!(
            "{marker}  {identity}  {phase}  {current} / {expected}  ·  plan {aggregate_current} / {plan_total}"
        )
    }
}

fn phase_label(phase: &DownloadPhase) -> &'static str {
    match phase {
        DownloadPhase::Downloading => "Downloading",
        DownloadPhase::Verifying => "Verifying",
        DownloadPhase::Completed => "Downloaded",
        DownloadPhase::AdoptedExisting => "Adopted",
        DownloadPhase::SkippedExisting => "Already available",
        DownloadPhase::Cancelled => "Cancelled",
    }
}

fn phase_marker(phase: &DownloadPhase, color: bool) -> String {
    let (symbol, ansi) = match phase {
        DownloadPhase::Downloading | DownloadPhase::Verifying => ("↓", ANSI_BLUE),
        DownloadPhase::Completed
        | DownloadPhase::AdoptedExisting
        | DownloadPhase::SkippedExisting => ("✓", ANSI_GREEN),
        DownloadPhase::Cancelled => ("!", ANSI_YELLOW),
    };
    paint(symbol, ansi, color)
}

pub(crate) fn format_review(plan: &DownloadPlan, capabilities: TerminalCapabilities) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n{}\n\n",
        paint("◆", ANSI_BLUE, capabilities.color),
        paint("Download plan", ANSI_BOLD, capabilities.color),
        divider(capabilities),
    ));

    output.push_str(&format!(
        "{}\n",
        paint("Runtime", ANSI_BOLD, capabilities.color)
    ));
    if capabilities.width.is_some_and(|width| width < 72) {
        output.push_str(&format!(
            "  {}  {} {} · {} files · {}\n",
            paint("•", ANSI_BLUE, capabilities.color),
            plan.runtime.package.display_name,
            plan.runtime.package.version,
            plan.runtime.files.len(),
            architecture_label(&plan.runtime.package.architecture),
        ));
    } else {
        output.push_str(&format!(
            "  {}  {} {} ({}) · {} files · {}\n",
            paint("•", ANSI_BLUE, capabilities.color),
            plan.runtime.package.display_name,
            plan.runtime.package.version,
            paint(
                plan.runtime.package.id.as_str(),
                ANSI_DIM,
                capabilities.color
            ),
            plan.runtime.files.len(),
            architecture_label(&plan.runtime.package.architecture),
        ));
    }
    output.push_str(&format!(
        "     {}\n\n",
        paint(
            format!("{} expected", format_bytes(plan.runtime.expected_bytes)),
            ANSI_DIM,
            capabilities.color,
        )
    ));

    output.push_str(&format!(
        "{}\n",
        paint("Targets", ANSI_BOLD, capabilities.color)
    ));
    for selection in &plan.selections {
        let default_marker = if selection.kernel_is_default {
            " · default"
        } else {
            ""
        };
        if capabilities.width.is_some_and(|width| width < 72) {
            output.push_str(&format!(
                "  {}  {} → {}{}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                selection.distribution.id,
                selection.kernel.id,
                default_marker,
            ));
            output.push_str(&format!(
                "     {} images · {}\n",
                selection.distribution.images.len(),
                format_bytes(selection_image_bytes(selection)),
            ));
        } else {
            output.push_str(&format!(
                "  {}  {} ({})\n     {}  {} ({}){} · {} images · {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                selection.distribution.id,
                selection.distribution.display_name,
                paint("↳", ANSI_DIM, capabilities.color),
                selection.kernel.id,
                selection.kernel.display_name,
                default_marker,
                selection.distribution.images.len(),
                format_bytes(selection_image_bytes(selection)),
            ));
        }
    }

    let image_count = plan
        .members
        .iter()
        .filter_map(|member| match member {
            PlanMember::DistributionImages { image_count, .. } => Some(*image_count),
            _ => None,
        })
        .fold(0_usize, usize::saturating_add);
    output.push_str(&format!(
        "\n{}\n  {} · {} planned groups · {} images\n\n",
        paint("Transfer", ANSI_BOLD, capabilities.color),
        format_bytes(plan.expected_bytes),
        plan.members.len(),
        image_count,
    ));
    output.push_str(&format!(
        "{}\n",
        paint(
            "Review the plan above. The download starts after confirmation.",
            ANSI_DIM,
            capabilities.color,
        )
    ));
    output
}

fn selection_image_bytes(selection: &crate::commands::download::DistributionSelection) -> u64 {
    selection
        .distribution
        .images
        .iter()
        .fold(0_u64, |total, image| total.saturating_add(image.size_bytes))
}

fn divider(capabilities: TerminalCapabilities) -> String {
    let width = capabilities
        .width
        .map(|width| width.saturating_sub(4).clamp(16, 56))
        .unwrap_or(48);
    paint("─".repeat(width), ANSI_DIM, capabilities.color)
}

pub(crate) fn write_review(
    plan: &DownloadPlan,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_review(plan, capabilities))
}

pub(crate) fn write_summary(
    outcome: &DownloadOutcome,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    let summary = if capabilities.color {
        format_summary_with_capabilities(outcome, capabilities)
    } else {
        format_summary(outcome)
    };
    write!(stdout, "{summary}")
}

pub(crate) fn format_summary(outcome: &DownloadOutcome) -> String {
    format_summary_with_capabilities(
        outcome,
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: None,
        },
    )
}

fn format_summary_with_capabilities(
    outcome: &DownloadOutcome,
    capabilities: TerminalCapabilities,
) -> String {
    let title = if outcome.cancelled {
        "Download cancelled"
    } else if outcome.is_success() {
        "Download complete"
    } else {
        "Download incomplete"
    };
    let title_marker = if outcome.cancelled {
        paint("!", ANSI_YELLOW, capabilities.color)
    } else if outcome.is_success() {
        paint("✓", ANSI_GREEN, capabilities.color)
    } else {
        paint("×", ANSI_RED, capabilities.color)
    };
    let (verified_files, verified_bytes) = outcome
        .verified
        .iter()
        .map(|artifact| match artifact {
            crate::commands::download::VerifiedArtifact::Binary(result) => (
                result.files.len(),
                result
                    .files
                    .iter()
                    .fold(0_u64, |total, file| total.saturating_add(file.size_bytes)),
            ),
            crate::commands::download::VerifiedArtifact::Kernel(result) => {
                (1, result.file.size_bytes)
            }
            crate::commands::download::VerifiedArtifact::Distribution(result) => (
                result.images.len(),
                result
                    .images
                    .iter()
                    .fold(0_u64, |total, file| total.saturating_add(file.size_bytes)),
            ),
        })
        .fold(
            (0_usize, 0_u64),
            |(files, bytes), (new_files, new_bytes)| {
                (
                    files.saturating_add(new_files),
                    bytes.saturating_add(new_bytes),
                )
            },
        );
    let ready_groups = outcome
        .groups
        .iter()
        .filter(|group| matches!(group, MemberOutcome::Verified { .. }))
        .count();
    let total_groups = outcome.groups.len();
    let mut output = format!(
        "\n{} {}\n{}\n\n  {ready_groups}/{total_groups} groups ready · {verified_files} files verified\n  {} available of {} planned · {} verified\n\n{}\n",
        title_marker,
        paint(title, ANSI_BOLD, capabilities.color),
        divider(capabilities),
        format_bytes(outcome.available_bytes),
        format_bytes(outcome.expected_bytes),
        format_bytes(verified_bytes),
        paint("Artifacts", ANSI_BOLD, capabilities.color),
    );
    for group in &outcome.groups {
        match group {
            MemberOutcome::Verified {
                label,
                availability,
            } => output.push_str(&format!(
                "  {}  {}  {}\n",
                paint("✓", ANSI_GREEN, capabilities.color),
                label,
                paint(
                    availability_label(availability),
                    ANSI_DIM,
                    capabilities.color
                ),
            )),
            MemberOutcome::Failed { label, reason } => {
                output.push_str(&format!(
                    "  {}  {}\n      {} · {}\n",
                    paint("×", ANSI_RED, capabilities.color),
                    label,
                    paint("Failed", ANSI_RED, capabilities.color),
                    reason,
                ));
            }
            MemberOutcome::Skipped { label, reason } => {
                output.push_str(&format!(
                    "  {}  {}\n      {} · {}\n",
                    paint("↷", ANSI_YELLOW, capabilities.color),
                    label,
                    paint("Skipped", ANSI_YELLOW, capabilities.color),
                    reason,
                ));
            }
            MemberOutcome::Cancelled { label } => {
                output.push_str(&format!(
                    "  {}  {}  {}\n",
                    paint("!", ANSI_YELLOW, capabilities.color),
                    label,
                    paint("Cancelled", ANSI_YELLOW, capabilities.color),
                ));
            }
        }
    }
    if outcome.cancelled {
        output.push_str(
            "\n  Why: transfer stopped before the remaining groups were verified; completed groups were preserved.\n  Next: retry `microvm download` to acquire the cancelled groups.\n",
        );
    } else if !outcome.is_success() {
        output.push_str(
            "\n  Why: one or more required groups failed or depended on a failed kernel.\n  Next: resolve the failed group and retry; verified groups will be reused.\n",
        );
    }
    output
}

fn availability_label(availability: &Availability) -> &'static str {
    match availability {
        Availability::Downloaded => "Downloaded",
        Availability::Adopted => "Adopted",
        Availability::AlreadyAvailable => "Already available",
        Availability::Mixed => "Verified",
    }
}

fn paint(text: impl AsRef<str>, ansi: &str, color: bool) -> String {
    let text = text.as_ref();
    if color {
        format!("{ansi}{text}{ANSI_RESET}")
    } else {
        text.to_owned()
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn architecture_label(architecture: &taumaru_microvm::Architecture) -> &'static str {
    match architecture {
        taumaru_microvm::Architecture::X86_64 => "x86_64",
        taumaru_microvm::Architecture::Aarch64 => "aarch64",
        taumaru_microvm::Architecture::Arm => "arm",
        taumaru_microvm::Architecture::Riscv64 => "riscv64",
        taumaru_microvm::Architecture::X86 => "x86",
    }
}

#[cfg(test)]
mod tests {
    use super::{format_progress_line, format_summary};
    use crate::commands::download::{Availability, DownloadOutcome, MemberOutcome};
    use crate::context::TerminalCapabilities;
    use taumaru_microvm::{ArtifactKind, DownloadPhase};

    fn view() -> crate::commands::download::DownloadProgressView {
        crate::commands::download::DownloadProgressView {
            artifact_kind: ArtifactKind::Kernel,
            artifact_id: "linux-6.8".to_owned(),
            member_name: None,
            phase: DownloadPhase::Downloading,
            current_bytes: 4,
            expected_bytes: 8,
            completed_plan_bytes: 16,
            aggregate_current_bytes: 20,
            aggregate_expected_bytes: 24,
            plan_total_bytes: 64,
        }
    }

    #[test]
    fn progress_line_keeps_identity_stage_and_byte_counters_visible() {
        let line = format_progress_line(
            &view(),
            TerminalCapabilities {
                interactive: true,
                color: true,
                width: Some(120),
            },
        );
        assert!(line.contains("kernel/linux-6.8"));
        assert!(line.contains("Downloading"));
        assert!(line.contains("4 B / 8 B"));
        assert!(line.contains("20 B / 64 B"));
    }

    #[test]
    fn narrow_and_non_tty_lines_remain_textually_distinguishable() {
        let narrow = format_progress_line(
            &view(),
            TerminalCapabilities {
                interactive: false,
                color: false,
                width: Some(50),
            },
        );
        assert!(narrow.contains("Downloading"));
        assert!(narrow.contains("linux-6.8"));
        assert!(!narrow.contains("\u{1b}["));
    }

    #[test]
    fn summary_uses_a_clear_status_hierarchy() {
        let outcome = DownloadOutcome {
            verified: Vec::new(),
            groups: vec![MemberOutcome::Verified {
                label: "kernel/linux-6.8".to_owned(),
                availability: Availability::AlreadyAvailable,
            }],
            cancelled: false,
            expected_bytes: 8,
            available_bytes: 8,
        };

        let summary = format_summary(&outcome);

        assert!(summary.starts_with("\n✓ Download complete"));
        assert!(summary.contains("Artifacts"));
        assert!(summary.contains("Already available"));
        assert!(!summary.contains("\u{1b}["));
    }
}
