use std::io::{self, Write};
use std::time::Duration;

use crate::commands::download::DownloadProgressView;
use crate::commands::download::{Availability, DownloadOutcome, DownloadPlan, MemberOutcome};
use crate::commands::new::{
    NewImageChoice, NewProvisioningPlan, NewVmRequest, format_gb, format_mb_gb,
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

pub(crate) struct StartSpinner {
    bar: ProgressBar,
    interactive: bool,
}

impl StartSpinner {
    pub(crate) fn new(name: &str, capabilities: TerminalCapabilities) -> Self {
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
            bar.set_message(format!("Starting MicroVM {name}"));
            bar.enable_steady_tick(Duration::from_millis(90));
            bar
        } else {
            eprintln!("·  Starting MicroVM {name}");
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

pub(crate) struct StopSpinner {
    bar: ProgressBar,
    interactive: bool,
}

impl StopSpinner {
    pub(crate) fn new(name: &str, capabilities: TerminalCapabilities) -> Self {
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
            bar.set_message(format!("Stopping MicroVM {name}"));
            bar.enable_steady_tick(Duration::from_millis(90));
            bar
        } else {
            eprintln!("·  Stopping MicroVM {name}");
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

pub(crate) struct DeleteSpinner {
    bar: ProgressBar,
    interactive: bool,
}

impl DeleteSpinner {
    pub(crate) fn new(name: &str, capabilities: TerminalCapabilities) -> Self {
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
            bar.set_message(format!("Deleting MicroVM {name}"));
            bar.enable_steady_tick(Duration::from_millis(90));
            bar
        } else {
            eprintln!("·  Deleting MicroVM {name}");
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
    for package in &plan.runtime.packages {
        if capabilities.width.is_some_and(|width| width < 72) {
            output.push_str(&format!(
                "  {}  {} {} · {} files · {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                package.display_name,
                package.version,
                package.files.len(),
                architecture_label(&package.architecture),
            ));
        } else {
            output.push_str(&format!(
                "  {}  {} {} ({}) · {} files · {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                package.display_name,
                package.version,
                paint(package.id.as_str(), ANSI_DIM, capabilities.color),
                package.files.len(),
                architecture_label(&package.architecture),
            ));
        }
    }
    output.push_str(&format!(
        "     {}\n\n",
        paint(
            format!(
                "{} files · {} expected",
                plan.runtime.file_count,
                format_bytes(plan.runtime.expected_bytes)
            ),
            ANSI_DIM,
            capabilities.color,
        )
    ));

    output.push_str(&format!(
        "{}\n",
        paint("Targets", ANSI_BOLD, capabilities.color)
    ));
    for selection in &plan.selections {
        if capabilities.width.is_some_and(|width| width < 72) {
            output.push_str(&format!(
                "  {}  {} / {} · default\n",
                paint("•", ANSI_BLUE, capabilities.color),
                selection.distribution.id,
                selection.image.id,
            ));
            output.push_str(&format!(
                "     {}\n",
                format_bytes(selection.expected_bytes),
            ));
        } else {
            output.push_str(&format!(
                "  {}  {} / {} ({})\n     {}  {} ({}) · default · {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                selection.distribution.id,
                selection.image.id,
                selection.image.display_name,
                paint("↳", ANSI_DIM, capabilities.color),
                selection.kernel.id,
                selection.kernel.display_name,
                format_bytes(selection.expected_bytes),
            ));
        }
    }

    let image_count = plan.selections.len();
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
            crate::commands::download::VerifiedArtifact::DistributionImage(result) => {
                (1, result.file.size_bytes)
            }
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
            "\n  Why: transfer stopped before the remaining groups were verified; completed groups were preserved.\n  Next: retry `microvm artifacts download` to acquire the cancelled groups.\n",
        );
    } else if !outcome.is_success() {
        output.push_str(
            "\n  Why: one or more required groups failed or depended on a failed kernel.\n  Next: resolve the failed group and retry; verified groups will be reused.\n",
        );
    }
    output
}

pub(crate) fn format_new_review(
    request: &NewVmRequest,
    choice: &NewImageChoice,
    plan: &NewProvisioningPlan,
    disk_text: &str,
    memory_text: &str,
    capabilities: TerminalCapabilities,
) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n{}\n\n",
        paint("\u{25c6}", ANSI_BLUE, capabilities.color),
        paint("New MicroVM", ANSI_BOLD, capabilities.color),
        divider(capabilities),
    ));
    let network = if request.expose_on_lan {
        "LAN exposed"
    } else {
        "host-only"
    };
    for (label, value) in [
        ("Name", request.name.clone()),
        (
            "Image",
            format!(
                "{} / {} ({})",
                choice.distribution.id, choice.image.id, choice.image.display_name
            ),
        ),
        (
            "Disk",
            format!("{disk_text} GB ({} bytes)", request.disk_size_bytes),
        ),
        (
            "Memory",
            format!("{memory_text} ({} bytes)", request.memory_bytes),
        ),
        ("vCPUs", request.vcpu_count.to_string()),
        ("Network", network.to_owned()),
    ] {
        output.push_str(&format!(
            "  {}  {}: {}\n",
            paint("\u{2022}", ANSI_BLUE, capabilities.color),
            paint(label, ANSI_BOLD, capabilities.color),
            value,
        ));
    }
    let missing: Vec<String> = plan
        .runtime_packages
        .iter()
        .map(|package| format!("runtime/{}", package.id))
        .chain(std::iter::once(format!("kernel/{}", plan.kernel.id)))
        .chain(std::iter::once(format!(
            "image/{}/{}",
            choice.distribution.id, choice.image.id
        )))
        .collect();
    output.push_str(&format!(
        "\n  {}\n\n{}\n",
        paint(
            format!(
                "Needs provisioning: {} · {} expected",
                missing.join(", "),
                format_bytes(plan.provision_bytes)
            ),
            ANSI_DIM,
            capabilities.color,
        ),
        paint(
            "Review the values above. Creation starts after confirmation.",
            ANSI_DIM,
            capabilities.color,
        ),
    ));
    output
}

pub(crate) fn write_new_review(
    request: &NewVmRequest,
    choice: &NewImageChoice,
    plan: &NewProvisioningPlan,
    disk_text: &str,
    memory_text: &str,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(
        stdout,
        "{}",
        format_new_review(request, choice, plan, disk_text, memory_text, capabilities)
    )
}

pub(crate) struct NewProgressRenderer {
    bar: ProgressBar,
    capabilities: TerminalCapabilities,
}

impl NewProgressRenderer {
    pub(crate) fn new(total: u64, capabilities: TerminalCapabilities) -> Self {
        let bar = if capabilities.interactive {
            ProgressBar::new(total)
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
                .progress_chars("\u{2501}\u{2578}\u{2500}")
                .tick_strings(&[
                    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}",
                    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
                ]),
            Err(_) => ProgressStyle::default_bar().progress_chars("=>-"),
        };
        bar.set_style(style);
        Self { bar, capabilities }
    }

    pub(crate) fn begin_member(&self, label: &str, member_bytes: u64) {
        self.bar.reset();
        self.bar.set_length(member_bytes);
        self.bar.set_position(0);
        self.bar.set_message(label.to_owned());
    }

    pub(crate) fn on_progress(&mut self, progress: DownloadProgressView) {
        self.bar.set_position(
            progress
                .aggregate_current_bytes
                .min(progress.aggregate_expected_bytes),
        );
        self.bar.set_message(format!(
            "{} · {}",
            new_artifact_label(&progress),
            phase_label(&progress.phase)
        ));
        if !self.capabilities.interactive {
            eprintln!("{}", format_new_progress_line(&progress, self.capabilities));
        }
    }

    pub(crate) fn finish_all(&self, capabilities: TerminalCapabilities) {
        self.bar.finish_and_clear();
        eprintln!(
            "{}  {}",
            paint("\u{2713}", ANSI_GREEN, capabilities.color),
            paint("Prerequisites ready", ANSI_BOLD, capabilities.color),
        );
    }
}

impl ProgressSink for NewProgressRenderer {
    fn on_progress(&mut self, progress: DownloadProgressView) {
        NewProgressRenderer::on_progress(self, progress);
    }
}

fn new_artifact_label(progress: &DownloadProgressView) -> String {
    match &progress.artifact_kind {
        taumaru_microvm::ArtifactKind::Kernel => match &progress.member_name {
            Some(_) => format!("kernel/{}", progress.artifact_id),
            None => format!("kernel/{}", progress.artifact_id),
        },
        taumaru_microvm::ArtifactKind::Binary => match &progress.member_name {
            Some(member) => format!("runtime/{}/{}", progress.artifact_id, member),
            None => format!("runtime/{}", progress.artifact_id),
        },
        taumaru_microvm::ArtifactKind::DistributionImage => match &progress.member_name {
            Some(member) => format!("image/{}/{}", progress.artifact_id, member),
            None => format!("image/{}", progress.artifact_id),
        },
    }
}

pub(crate) fn format_new_progress_line(
    progress: &DownloadProgressView,
    capabilities: TerminalCapabilities,
) -> String {
    let current = format_bytes(progress.current_bytes);
    let expected = format_bytes(progress.expected_bytes);
    let identity = new_artifact_label(progress);
    let phase = phase_label(&progress.phase);
    let marker = phase_marker(&progress.phase, capabilities.color);
    format!("{marker}  {identity}  {phase}  {current} / {expected}")
}

pub(crate) struct CreationProgressRenderer {
    bar: ProgressBar,
    capabilities: TerminalCapabilities,
}

impl CreationProgressRenderer {
    pub(crate) fn new(capabilities: TerminalCapabilities) -> Self {
        let bar = if capabilities.interactive {
            indicatif::ProgressBar::new(100)
        } else {
            indicatif::ProgressBar::hidden()
        };
        let template = if capabilities.color {
            "{spinner:.dim} {bar:28} {pos}/100 {msg}"
        } else {
            "{spinner} {bar:28} {pos}/100 {msg}"
        };
        let style = match ProgressStyle::with_template(template) {
            Ok(style) => style
                .progress_chars("\u{2501}\u{2578}\u{2500}")
                .tick_strings(&[
                    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}",
                    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
                ]),
            Err(_) => ProgressStyle::default_bar().progress_chars("=>-"),
        };
        bar.set_style(style);
        Self { bar, capabilities }
    }

    pub(crate) fn on_creation_progress(&mut self, progress: taumaru_microvm::CreationProgress) {
        self.bar.set_position(progress.overall_percent.min(100));
        self.bar.set_message(format!(
            "creation/{} · {}/{} · {}",
            progress.stage,
            progress.completed_steps,
            progress.total_steps,
            creation_phase_label(&progress.phase)
        ));
        if !self.capabilities.interactive {
            eprintln!(
                "{}",
                format_creation_progress_line(&progress, self.capabilities)
            );
        }
    }

    pub(crate) fn finish(self) {
        if self.capabilities.interactive {
            self.bar.finish_and_clear();
        }
    }
}

fn creation_phase_label(phase: &taumaru_microvm::CreationEventPhase) -> &'static str {
    match phase {
        taumaru_microvm::CreationEventPhase::Started => "Started",
        taumaru_microvm::CreationEventPhase::InProgress => "InProgress",
        taumaru_microvm::CreationEventPhase::Finished => "Finished",
    }
}

pub(crate) fn format_creation_progress_line(
    progress: &taumaru_microvm::CreationProgress,
    capabilities: TerminalCapabilities,
) -> String {
    let marker = match &progress.outcome {
        Some(taumaru_microvm::CreationOutcome::Failed { .. }) => {
            paint("\u{00d7}", ANSI_RED, capabilities.color)
        }
        Some(_) => paint("\u{2713}", ANSI_GREEN, capabilities.color),
        None => paint("\u{25c6}", ANSI_BLUE, capabilities.color),
    };
    let outcome = match &progress.outcome {
        Some(taumaru_microvm::CreationOutcome::Completed) => " · Completed",
        Some(taumaru_microvm::CreationOutcome::AlreadyConfigured) => " · AlreadyConfigured",
        Some(taumaru_microvm::CreationOutcome::Failed { stage }) => {
            return format!(
                "{marker}  creation/{stage}  Failed  {}/{} · {}%",
                progress.completed_steps, progress.total_steps, progress.overall_percent
            );
        }
        None => "",
    };
    match (progress.bytes_completed, progress.expected_bytes) {
        (Some(done), Some(expected)) => format!(
            "{marker}  creation/{}  {}  {}/{}  ·  {} B / {} B{outcome}",
            progress.stage,
            creation_phase_label(&progress.phase),
            progress.completed_steps,
            progress.total_steps,
            done,
            expected,
        ),
        _ => format!(
            "{marker}  creation/{}  {}  {}/{} · {}%{outcome}",
            progress.stage,
            creation_phase_label(&progress.phase),
            progress.completed_steps,
            progress.total_steps,
            progress.overall_percent,
        ),
    }
}

pub(crate) fn format_new_result(
    result: &taumaru_microvm::MicroVmCreationResult,
    request: &NewVmRequest,
    interrupted: bool,
    capabilities: TerminalCapabilities,
) -> String {
    let network = if request.expose_on_lan {
        format!("LAN exposed {}", result.network.guest_address)
    } else {
        format!("host-only {}", result.network.guest_address)
    };
    let capacity = format!(
        "{} · {} · {} vCPUs",
        format_gb(result.disk_size_bytes),
        format_mb_gb(result.memory_bytes),
        result.vcpu_count
    );
    let ssh = format!(
        "{}:{} · key {}",
        result.ssh.user,
        result.ssh.port,
        result.ssh.private_key_path.display()
    );
    let title = paint(
        format!("MicroVM {} created", result.name),
        ANSI_BOLD,
        capabilities.color,
    );
    let check = paint("\u{2713}", ANSI_GREEN, capabilities.color);
    let rule = divider(capabilities);
    let network_label = paint("Network:", ANSI_DIM, capabilities.color);
    let resources_label = paint("Resources:", ANSI_DIM, capabilities.color);
    let volume_label = paint("Volume:", ANSI_DIM, capabilities.color);
    let ssh_label = paint("SSH:", ANSI_DIM, capabilities.color);
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!("  {network_label} {network}\n"));
    output.push_str(&format!("  {resources_label} {capacity}\n"));
    output.push_str(&format!(
        "  {volume_label} {}\n",
        result.volume_path.display()
    ));
    output.push_str(&format!("  {ssh_label} {ssh}\n"));
    output.push_str(&format!(
        "  {} microvm start {}\n",
        paint("Start:", ANSI_DIM, capabilities.color),
        result.name
    ));
    if interrupted {
        output.push_str(&format!(
            "\n  {}\n",
            paint(
                "Note: an interrupt was received while creation settled; the result above reflects the settled operation.",
                ANSI_DIM,
                capabilities.color
            ),
        ));
    }
    output
}

pub(crate) fn write_new_result(
    result: &taumaru_microvm::MicroVmCreationResult,
    request: &NewVmRequest,
    interrupted: bool,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(
        stdout,
        "{}",
        format_new_result(result, request, interrupted, capabilities)
    )
}

pub(crate) fn format_start_result(
    result: &taumaru_microvm::MicroVmStartResult,
    ssh_prefix: &str,
    capabilities: TerminalCapabilities,
) -> String {
    let title = paint(
        format!("MicroVM {} running", result.name),
        ANSI_BOLD,
        capabilities.color,
    );
    let check = paint("✓", ANSI_GREEN, capabilities.color);
    let rule = divider(capabilities);
    let connect_label = paint("Connect:", ANSI_DIM, capabilities.color);
    let direct_label = paint("Direct:", ANSI_DIM, capabilities.color);
    let lan_label = paint("LAN:", ANSI_DIM, capabilities.color);
    let stop_label = paint("Stop:", ANSI_DIM, capabilities.color);
    let direct = format!(
        "{ssh_prefix}ssh -i {} -p {} {}@{}",
        result.ssh.private_key_path.display(),
        result.ssh.port,
        result.ssh.user,
        result.ssh.address
    );
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!("  {connect_label} microvm ssh {}\n", result.name));
    output.push_str(&format!("  {direct_label}  {direct}\n"));
    if result.network.mode == taumaru_microvm::NetworkMode::Lan {
        match result.network.lan_address {
            Some(address) => {
                output.push_str(&format!(
                    "  {lan_label}     copy {} to the other machine, then\n           ssh -i {} -p {} {}@{address}\n",
                    result.ssh.private_key_path.display(),
                    result
                        .ssh
                        .private_key_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| String::from("id_ed25519")),
                    result.ssh.port,
                    result.ssh.user,
                ));
            }
            None => {
                output.push_str(&format!("  {lan_label}     LAN exposed\n"));
            }
        }
    }
    output.push_str(&format!("  {stop_label}    microvm stop {}\n", result.name));
    output
}

pub(crate) fn write_start_result(
    result: &taumaru_microvm::MicroVmStartResult,
    ssh_prefix: &str,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(
        stdout,
        "{}",
        format_start_result(result, ssh_prefix, capabilities)
    )
}
pub(crate) fn format_stop_result(
    result: &taumaru_microvm::MicroVmStopResult,
    capabilities: TerminalCapabilities,
) -> String {
    let title = paint(
        format!("MicroVM {} stopped", result.name),
        ANSI_BOLD,
        capabilities.color,
    );
    let check = paint("✓", ANSI_GREEN, capabilities.color);
    let rule = divider(capabilities);
    let shutdown_label = paint("Shutdown:", ANSI_DIM, capabilities.color);
    let start_label = paint("Start:", ANSI_DIM, capabilities.color);
    let shutdown = if result.forced {
        "forced — the guest did not exit and was force-terminated"
    } else {
        "graceful — the guest exited on its own"
    };
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!("  {shutdown_label} {shutdown}\n"));
    output.push_str(&format!(
        "  {start_label}    microvm start {}\n",
        result.name
    ));
    output
}

pub(crate) fn write_stop_result(
    result: &taumaru_microvm::MicroVmStopResult,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_stop_result(result, capabilities))
}

pub(crate) fn format_delete_result(
    result: &taumaru_microvm::MicroVmDeleteResult,
    capabilities: TerminalCapabilities,
) -> String {
    let title = paint(
        format!("MicroVM {} deleted", result.name),
        ANSI_BOLD,
        capabilities.color,
    );
    let check = paint("✓", ANSI_GREEN, capabilities.color);
    let rule = divider(capabilities);
    let removed_label = paint("Removed:", ANSI_DIM, capabilities.color);
    let preserved_label = paint("Preserved:", ANSI_DIM, capabilities.color);
    let create_label = paint("Create:", ANSI_DIM, capabilities.color);
    let mut output = String::from("\n");
    output.push_str(&format!("{check} {title}\n{rule}\n\n"));
    output.push_str(&format!(
        "  {removed_label}  record, volume, and owned network attachment\n"
    ));
    output.push_str(&format!("  {preserved_label} shared kernels and images\n"));
    output.push_str(&format!("  {create_label}    microvm new\n"));
    output
}

pub(crate) fn write_delete_result(
    result: &taumaru_microvm::MicroVmDeleteResult,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_delete_result(result, capabilities))
}

fn ls_network_cell(summary: &taumaru_microvm::MicroVmSummary) -> String {
    let (Some(mode), Some(guest)) = (summary.network_mode, summary.guest_address) else {
        return String::from("-");
    };
    match (mode, summary.lan_address) {
        (taumaru_microvm::NetworkMode::HostOnly, _) => format!("host-only {guest}"),
        (taumaru_microvm::NetworkMode::Lan, Some(lan)) => {
            format!("lan {lan} (guest {guest})")
        }
        (taumaru_microvm::NetworkMode::Lan, None) => format!("lan (guest {guest})"),
    }
}

fn ls_state_text(summary: &taumaru_microvm::MicroVmSummary) -> &'static str {
    match summary.state {
        taumaru_microvm::MicroVmState::Running => "running",
        taumaru_microvm::MicroVmState::Stopped => "stopped",
    }
}

pub(crate) fn format_ls_table(
    summaries: &[taumaru_microvm::MicroVmSummary],
    capabilities: TerminalCapabilities,
) -> String {
    let headers = [
        "NAME", "STATE", "VCPUS", "MEMORY", "DISK", "IMAGE", "NETWORK",
    ];
    let rows: Vec<[String; 7]> = summaries
        .iter()
        .map(|summary| {
            [
                summary.name.clone(),
                ls_state_text(summary).to_owned(),
                summary.vcpu_count.to_string(),
                format_mb_gb(summary.memory_bytes),
                format_gb(summary.disk_size_bytes),
                format!("{}={}", summary.distribution_id, summary.image_id),
                ls_network_cell(summary),
            ]
        })
        .collect();
    let mut widths = [0usize; 7];
    for (index, header) in headers.iter().enumerate() {
        widths[index] = widths[index].max(header.len());
    }
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }
    let mut output = String::from("\n");
    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| format!("{:<width$}", header, width = widths[index]))
        .collect();
    output.push_str(&format!(
        "{}\n",
        paint(header_cells.join("  "), ANSI_DIM, capabilities.color)
    ));
    for (row_index, row) in rows.iter().enumerate() {
        let mut cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{:<width$}", cell, width = widths[index]))
            .collect();
        cells[1] = match summaries[row_index].state {
            taumaru_microvm::MicroVmState::Running => {
                paint(&cells[1], ANSI_GREEN, capabilities.color)
            }
            taumaru_microvm::MicroVmState::Stopped => {
                paint(&cells[1], ANSI_DIM, capabilities.color)
            }
        };
        output.push_str(&format!("{}\n", cells.join("  ")));
    }
    output
}

pub(crate) fn write_ls_table(
    summaries: &[taumaru_microvm::MicroVmSummary],
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_ls_table(summaries, capabilities))
}

pub(crate) fn format_ls_empty(capabilities: TerminalCapabilities) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n",
        paint("○", ANSI_DIM, capabilities.color),
        paint("No MicroVMs yet", ANSI_BOLD, capabilities.color)
    ));
    output.push_str(&format!(
        "  {} Run `microvm new` to create your first machine.\n",
        paint("Next:", ANSI_BOLD, capabilities.color)
    ));
    output
}

pub(crate) fn write_ls_empty(capabilities: TerminalCapabilities) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_ls_empty(capabilities))
}

pub(crate) fn format_prune_preview(
    kernels: &[String],
    images: &[(String, String)],
    estimated_bytes: u64,
    capabilities: TerminalCapabilities,
) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n{}\n\n",
        paint("◆", ANSI_BLUE, capabilities.color),
        paint("Prune preview", ANSI_BOLD, capabilities.color),
        divider(capabilities),
    ));
    output.push_str(&format!(
        "{}\n",
        paint("Kernels", ANSI_BOLD, capabilities.color)
    ));
    if kernels.is_empty() {
        output.push_str(&format!(
            "  {}  {}\n",
            paint("-", ANSI_DIM, capabilities.color),
            paint("no kernels to reclaim", ANSI_DIM, capabilities.color),
        ));
    } else {
        for kernel in kernels {
            output.push_str(&format!(
                "  {}  {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                kernel,
            ));
        }
    }
    output.push_str(&format!(
        "\n{}\n",
        paint("Images", ANSI_BOLD, capabilities.color)
    ));
    if images.is_empty() {
        output.push_str(&format!(
            "  {}  {}\n",
            paint("-", ANSI_DIM, capabilities.color),
            paint("no images to reclaim", ANSI_DIM, capabilities.color),
        ));
    } else {
        for (distribution, image) in images {
            output.push_str(&format!(
                "  {}  {} / {}\n",
                paint("•", ANSI_BLUE, capabilities.color),
                distribution,
                image,
            ));
        }
    }
    output.push_str(&format!(
        "\n{} · {} kernels · {} images\n\n",
        format_bytes(estimated_bytes),
        kernels.len(),
        images.len(),
    ));
    output.push_str(&format!(
        "{}\n",
        paint(
            "Review the artifacts above. Pruning starts after confirmation.",
            ANSI_DIM,
            capabilities.color,
        )
    ));
    output
}

pub(crate) fn write_prune_preview(
    kernels: &[String],
    images: &[(String, String)],
    estimated_bytes: u64,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(
        stdout,
        "{}",
        format_prune_preview(kernels, images, estimated_bytes, capabilities)
    )
}

pub(crate) fn format_prune_result(
    summary: &taumaru_microvm::PruneSummary,
    capabilities: TerminalCapabilities,
) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n{}\n\n",
        paint("✓", ANSI_GREEN, capabilities.color),
        paint("Artifacts pruned", ANSI_BOLD, capabilities.color),
        divider(capabilities),
    ));
    output.push_str(&format!(
        "  {} removed {} · {}\n",
        paint("Kernels:", ANSI_DIM, capabilities.color),
        kernel_count_text(summary.removed_kernels.len()),
        format_bytes(summary.freed_bytes_kernels),
    ));
    for kernel in &summary.removed_kernels {
        output.push_str(&format!(
            "  {}  {}\n",
            paint("•", ANSI_GREEN, capabilities.color),
            kernel,
        ));
    }
    output.push_str(&format!(
        "  {} removed {} · {}\n",
        paint("Images:", ANSI_DIM, capabilities.color),
        image_count_text(summary.removed_images.len()),
        format_bytes(summary.freed_bytes_images),
    ));
    for image in &summary.removed_images {
        output.push_str(&format!(
            "  {}  {} / {}\n",
            paint("•", ANSI_GREEN, capabilities.color),
            image.distribution_id,
            image.image_id,
        ));
    }
    output.push_str(&format!(
        "\n  {} {}\n",
        paint("Total freed:", ANSI_DIM, capabilities.color),
        format_bytes(summary.freed_bytes_total),
    ));
    if !summary.skipped_artifact_keys.is_empty() {
        output.push_str(&format!(
            "\n{}\n",
            paint(
                "Skipped (active transfer, kept intact)",
                ANSI_BOLD,
                capabilities.color
            )
        ));
        for key in &summary.skipped_artifact_keys {
            output.push_str(&format!(
                "  {}  {} · skipped\n",
                paint("○", ANSI_YELLOW, capabilities.color),
                key,
            ));
        }
    }
    output
}

pub(crate) fn write_prune_result(
    summary: &taumaru_microvm::PruneSummary,
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_prune_result(summary, capabilities))
}

pub(crate) fn format_prune_empty(capabilities: TerminalCapabilities) -> String {
    let mut output = String::from("\n");
    output.push_str(&format!(
        "{} {}\n",
        paint("○", ANSI_DIM, capabilities.color),
        paint("Nothing to prune", ANSI_BOLD, capabilities.color)
    ));
    output.push_str(&format!(
        "  {} Every downloaded kernel and image is referenced by an existing MicroVM.\n",
        paint("State:", ANSI_DIM, capabilities.color)
    ));
    output
}

pub(crate) fn write_prune_empty(capabilities: TerminalCapabilities) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", format_prune_empty(capabilities))
}

pub(crate) fn format_prune_partial(
    summary: &taumaru_microvm::PruneSummary,
    failures: &[taumaru_microvm::PruneFailure],
    capabilities: TerminalCapabilities,
) -> String {
    let mut output = format_prune_result(summary, capabilities);
    output.push_str(&format!(
        "\n{}\n",
        paint("Failed", ANSI_BOLD, capabilities.color)
    ));
    for failure in failures {
        output.push_str(&format!(
            "  {}  {} · failed — {}\n",
            paint("×", ANSI_RED, capabilities.color),
            failure.artifact_key,
            failure.reason,
        ));
    }
    output.push_str(&format!(
        "\n  {} Repair the cause above, then run `microvm artifacts prune` again.\n",
        paint("Next:", ANSI_BOLD, capabilities.color)
    ));
    output
}

pub(crate) fn write_prune_partial(
    summary: &taumaru_microvm::PruneSummary,
    failures: &[taumaru_microvm::PruneFailure],
    capabilities: TerminalCapabilities,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    write!(
        stdout,
        "{}",
        format_prune_partial(summary, failures, capabilities)
    )
}

fn kernel_count_text(count: usize) -> String {
    if count == 1 {
        String::from("1 kernel")
    } else {
        format!("{count} kernels")
    }
}

fn image_count_text(count: usize) -> String {
    if count == 1 {
        String::from("1 image")
    } else {
        format!("{count} images")
    }
}

#[cfg(test)]
mod prune_tests {
    use super::{
        format_prune_empty, format_prune_partial, format_prune_preview, format_prune_result,
    };
    use crate::context::TerminalCapabilities;

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn summary() -> taumaru_microvm::PruneSummary {
        taumaru_microvm::PruneSummary {
            removed_kernels: vec!["linux-6.18".to_owned()],
            removed_images: vec![taumaru_microvm::PrunedImageId {
                distribution_id: "ubuntu-24.04".to_owned(),
                image_id: "base".to_owned(),
            }],
            skipped_artifact_keys: vec!["kernel:busy-kernel".to_owned()],
            freed_bytes_kernels: 2048,
            freed_bytes_images: 3072,
            freed_bytes_total: 5120,
        }
    }

    #[test]
    fn preview_lists_identities_with_estimated_bytes() {
        let report = format_prune_preview(
            &["linux-6.18".to_owned()],
            &[("ubuntu-24.04".to_owned(), "base".to_owned())],
            5120,
            capabilities(),
        );
        assert!(report.contains("Prune preview"));
        assert!(report.contains("linux-6.18"));
        assert!(report.contains("ubuntu-24.04 / base"));
        assert!(report.contains("5.0 KiB"));
        assert!(report.contains("1 kernels"));
        assert!(report.contains("1 images"));
    }

    #[test]
    fn result_groups_removed_and_skipped_with_text_labels() {
        let report = format_prune_result(&summary(), capabilities());
        assert!(report.contains("Artifacts pruned"));
        assert!(report.contains("removed 1 kernel"));
        assert!(report.contains("removed 1 image"));
        assert!(report.contains("linux-6.18"));
        assert!(report.contains("ubuntu-24.04 / base"));
        assert!(report.contains("2.0 KiB"));
        assert!(report.contains("3.0 KiB"));
        assert!(report.contains("5.0 KiB"));
        assert!(report.contains("Skipped"));
        assert!(report.contains("kernel:busy-kernel"));
        assert!(report.contains("skipped"));
    }

    #[test]
    fn empty_report_is_calm_with_no_removal_list() {
        let report = format_prune_empty(capabilities());
        assert!(report.contains("Nothing to prune"));
        assert!(!report.contains("removed"));
        assert!(!report.contains("Failed"));
    }

    #[test]
    fn partial_keeps_removed_set_with_named_causes() {
        let failures = vec![taumaru_microvm::PruneFailure {
            artifact_key: "kernel:stuck-kernel".to_owned(),
            reason: "permission denied".to_owned(),
        }];
        let report = format_prune_partial(&summary(), &failures, capabilities());
        assert!(report.contains("linux-6.18"));
        assert!(report.contains("Failed"));
        assert!(report.contains("kernel:stuck-kernel"));
        assert!(report.contains("permission denied"));
        assert!(report.contains("microvm artifacts prune"));
    }
}

#[cfg(test)]
mod ls_tests {
    use super::{format_ls_empty, format_ls_table};
    use crate::context::TerminalCapabilities;
    use std::net::IpAddr;
    use taumaru_microvm::{MicroVmState, MicroVmSummary, NetworkMode};

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn summary(
        name: &str,
        state: MicroVmState,
        mode: Option<NetworkMode>,
        guest: Option<&str>,
        lan: Option<&str>,
    ) -> MicroVmSummary {
        MicroVmSummary {
            name: name.to_owned(),
            state,
            vcpu_count: 2,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            disk_size_bytes: 20 * 1024 * 1024 * 1024,
            distribution_id: "ubuntu-24.04".to_owned(),
            image_id: "base".to_owned(),
            network_mode: mode,
            guest_address: guest
                .map(|value| value.parse::<IpAddr>().expect("guest IP should parse")),
            lan_address: lan.map(|value| value.parse::<IpAddr>().expect("LAN IP should parse")),
        }
    }

    #[test]
    fn table_lists_every_machine_with_details_in_name_order() {
        let rows = vec![
            summary(
                "db-01",
                MicroVmState::Stopped,
                Some(NetworkMode::HostOnly),
                Some("10.200.8.2"),
                None,
            ),
            summary(
                "web-01",
                MicroVmState::Running,
                Some(NetworkMode::Lan),
                Some("10.200.8.3"),
                Some("192.168.1.50"),
            ),
        ];
        let table = format_ls_table(&rows, capabilities());
        assert!(table.contains("NAME"));
        assert!(table.contains("NETWORK"));
        assert!(table.contains("web-01"));
        assert!(table.contains("running"));
        assert!(table.contains("stopped"));
        assert!(table.contains("2 GB"));
        assert!(table.contains("20 GB"));
        assert!(table.contains("ubuntu-24.04=base"));
        assert!(table.contains("lan 192.168.1.50 (guest 10.200.8.3)"));
        assert!(table.contains("host-only 10.200.8.2"));
        assert!(
            table.find("db-01").expect("db-01 should render")
                < table.find("web-01").expect("web-01 should render")
        );
        assert!(!table.contains("id_ed25519"));
    }

    #[test]
    fn degraded_row_keeps_place_with_dashes() {
        let rows = vec![summary("half-01", MicroVmState::Stopped, None, None, None)];
        let table = format_ls_table(&rows, capabilities());
        assert!(table.contains("half-01"));
        assert!(table.contains("stopped"));
        assert!(table.contains('-'));
    }

    #[test]
    fn empty_report_points_at_creation_without_table() {
        let report = format_ls_empty(capabilities());
        assert!(report.contains("No MicroVMs yet"));
        assert!(report.contains("microvm new"));
        assert!(!report.contains("NAME"));
    }
}

#[cfg(test)]
mod stop_tests {
    use super::format_stop_result;
    use crate::context::TerminalCapabilities;
    use std::path::PathBuf;
    use taumaru_microvm::{MicroVmState, MicroVmStopResult};

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn stop_result(forced: bool) -> MicroVmStopResult {
        MicroVmStopResult {
            name: String::from("web-01"),
            state: MicroVmState::Stopped,
            socket_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01/firecracker.sock"),
            forced,
        }
    }

    #[test]
    fn graceful_report_names_machine_with_restart_hint() {
        let report = format_stop_result(&stop_result(false), capabilities());
        assert!(report.contains("MicroVM web-01 stopped"));
        assert!(report.contains("graceful"));
        assert!(!report.contains("forced"));
        assert!(report.contains("microvm start web-01"));
    }

    #[test]
    fn forced_report_is_text_distinguishable_from_graceful() {
        let forced = format_stop_result(&stop_result(true), capabilities());
        let graceful = format_stop_result(&stop_result(false), capabilities());
        assert!(forced.contains("forced"));
        assert!(!forced.contains("graceful — the guest exited on its own"));
        assert!(graceful.contains("graceful — the guest exited on its own"));
        assert_ne!(forced, graceful);
    }
}

#[cfg(test)]
mod delete_tests {
    use super::format_delete_result;
    use crate::context::TerminalCapabilities;
    use taumaru_microvm::MicroVmDeleteResult;

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn delete_result() -> MicroVmDeleteResult {
        MicroVmDeleteResult {
            name: String::from("web-01"),
        }
    }

    #[test]
    fn deleted_report_names_machine_with_removed_and_preserved_lines() {
        let report = format_delete_result(&delete_result(), capabilities());
        assert!(report.contains("MicroVM web-01 deleted"));
        assert!(report.contains("Removed:"));
        assert!(report.contains("record, volume, and owned network attachment"));
        assert!(report.contains("Preserved:"));
        assert!(report.contains("shared kernels and images"));
        assert!(report.contains("microvm new"));
    }

    #[test]
    fn deleted_report_stays_text_readable_without_color() {
        let plain = format_delete_result(&delete_result(), capabilities());
        let colored = format_delete_result(
            &delete_result(),
            TerminalCapabilities {
                interactive: false,
                color: true,
                width: Some(40),
            },
        );
        for report in [&plain, &colored] {
            assert!(report.contains("MicroVM web-01 deleted"));
            assert!(report.contains("Removed:"));
            assert!(report.contains("Preserved:"));
            assert!(report.contains("microvm new"));
        }
    }
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
mod new_tests {
    use super::{format_creation_progress_line, format_new_progress_line, format_new_review};
    use crate::commands::new::{NewImageChoice, NewProvisioningPlan, NewVmRequest};
    use crate::context::TerminalCapabilities;
    use taumaru_microvm::{
        Architecture, ArtifactKind, BinaryFile, BinaryPackage, BootConfiguration,
        CreationEventPhase, CreationProgress, CreationStage, Distribution, DistributionImage,
        DistributionRequirements, DownloadPhase, FilesystemMetadata, Kernel,
    };

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn choice() -> NewImageChoice {
        let image = DistributionImage {
            id: "image-a".to_owned(),
            name: "image-a".to_owned(),
            display_name: "Alpine Test Minimal".to_owned(),
            description: "image-a".to_owned(),
            variant: "minimal".to_owned(),
            path: "images/image-a".to_owned(),
            url: "https://example.invalid/image-a".to_owned(),
            filename: "image-a.ext4".to_owned(),
            format: "ext4".to_owned(),
            filesystem: FilesystemMetadata {
                filesystem_type: "ext4".to_owned(),
                uuid: "uuid-image-a".to_owned(),
                block_size: 4096,
                block_count: 1,
                free_blocks: 0,
                inode_count: 1,
                free_inodes: 0,
                features: Vec::new(),
            },
            size_bytes: 37,
            sha256: "0".repeat(64),
            capabilities: Vec::new(),
            mime_type: "application/octet-stream".to_owned(),
            modified_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let distribution = Distribution {
            id: "distro-a".to_owned(),
            name: "distro-a".to_owned(),
            display_name: "distro-a".to_owned(),
            description: "distro-a".to_owned(),
            distribution: "distro-a".to_owned(),
            version: "1.0".to_owned(),
            codename: "test".to_owned(),
            architecture: Architecture::X86_64,
            vendor: "Taumaru".to_owned(),
            homepage: "https://example.invalid".to_owned(),
            default_kernel: "kernel-a".to_owned(),
            supported_kernels: vec!["kernel-a".to_owned()],
            boot: BootConfiguration {
                root_device: "/dev/vda".to_owned(),
                kernel_args: Vec::new(),
            },
            requirements: DistributionRequirements {
                min_memory_mb: 128,
                min_vcpus: 1,
            },
            images: vec![image.clone()],
        };
        NewImageChoice {
            distribution,
            image,
            kernel: Kernel {
                id: "kernel-a".to_owned(),
                name: "kernel-a".to_owned(),
                display_name: "kernel-a".to_owned(),
                version: "6.2.0".to_owned(),
                architecture: Architecture::X86_64,
                path: "kernels/kernel-a/vmlinux".to_owned(),
                url: "https://example.invalid/kernel-a".to_owned(),
                filename: "vmlinux".to_owned(),
                size_bytes: 13,
                sha256: "0".repeat(64),
                format: "elf".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                elf: None,
                modified_at: "2026-01-01T00:00:00Z".to_owned(),
            },
            expected_bytes: 37,
        }
    }

    fn file(name: &str, size_bytes: u64) -> BinaryFile {
        BinaryFile {
            name: name.to_owned(),
            path: format!("binaries/runtime/{name}"),
            url: format!("https://example.invalid/{name}"),
            filename: name.to_owned(),
            size_bytes,
            sha256: "0".repeat(64),
            mime_type: "application/octet-stream".to_owned(),
            executable: true,
            mode: Some("755".to_owned()),
            permissions: None,
            format: Some("elf".to_owned()),
            elf: None,
            modified_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn new_review_names_values_and_missing_prerequisites() {
        let choice = choice();
        let request = NewVmRequest {
            name: "web-01".to_owned(),
            distribution_id: choice.distribution.id.clone(),
            image_id: choice.image.id.clone(),
            disk_size_bytes: 20 * 1024 * 1024 * 1024,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            vcpu_count: 2,
            expose_on_lan: false,
        };
        let plan = NewProvisioningPlan {
            runtime_packages: vec![BinaryPackage {
                id: "runtime-new".to_owned(),
                name: "firecracker".to_owned(),
                display_name: "runtime-new".to_owned(),
                description: None,
                version: "2.0.0".to_owned(),
                architecture: Architecture::X86_64,
                files: vec![file("firecracker", 19), file("firectl", 23)],
            }],
            kernel: choice.kernel.clone(),
            provision_bytes: 19 + 23 + 13 + 37,
        };
        let review = format_new_review(&request, &choice, &plan, "20", "2GB", capabilities());
        assert!(review.contains("web-01"));
        assert!(review.contains("distro-a / image-a"));
        assert!(review.contains("host-only"));
        assert!(review.contains("runtime/runtime-new"));
        assert!(review.contains("kernel/kernel-a"));
        assert!(review.contains("image/distro-a/image-a"));
        assert!(!review.contains("Download plan"));
    }

    #[test]
    fn new_progress_lines_keep_identity_stage_and_bytes_visible() {
        let line = format_new_progress_line(
            &crate::commands::download::DownloadProgressView {
                artifact_kind: ArtifactKind::DistributionImage,
                artifact_id: "distro-a".to_owned(),
                member_name: Some("image-a".to_owned()),
                phase: DownloadPhase::Downloading,
                current_bytes: 4,
                expected_bytes: 8,
                completed_plan_bytes: 0,
                aggregate_current_bytes: 4,
                aggregate_expected_bytes: 8,
                plan_total_bytes: 92,
            },
            capabilities(),
        );
        assert!(line.contains("image/distro-a/image-a"));
        assert!(line.contains("Downloading"));
        assert!(line.contains("4 B / 8 B"));
    }

    #[test]
    fn creation_progress_lines_keep_stage_counters_and_outcome_visible() {
        let line = format_creation_progress_line(
            &CreationProgress {
                stage: CreationStage::VolumePreparation,
                completed_steps: 2,
                total_steps: 6,
                overall_percent: 33,
                phase: CreationEventPhase::InProgress,
                bytes_completed: Some(4),
                expected_bytes: Some(8),
                outcome: None,
            },
            capabilities(),
        );
        assert!(line.contains("creation/volume_preparation"));
        assert!(line.contains("2/6"));
        assert!(line.contains("4 B / 8 B"));
    }
}

#[cfg(test)]
mod start_tests {
    use super::{format_new_result, format_start_result};
    use crate::commands::new::NewVmRequest;
    use crate::context::TerminalCapabilities;
    use std::net::IpAddr;
    use std::path::PathBuf;
    use taumaru_microvm::{
        MicroVmCreationResult, MicroVmStartResult, MicroVmState, NetworkConfiguration, NetworkMode,
        SshConnectionInfo,
    };

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        }
    }

    fn ssh() -> SshConnectionInfo {
        SshConnectionInfo {
            user: String::from("root"),
            port: 22,
            address: "192.168.127.2"
                .parse::<IpAddr>()
                .expect("test address parses"),
            private_key_path: PathBuf::from(
                "/home/user/.taumaru-microvm/vms/web-01/ssh/id_ed25519",
            ),
            public_key_path: PathBuf::from(
                "/home/user/.taumaru-microvm/vms/web-01/ssh/id_ed25519.pub",
            ),
        }
    }

    fn network(mode: NetworkMode, lan_address: Option<IpAddr>) -> NetworkConfiguration {
        NetworkConfiguration {
            mode,
            guest_address: "192.168.127.2"
                .parse::<IpAddr>()
                .expect("test address parses"),
            prefix_length: 30,
            gateway: None,
            tap_name: String::from("tap-web-01"),
            bridge_name: None,
            uplink_name: None,
            lan_address,
        }
    }

    fn start_result(mode: NetworkMode, lan_address: Option<IpAddr>) -> MicroVmStartResult {
        MicroVmStartResult {
            name: String::from("web-01"),
            state: MicroVmState::Running,
            volume_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01"),
            rootfs_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01/rootfs.ext4"),
            socket_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01/firecracker.sock"),
            process_id: 4242,
            network: network(mode, lan_address),
            ssh: ssh(),
        }
    }

    fn creation_result() -> MicroVmCreationResult {
        MicroVmCreationResult {
            name: String::from("web-01"),
            state: MicroVmState::Stopped,
            distribution_id: String::from("distro-a"),
            image_id: String::from("image-a"),
            volume_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01"),
            rootfs_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01/rootfs.ext4"),
            socket_path: PathBuf::from("/home/user/.taumaru-microvm/vms/web-01/firecracker.sock"),
            vcpu_count: 2,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            disk_size_bytes: 20 * 1024 * 1024 * 1024,
            network: network(NetworkMode::HostOnly, None),
            ssh: ssh(),
        }
    }

    #[test]
    fn host_only_report_orders_hints_without_lan_paragraph() {
        let report = format_start_result(
            &start_result(NetworkMode::HostOnly, None),
            "",
            capabilities(),
        );
        let connect = report
            .find("microvm ssh web-01")
            .expect("connect hint renders");
        let direct = report.find("ssh -i").expect("direct hint renders");
        let stop = report
            .find("microvm stop web-01")
            .expect("stop hint renders");
        assert!(connect < direct);
        assert!(direct < stop);
        assert!(!report.contains("copy "));
        assert!(!report.contains("LAN:"));
        assert!(report.contains("MicroVM web-01 running"));
    }

    #[test]
    fn lan_report_uses_lan_address_with_elevation_prefix() {
        let lan: IpAddr = "192.168.10.30"
            .parse::<IpAddr>()
            .expect("test address parses");
        let report = format_start_result(
            &start_result(NetworkMode::Lan, Some(lan)),
            "sudo ",
            capabilities(),
        );
        assert!(report.contains("sudo ssh -i"));
        assert!(report.contains("copy /home/user/.taumaru-microvm/vms/web-01/ssh/id_ed25519"));
        assert!(report.contains("root@192.168.10.30"));
        assert!(!report.contains("id_ed25519\n"));
    }

    #[test]
    fn new_report_appends_start_command_with_existing_rows() {
        let request = NewVmRequest {
            name: String::from("web-01"),
            distribution_id: String::from("distro-a"),
            image_id: String::from("image-a"),
            disk_size_bytes: 20 * 1024 * 1024 * 1024,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            vcpu_count: 2,
            expose_on_lan: false,
        };
        let report = format_new_result(&creation_result(), &request, false, capabilities());
        assert!(report.contains("MicroVM web-01 created"));
        assert!(report.contains("host-only"));
        assert!(report.contains("Start: microvm start web-01"));
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
    fn image_progress_line_keeps_distribution_and_image_visible() {
        let mut image_view = view();
        image_view.artifact_kind = ArtifactKind::DistributionImage;
        image_view.artifact_id = "distro-a".to_owned();
        image_view.member_name = Some("image-a".to_owned());
        let line = format_progress_line(
            &image_view,
            TerminalCapabilities {
                interactive: false,
                color: false,
                width: Some(50),
            },
        );
        assert!(line.contains("distribution/distro-a/image-a"));
        assert!(line.contains("Downloading"));
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
