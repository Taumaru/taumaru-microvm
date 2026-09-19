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
