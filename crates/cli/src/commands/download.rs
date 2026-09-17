use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::io;

use inquire::ui::{Color, RenderConfig, StyleSheet, Styled};
use inquire::{Confirm, InquireError, MultiSelect, Select};
use semver::Version;
use taumaru_microvm::{
    Architecture, BinaryPackage, Distribution, DownloadCancellation, DownloadProgress,
    DownloadedBinary, DownloadedDistribution, DownloadedKernel, Kernel, MicroVmSdk, SdkError,
};

use crate::cli::DownloadArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::ProgressSink;

trait ArtifactClient {
    async fn list_kernels(&self) -> Result<Vec<Kernel>, SdkError>;
    async fn list_binaries(&self) -> Result<Vec<BinaryPackage>, SdkError>;
    async fn list_distributions(&self) -> Result<Vec<Distribution>, SdkError>;

    async fn download_kernel<F>(
        &self,
        kernel_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedKernel, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;

    async fn download_binary<F>(
        &self,
        binary_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedBinary, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;

    async fn download_distribution<F>(
        &self,
        distribution_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedDistribution, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;
}

struct SdkArtifactClient<'a> {
    sdk: &'a MicroVmSdk,
}

impl<'a> SdkArtifactClient<'a> {
    fn new(sdk: &'a MicroVmSdk) -> Self {
        Self { sdk }
    }
}

impl ArtifactClient for SdkArtifactClient<'_> {
    async fn list_kernels(&self) -> Result<Vec<Kernel>, SdkError> {
        self.sdk.list_kernels().await
    }

    async fn list_binaries(&self) -> Result<Vec<BinaryPackage>, SdkError> {
        self.sdk.list_binaries().await
    }

    async fn list_distributions(&self) -> Result<Vec<Distribution>, SdkError> {
        self.sdk.list_distributions().await
    }

    async fn download_kernel<F>(
        &self,
        kernel_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedKernel, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        self.sdk
            .download_kernel_with_cancellation(kernel_id, cancellation, on_progress)
            .await
    }

    async fn download_binary<F>(
        &self,
        binary_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedBinary, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        self.sdk
            .download_binary_with_cancellation(binary_id, cancellation, on_progress)
            .await
    }

    async fn download_distribution<F>(
        &self,
        distribution_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedDistribution, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        self.sdk
            .download_distribution_with_cancellation(distribution_id, cancellation, on_progress)
            .await
    }
}

struct ProgressForwarder<'a, S: ProgressSink> {
    sink: &'a mut S,
    completed_plan_bytes: u64,
    plan_total_bytes: u64,
    mapping_error: Option<CliError>,
}

impl<'a, S: ProgressSink> ProgressForwarder<'a, S> {
    fn new(sink: &'a mut S, completed_plan_bytes: u64, plan_total_bytes: u64) -> Self {
        Self {
            sink,
            completed_plan_bytes,
            plan_total_bytes,
            mapping_error: None,
        }
    }

    fn forward(&mut self, progress: DownloadProgress) {
        if self.mapping_error.is_some() {
            return;
        }
        match normalize_progress(&progress, self.completed_plan_bytes, self.plan_total_bytes) {
            Ok(view) => self.sink.on_progress(view),
            Err(error) => self.mapping_error = Some(error),
        }
    }

    fn take_error(&mut self) -> Option<CliError> {
        self.mapping_error.take()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DownloadProgressView {
    pub(crate) artifact_kind: taumaru_microvm::ArtifactKind,
    pub(crate) artifact_id: String,
    pub(crate) member_name: Option<String>,
    pub(crate) phase: taumaru_microvm::DownloadPhase,
    pub(crate) current_bytes: u64,
    pub(crate) expected_bytes: u64,
    pub(crate) completed_plan_bytes: u64,
    pub(crate) aggregate_current_bytes: u64,
    pub(crate) aggregate_expected_bytes: u64,
    pub(crate) plan_total_bytes: u64,
}

fn normalize_progress(
    progress: &DownloadProgress,
    completed_plan_bytes: u64,
    plan_total_bytes: u64,
) -> Result<DownloadProgressView, CliError> {
    let aggregate_current_bytes = completed_plan_bytes
        .checked_add(progress.aggregate_bytes_received)
        .ok_or_else(|| {
            CliError::Validation("download progress exceeded the supported size range".to_owned())
        })?;
    let aggregate_expected_bytes = completed_plan_bytes
        .checked_add(progress.aggregate_total_bytes)
        .ok_or_else(|| {
            CliError::Validation("download progress exceeded the supported size range".to_owned())
        })?;
    Ok(DownloadProgressView {
        artifact_kind: progress.artifact_kind.clone(),
        artifact_id: progress.artifact_id.clone(),
        member_name: progress.member_name.clone(),
        phase: progress.phase.clone(),
        current_bytes: progress.bytes_received,
        expected_bytes: progress.total_bytes,
        completed_plan_bytes,
        aggregate_current_bytes,
        aggregate_expected_bytes,
        plan_total_bytes,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct RegistryCatalog {
    kernels: Vec<Kernel>,
    binaries: Vec<BinaryPackage>,
    distributions: Vec<Distribution>,
    host_architecture: Architecture,
}

#[derive(Clone, Debug)]
pub(crate) struct DistributionSelection {
    pub(crate) distribution: Distribution,
    pub(crate) kernel: Kernel,
    pub(crate) kernel_is_default: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeBinarySelection {
    pub(crate) package: BinaryPackage,
    pub(crate) files: Vec<taumaru_microvm::BinaryFile>,
    pub(crate) expected_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlanMember {
    RuntimeBinary {
        package_id: String,
        expected_bytes: u64,
    },
    Kernel {
        kernel_id: String,
        expected_bytes: u64,
    },
    DistributionImages {
        distribution_id: String,
        image_count: usize,
        expected_bytes: u64,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadPlan {
    pub(crate) runtime: RuntimeBinarySelection,
    pub(crate) selections: Vec<DistributionSelection>,
    pub(crate) members: Vec<PlanMember>,
    pub(crate) expected_bytes: u64,
}

impl RegistryCatalog {
    fn new(
        mut kernels: Vec<Kernel>,
        mut binaries: Vec<BinaryPackage>,
        mut distributions: Vec<Distribution>,
        host_architecture: Architecture,
    ) -> Result<Self, CliError> {
        validate_ids(&kernels, "kernel", |kernel| &kernel.id)?;
        validate_ids(&binaries, "binary package", |binary| &binary.id)?;
        validate_ids(&distributions, "distribution", |distribution| {
            &distribution.id
        })?;
        for distribution in &distributions {
            validate_ids(&distribution.images, "distribution image", |image| {
                &image.id
            })?;
        }
        kernels.sort_by(|left, right| left.id.cmp(&right.id));
        binaries.sort_by(|left, right| left.id.cmp(&right.id));
        distributions.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(Self {
            kernels,
            binaries,
            distributions,
            host_architecture,
        })
    }

    fn compatible_distributions(&self) -> Vec<&Distribution> {
        self.distributions
            .iter()
            .filter(|distribution| {
                same_architecture(&distribution.architecture, &self.host_architecture)
            })
            .collect()
    }

    fn compatible_kernels(&self, distribution: &Distribution) -> Result<Vec<&Kernel>, CliError> {
        if !same_architecture(&distribution.architecture, &self.host_architecture) {
            return Err(CliError::Validation(format!(
                "distribution {} is not published for host architecture {}",
                distribution.id,
                architecture_label(&distribution.architecture)
            )));
        }
        let mut supported_ids = HashSet::new();
        let mut kernels = Vec::new();
        for kernel_id in &distribution.supported_kernels {
            if !supported_ids.insert(kernel_id.as_str()) {
                return Err(CliError::Validation(format!(
                    "distribution {} lists kernel {} more than once",
                    distribution.id, kernel_id
                )));
            }
            let kernel = self
                .kernels
                .iter()
                .find(|candidate| candidate.id == *kernel_id)
                .ok_or_else(|| {
                    CliError::Validation(format!(
                        "distribution {} references unavailable kernel {}",
                        distribution.id, kernel_id
                    ))
                })?;
            if same_architecture(&kernel.architecture, &self.host_architecture)
                && same_architecture(&kernel.architecture, &distribution.architecture)
            {
                kernels.push(kernel);
            }
        }
        kernels.sort_by(|left, right| left.id.cmp(&right.id));
        if kernels.is_empty() {
            return Err(CliError::Validation(format!(
                "distribution {} has no compatible kernels for host architecture {}",
                distribution.id,
                architecture_label(&self.host_architecture)
            )));
        }
        Ok(kernels)
    }

    fn select_runtime_package(&self) -> Result<RuntimeBinarySelection, CliError> {
        let mut candidates = self
            .binaries
            .iter()
            .filter(|package| {
                same_architecture(&package.architecture, &self.host_architecture)
                    && has_required_runtime_components(package)
            })
            .filter_map(|package| {
                Version::parse(&package.version)
                    .ok()
                    .map(|version| (version, package))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(
            |(left_version, left_package), (right_version, right_package)| {
                right_version
                    .cmp(left_version)
                    .then_with(|| left_package.id.cmp(&right_package.id))
            },
        );
        let (_, package) = candidates.first().ok_or_else(|| {
            CliError::Validation(format!(
                "no valid runtime package contains firecracker and firectl for host architecture {}",
                architecture_label(&self.host_architecture)
            ))
        })?;
        let mut files = package.files.clone();
        files.sort_by(|left, right| left.name.cmp(&right.name));
        let expected_bytes = files.iter().try_fold(0_u64, |total, file| {
            total.checked_add(file.size_bytes).ok_or_else(|| {
                CliError::Validation(format!(
                    "runtime package {} exceeds the supported size range",
                    package.id
                ))
            })
        })?;
        Ok(RuntimeBinarySelection {
            package: (*package).clone(),
            files,
            expected_bytes,
        })
    }

    fn distribution(&self, id: &str) -> Option<&Distribution> {
        self.distributions
            .iter()
            .find(|distribution| distribution.id == id)
    }

    fn kernel(&self, id: &str) -> Option<&Kernel> {
        self.kernels.iter().find(|kernel| kernel.id == id)
    }

    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (
            self.distributions.len(),
            self.kernels.len(),
            self.binaries.len(),
        )
    }
}

fn validate_ids<T, F>(items: &[T], kind: &str, id: F) -> Result<(), CliError>
where
    F: Fn(&T) -> &str,
{
    let mut seen = HashSet::new();
    for item in items {
        let identifier = id(item);
        if identifier.trim().is_empty() {
            return Err(CliError::Validation(format!(
                "{kind} identifier cannot be empty"
            )));
        }
        if !seen.insert(identifier) {
            return Err(CliError::Validation(format!(
                "duplicate {kind} identifier {identifier}"
            )));
        }
    }
    Ok(())
}

fn same_architecture(left: &Architecture, right: &Architecture) -> bool {
    matches!(
        (left, right),
        (Architecture::X86_64, Architecture::X86_64)
            | (Architecture::Aarch64, Architecture::Aarch64)
            | (Architecture::Arm, Architecture::Arm)
            | (Architecture::Riscv64, Architecture::Riscv64)
            | (Architecture::X86, Architecture::X86)
    )
}

fn architecture_label(architecture: &Architecture) -> &'static str {
    match architecture {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::Riscv64 => "riscv64",
        Architecture::X86 => "x86",
    }
}

fn host_architecture() -> Result<Architecture, CliError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(Architecture::X86_64),
        "aarch64" => Ok(Architecture::Aarch64),
        "arm" | "armv7" | "armv7l" => Ok(Architecture::Arm),
        "riscv64" => Ok(Architecture::Riscv64),
        "x86" | "i686" => Ok(Architecture::X86),
        architecture => Err(CliError::Validation(format!(
            "host architecture {architecture} is not supported by the registry"
        ))),
    }
}

fn has_required_runtime_components(package: &BinaryPackage) -> bool {
    ["firecracker", "firectl"]
        .iter()
        .all(|required| package.files.iter().any(|file| file.name == *required))
}

fn parse_kernel_mappings(values: &[String]) -> Result<HashMap<String, String>, CliError> {
    let mut mappings = HashMap::new();
    for value in values {
        let parts = value.split('=').collect::<Vec<_>>();
        if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
            return Err(CliError::Validation(format!(
                "kernel mapping {value:?} must use DISTRIBUTION_ID=KERNEL_ID"
            )));
        }
        let distribution_id = parts[0].to_owned();
        let kernel_id = parts[1].to_owned();
        if mappings
            .insert(distribution_id.clone(), kernel_id)
            .is_some()
        {
            return Err(CliError::Validation(format!(
                "distribution {distribution_id} has more than one kernel mapping"
            )));
        }
    }
    Ok(mappings)
}

fn build_plan(
    catalog: &RegistryCatalog,
    selected_distribution_ids: &[String],
    kernel_mappings: &HashMap<String, String>,
) -> Result<DownloadPlan, CliError> {
    if selected_distribution_ids.is_empty() {
        return Err(CliError::Validation(
            "at least one distribution must be selected".to_owned(),
        ));
    }
    validate_ids(selected_distribution_ids, "selected distribution", |id| id)?;
    for distribution_id in kernel_mappings.keys() {
        if !selected_distribution_ids
            .iter()
            .any(|selected| selected == distribution_id)
        {
            return Err(CliError::Validation(format!(
                "kernel mapping for unselected distribution {distribution_id}"
            )));
        }
    }

    let mut distribution_ids = selected_distribution_ids.to_vec();
    distribution_ids.sort();
    let mut selections = Vec::with_capacity(distribution_ids.len());
    for distribution_id in distribution_ids {
        let distribution = catalog.distribution(&distribution_id).ok_or_else(|| {
            CliError::Validation(format!(
                "distribution {distribution_id} is unavailable for this host"
            ))
        })?;
        let kernel_id = kernel_mappings.get(&distribution_id).ok_or_else(|| {
            CliError::Validation(format!(
                "distribution {distribution_id} requires exactly one kernel mapping"
            ))
        })?;
        let kernel = catalog.kernel(kernel_id).ok_or_else(|| {
            CliError::Validation(format!(
                "kernel {kernel_id} is not available in the registry"
            ))
        })?;
        let compatible_kernels = catalog.compatible_kernels(distribution)?;
        if !compatible_kernels
            .iter()
            .any(|candidate| candidate.id == kernel.id)
        {
            return Err(CliError::Validation(format!(
                "kernel {kernel_id} is not compatible with distribution {distribution_id}"
            )));
        }
        selections.push(DistributionSelection {
            distribution: distribution.clone(),
            kernel: kernel.clone(),
            kernel_is_default: distribution.default_kernel == kernel.id,
        });
    }

    let runtime = catalog.select_runtime_package()?;
    let mut unique_kernels = BTreeMap::new();
    for selection in &selections {
        unique_kernels
            .entry(selection.kernel.id.clone())
            .or_insert_with(|| selection.kernel.clone());
    }

    let mut members = vec![PlanMember::RuntimeBinary {
        package_id: runtime.package.id.clone(),
        expected_bytes: runtime.expected_bytes,
    }];
    let mut expected_bytes = runtime.expected_bytes;
    for kernel in unique_kernels.values() {
        expected_bytes = checked_size_add(expected_bytes, kernel.size_bytes, "kernel")?;
        members.push(PlanMember::Kernel {
            kernel_id: kernel.id.clone(),
            expected_bytes: kernel.size_bytes,
        });
    }
    for selection in &selections {
        let image_bytes =
            selection
                .distribution
                .images
                .iter()
                .try_fold(0_u64, |total, image| {
                    total.checked_add(image.size_bytes).ok_or_else(|| {
                        CliError::Validation(format!(
                            "distribution {} image sizes exceed the supported range",
                            selection.distribution.id
                        ))
                    })
                })?;
        expected_bytes = checked_size_add(expected_bytes, image_bytes, "distribution images")?;
        members.push(PlanMember::DistributionImages {
            distribution_id: selection.distribution.id.clone(),
            image_count: selection.distribution.images.len(),
            expected_bytes: image_bytes,
        });
    }
    Ok(DownloadPlan {
        runtime,
        selections,
        members,
        expected_bytes,
    })
}

fn checked_size_add(total: u64, value: u64, kind: &str) -> Result<u64, CliError> {
    total.checked_add(value).ok_or_else(|| {
        CliError::Validation(format!(
            "{kind} sizes exceed the supported download size range"
        ))
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Availability {
    Downloaded,
    Adopted,
    AlreadyAvailable,
    Mixed,
}

#[derive(Clone, Debug)]
pub(crate) enum VerifiedArtifact {
    Binary(Box<DownloadedBinary>),
    Kernel(Box<DownloadedKernel>),
    Distribution(Box<DownloadedDistribution>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemberOutcome {
    Verified {
        label: String,
        availability: Availability,
    },
    Failed {
        label: String,
        reason: String,
    },
    Skipped {
        label: String,
        reason: String,
    },
    Cancelled {
        label: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadOutcome {
    pub(crate) verified: Vec<VerifiedArtifact>,
    pub(crate) groups: Vec<MemberOutcome>,
    pub(crate) cancelled: bool,
    pub(crate) expected_bytes: u64,
    pub(crate) available_bytes: u64,
}

impl DownloadOutcome {
    fn new(expected_bytes: u64) -> Self {
        Self {
            verified: Vec::new(),
            groups: Vec::new(),
            cancelled: false,
            expected_bytes,
            available_bytes: 0,
        }
    }

    fn add_available_bytes(&mut self, bytes: u64) -> Result<(), CliError> {
        self.available_bytes = self.available_bytes.checked_add(bytes).ok_or_else(|| {
            CliError::Validation("verified download size exceeded the supported range".to_owned())
        })?;
        Ok(())
    }

    pub(crate) fn is_success(&self) -> bool {
        !self.cancelled
            && self
                .groups
                .iter()
                .all(|outcome| matches!(outcome, MemberOutcome::Verified { .. }))
    }

    pub(crate) fn exit_code(&self) -> u8 {
        if self.cancelled {
            130
        } else if self.is_success() {
            0
        } else {
            1
        }
    }
}

enum OperationResult<T> {
    Finished(Result<T, SdkError>),
    Cancelled,
}

async fn call_with_signal<T, O, Sig>(
    operation: O,
    cancellation: &DownloadCancellation,
    signal: Sig,
) -> Result<OperationResult<T>, CliError>
where
    O: Future<Output = Result<T, SdkError>>,
    Sig: Future<Output = Result<(), io::Error>>,
{
    tokio::pin!(operation);
    tokio::pin!(signal);
    tokio::select! {
        result = &mut operation => {
            if matches!(result, Err(SdkError::Cancelled)) {
                Ok(OperationResult::Cancelled)
            } else {
                Ok(OperationResult::Finished(result))
            }
        }
        signal_result = &mut signal => {
            match signal_result {
                Ok(()) => {
                    cancellation.cancel();
                    let _ = operation.await;
                    Ok(OperationResult::Cancelled)
                }
                Err(error) => Err(CliError::Io(error)),
            }
        }
    }
}

async fn execute_plan_with_signals<C, S, Factory, Sig>(
    client: &C,
    plan: &DownloadPlan,
    cancellation: &DownloadCancellation,
    sink: &mut S,
    mut signal_factory: Factory,
) -> Result<DownloadOutcome, CliError>
where
    C: ArtifactClient,
    S: ProgressSink + Send,
    Factory: FnMut() -> Sig,
    Sig: Future<Output = Result<(), io::Error>>,
{
    let mut outcome = DownloadOutcome::new(plan.expected_bytes);
    let mut completed_plan_bytes = 0_u64;

    if cancellation.is_cancelled() {
        outcome.cancelled = true;
        append_cancelled_after_runtime(plan, &mut outcome);
        return Ok(outcome);
    }

    let runtime_id = plan.runtime.package.id.clone();
    let mut runtime_progress =
        ProgressForwarder::new(sink, completed_plan_bytes, plan.expected_bytes);
    let runtime_call = client.download_binary(&runtime_id, cancellation, |progress| {
        runtime_progress.forward(progress)
    });
    let runtime_result = call_with_signal(runtime_call, cancellation, signal_factory()).await?;
    if let Some(error) = runtime_progress.take_error() {
        return Err(error);
    }
    match runtime_result {
        OperationResult::Finished(Ok(result)) => {
            let availability = availability_for_files(&result.files);
            outcome.add_available_bytes(plan.runtime.expected_bytes)?;
            completed_plan_bytes = checked_size_add(
                completed_plan_bytes,
                plan.runtime.expected_bytes,
                "completed runtime",
            )?;
            outcome
                .verified
                .push(VerifiedArtifact::Binary(Box::new(result)));
            outcome.groups.push(MemberOutcome::Verified {
                label: format!("runtime/{runtime_id}"),
                availability,
            });
        }
        OperationResult::Finished(Err(error)) => {
            outcome.groups.push(MemberOutcome::Failed {
                label: format!("runtime/{runtime_id}"),
                reason: error.to_string(),
            });
            return Ok(outcome);
        }
        OperationResult::Cancelled => {
            outcome.cancelled = true;
            outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("runtime/{runtime_id}"),
            });
            append_cancelled_after_runtime(plan, &mut outcome);
            return Ok(outcome);
        }
    }

    let kernel_ids = plan
        .members
        .iter()
        .filter_map(|member| match member {
            PlanMember::Kernel { kernel_id, .. } => Some(kernel_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut failed_kernels = HashSet::new();
    for (index, kernel_id) in kernel_ids.iter().enumerate() {
        if cancellation.is_cancelled() {
            outcome.cancelled = true;
            outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("kernel/{kernel_id}"),
            });
            append_cancelled_after_kernel(plan, &mut outcome, index + 1);
            return Ok(outcome);
        }
        let expected_bytes = plan
            .members
            .iter()
            .find_map(|member| match member {
                PlanMember::Kernel {
                    kernel_id: member_kernel_id,
                    expected_bytes,
                } if member_kernel_id == kernel_id => Some(*expected_bytes),
                _ => None,
            })
            .ok_or_else(|| {
                CliError::Validation(format!(
                    "kernel {kernel_id} is missing from the download plan"
                ))
            })?;
        let mut kernel_progress =
            ProgressForwarder::new(sink, completed_plan_bytes, plan.expected_bytes);
        let kernel_call = client.download_kernel(kernel_id, cancellation, |progress| {
            kernel_progress.forward(progress)
        });
        let kernel_result = call_with_signal(kernel_call, cancellation, signal_factory()).await?;
        if let Some(error) = kernel_progress.take_error() {
            return Err(error);
        }
        match kernel_result {
            OperationResult::Finished(Ok(result)) => {
                let label = format!("kernel/{kernel_id}");
                outcome.add_available_bytes(expected_bytes)?;
                completed_plan_bytes =
                    checked_size_add(completed_plan_bytes, expected_bytes, "completed kernel")?;
                outcome.groups.push(MemberOutcome::Verified {
                    label,
                    availability: availability_for_files(std::slice::from_ref(&result.file)),
                });
                outcome
                    .verified
                    .push(VerifiedArtifact::Kernel(Box::new(result)));
            }
            OperationResult::Finished(Err(error)) => {
                failed_kernels.insert(kernel_id.clone());
                outcome.groups.push(MemberOutcome::Failed {
                    label: format!("kernel/{kernel_id}"),
                    reason: error.to_string(),
                });
            }
            OperationResult::Cancelled => {
                outcome.cancelled = true;
                outcome.groups.push(MemberOutcome::Cancelled {
                    label: format!("kernel/{kernel_id}"),
                });
                append_cancelled_after_kernel(plan, &mut outcome, index + 1);
                return Ok(outcome);
            }
        }
    }

    for (index, selection) in plan.selections.iter().enumerate() {
        let distribution_id = &selection.distribution.id;
        if failed_kernels.contains(&selection.kernel.id) {
            outcome.groups.push(MemberOutcome::Skipped {
                label: format!("distribution/{distribution_id}"),
                reason: format!("kernel/{} failed", selection.kernel.id),
            });
            continue;
        }
        if cancellation.is_cancelled() {
            outcome.cancelled = true;
            outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("distribution/{distribution_id}"),
            });
            append_cancelled_after_distribution(plan, &mut outcome, index + 1);
            return Ok(outcome);
        }
        let expected_bytes = plan
            .members
            .iter()
            .find_map(|member| match member {
                PlanMember::DistributionImages {
                    distribution_id: member_distribution_id,
                    expected_bytes,
                    ..
                } if member_distribution_id == distribution_id => Some(*expected_bytes),
                _ => None,
            })
            .ok_or_else(|| {
                CliError::Validation(format!(
                    "distribution {distribution_id} is missing from the download plan"
                ))
            })?;
        let mut distribution_progress =
            ProgressForwarder::new(sink, completed_plan_bytes, plan.expected_bytes);
        let distribution_call =
            client.download_distribution(distribution_id, cancellation, |progress| {
                distribution_progress.forward(progress)
            });
        let distribution_result =
            call_with_signal(distribution_call, cancellation, signal_factory()).await?;
        if let Some(error) = distribution_progress.take_error() {
            return Err(error);
        }
        match distribution_result {
            OperationResult::Finished(Ok(result)) => {
                outcome.add_available_bytes(expected_bytes)?;
                completed_plan_bytes = checked_size_add(
                    completed_plan_bytes,
                    expected_bytes,
                    "completed distribution",
                )?;
                outcome.groups.push(MemberOutcome::Verified {
                    label: format!("distribution/{distribution_id}"),
                    availability: availability_for_files(&result.images),
                });
                outcome
                    .verified
                    .push(VerifiedArtifact::Distribution(Box::new(result)));
            }
            OperationResult::Finished(Err(error)) => {
                outcome.groups.push(MemberOutcome::Failed {
                    label: format!("distribution/{distribution_id}"),
                    reason: error.to_string(),
                });
            }
            OperationResult::Cancelled => {
                outcome.cancelled = true;
                outcome.groups.push(MemberOutcome::Cancelled {
                    label: format!("distribution/{distribution_id}"),
                });
                append_cancelled_after_distribution(plan, &mut outcome, index + 1);
                return Ok(outcome);
            }
        }
    }
    Ok(outcome)
}

fn availability_for_files(files: &[taumaru_microvm::DownloadedFile]) -> Availability {
    let mut dispositions = files.iter().map(|file| &file.disposition);
    let Some(first) = dispositions.next() else {
        return Availability::Downloaded;
    };
    if !dispositions.all(|disposition| disposition == first) {
        return Availability::Mixed;
    }
    match first {
        taumaru_microvm::DownloadDisposition::Downloaded => Availability::Downloaded,
        taumaru_microvm::DownloadDisposition::AdoptedExisting => Availability::Adopted,
        taumaru_microvm::DownloadDisposition::SkippedExisting => Availability::AlreadyAvailable,
    }
}

fn append_cancelled_after_runtime(plan: &DownloadPlan, outcome: &mut DownloadOutcome) {
    for member in plan.members.iter().skip(1) {
        match member {
            PlanMember::Kernel { kernel_id, .. } => outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("kernel/{kernel_id}"),
            }),
            PlanMember::DistributionImages {
                distribution_id, ..
            } => outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("distribution/{distribution_id}"),
            }),
            PlanMember::RuntimeBinary { .. } => {}
        }
    }
}

fn append_cancelled_after_kernel(
    plan: &DownloadPlan,
    outcome: &mut DownloadOutcome,
    kernel_start: usize,
) {
    let kernel_ids = plan
        .members
        .iter()
        .filter_map(|member| match member {
            PlanMember::Kernel { kernel_id, .. } => Some(kernel_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    for kernel_id in kernel_ids.iter().skip(kernel_start) {
        outcome.groups.push(MemberOutcome::Cancelled {
            label: format!("kernel/{kernel_id}"),
        });
    }
    for selection in &plan.selections {
        outcome.groups.push(MemberOutcome::Cancelled {
            label: format!("distribution/{}", selection.distribution.id),
        });
    }
}

fn append_cancelled_after_distribution(
    plan: &DownloadPlan,
    outcome: &mut DownloadOutcome,
    distribution_start: usize,
) {
    for selection in plan.selections.iter().skip(distribution_start) {
        outcome.groups.push(MemberOutcome::Cancelled {
            label: format!("distribution/{}", selection.distribution.id),
        });
    }
}

#[derive(Clone, Debug)]
struct SelectionOption {
    id: String,
    label: String,
}

impl fmt::Display for SelectionOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

async fn load_catalog<C: ArtifactClient>(client: &C) -> Result<RegistryCatalog, CliError> {
    let host_architecture = host_architecture()?;
    let (kernels, binaries, distributions) = tokio::try_join!(
        client.list_kernels(),
        client.list_binaries(),
        client.list_distributions(),
    )?;
    RegistryCatalog::new(kernels, binaries, distributions, host_architecture)
}

fn build_explicit_plan(
    catalog: &RegistryCatalog,
    arguments: &DownloadArgs,
) -> Result<DownloadPlan, CliError> {
    let mappings = parse_kernel_mappings(&arguments.kernels)?;
    build_plan(catalog, &arguments.distributions, &mappings)
}

fn prompt_render_config(color: bool) -> RenderConfig<'static> {
    let mut config = if color {
        RenderConfig::default_colored()
    } else {
        RenderConfig::empty()
    };
    let prompt_prefix = if color {
        Styled::new("›").with_fg(Color::LightBlue)
    } else {
        Styled::new("›")
    };
    let answered_prompt_prefix = if color {
        Styled::new("✓").with_fg(Color::LightGreen)
    } else {
        Styled::new("✓")
    };
    let highlighted_option_prefix = if color {
        Styled::new("›").with_fg(Color::LightBlue)
    } else {
        Styled::new("›")
    };
    let selected_checkbox = if color {
        Styled::new("●").with_fg(Color::LightGreen)
    } else {
        Styled::new("●")
    };
    let unselected_checkbox = if color {
        Styled::new("○").with_fg(Color::DarkGrey)
    } else {
        Styled::new("○")
    };
    config.prompt = if color {
        StyleSheet::new().with_fg(Color::White)
    } else {
        StyleSheet::empty()
    };
    config.help_message = if color {
        StyleSheet::new().with_fg(Color::DarkGrey)
    } else {
        StyleSheet::empty()
    };
    config
        .with_prompt_prefix(prompt_prefix)
        .with_answered_prompt_prefix(answered_prompt_prefix)
        .with_highlighted_option_prefix(highlighted_option_prefix)
        .with_selected_checkbox(selected_checkbox)
        .with_unselected_checkbox(unselected_checkbox)
}

fn prompt_selections(
    catalog: &RegistryCatalog,
    capabilities: crate::context::TerminalCapabilities,
) -> Result<(Vec<String>, HashMap<String, String>), CliError> {
    let render_config = prompt_render_config(capabilities.color);
    let distribution_options = catalog
        .compatible_distributions()
        .into_iter()
        .map(|distribution| SelectionOption {
            id: distribution.id.clone(),
            label: format!("{} ({})", distribution.display_name, distribution.id),
        })
        .collect::<Vec<_>>();
    let selected = MultiSelect::new("Choose distributions to prepare", distribution_options)
        .with_help_message("↑↓ move  ·  space select  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(render_config)
        .prompt()
        .map_err(prompt_error)?;
    if selected.is_empty() {
        return Err(CliError::Validation(
            "at least one distribution must be selected".to_owned(),
        ));
    }
    let selected_ids = selected
        .iter()
        .map(|option| option.id.clone())
        .collect::<Vec<_>>();
    let mut mappings = HashMap::new();
    for distribution_id in &selected_ids {
        let distribution = catalog.distribution(distribution_id).ok_or_else(|| {
            CliError::Validation(format!(
                "distribution {distribution_id} is unavailable for this host"
            ))
        })?;
        let kernel_options = catalog
            .compatible_kernels(distribution)?
            .into_iter()
            .map(|kernel| SelectionOption {
                id: kernel.id.clone(),
                label: if kernel.id == distribution.default_kernel {
                    format!("{} ({}) [default]", kernel.display_name, kernel.id)
                } else {
                    format!("{} ({})", kernel.display_name, kernel.id)
                },
            })
            .collect::<Vec<_>>();
        let prompt = format!("Kernel for {}", distribution.display_name);
        let selected_kernel = Select::new(&prompt, kernel_options)
            .with_help_message("↑↓ move  ·  enter select")
            .with_render_config(prompt_render_config(capabilities.color))
            .prompt()
            .map_err(prompt_error)?;
        mappings.insert(distribution_id.clone(), selected_kernel.id);
    }
    Ok((selected_ids, mappings))
}

fn prompt_error(error: InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::Prompt("cancelled".to_owned())
    } else {
        CliError::Prompt(message)
    }
}

pub(crate) async fn run(context: &CliContext, arguments: DownloadArgs) -> Result<u8, CliError> {
    let explicit = arguments.non_interactive
        || !arguments.distributions.is_empty()
        || !arguments.kernels.is_empty();
    if !explicit && !context.terminal.interactive {
        return Err(CliError::Validation(
            "an interactive terminal is required when selections are omitted; use --non-interactive with --distribution and --kernel".to_owned(),
        ));
    }
    if explicit && arguments.non_interactive && arguments.distributions.is_empty() {
        return Err(CliError::Validation(
            "--non-interactive requires at least one --distribution".to_owned(),
        ));
    }

    let client = SdkArtifactClient::new(&context.sdk);
    let catalog_spinner = crate::output::human::CatalogSpinner::new(context.terminal);
    let catalog_result = load_catalog(&client).await;
    catalog_spinner.finish();
    let catalog = catalog_result?;
    let (distribution_count, kernel_count, binary_count) = catalog.counts();
    crate::output::human::write_catalog_ready(
        context.terminal,
        distribution_count,
        kernel_count,
        binary_count,
    )
    .map_err(CliError::from)?;
    let plan = if explicit {
        build_explicit_plan(&catalog, &arguments)?
    } else {
        let (selected_ids, mappings) = prompt_selections(&catalog, context.terminal)?;
        build_plan(&catalog, &selected_ids, &mappings)?
    };

    if !explicit {
        crate::output::human::write_review(&plan, context.terminal).map_err(CliError::from)?;
        let confirmed = Confirm::new("Start download?")
            .with_default(false)
            .with_help_message("Enter starts the transfer  ·  Ctrl-C cancels")
            .with_render_config(prompt_render_config(context.terminal.color))
            .prompt()
            .map_err(prompt_error)?;
        if !confirmed {
            return Err(CliError::Prompt("cancelled".to_owned()));
        }
    }

    let cancellation = DownloadCancellation::new();
    let mut renderer =
        crate::output::human::ProgressRenderer::new(plan.expected_bytes, context.terminal);
    let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut renderer, || {
        tokio::signal::ctrl_c()
    })
    .await?;
    renderer.finish();
    crate::output::human::write_summary(&outcome, context.terminal).map_err(CliError::from)?;
    Ok(outcome.exit_code())
}

#[cfg(test)]
mod tests {
    use std::future::{Future, pending};
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use taumaru_microvm::{
        Architecture, ArtifactKind, BinaryFile, BinaryPackage, BootConfiguration, Distribution,
        DistributionImage, DistributionRequirements, DownloadDisposition, DownloadPhase,
        DownloadedBinary, DownloadedDistribution, DownloadedFile, DownloadedKernel,
        FilesystemMetadata, Kernel,
    };

    use crate::output::ProgressSink;

    use super::{
        ArtifactClient, PlanMember, RegistryCatalog, build_plan, execute_plan_with_signals,
        parse_kernel_mappings,
    };

    #[derive(Default)]
    struct NoopSink;

    impl ProgressSink for NoopSink {
        fn on_progress(&mut self, _progress: super::DownloadProgressView) {}
    }

    #[derive(Clone)]
    struct RecordingClient {
        calls: Arc<Mutex<Vec<String>>>,
        failure: Option<String>,
        block_kernel: bool,
    }

    impl RecordingClient {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                failure: None,
                block_kernel: false,
            }
        }

        fn with_failure(call: &str) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                failure: Some(call.to_owned()),
                block_kernel: false,
            }
        }

        fn with_blocking_kernel() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                failure: None,
                block_kernel: true,
            }
        }

        fn record(&self, call: String) {
            self.calls.lock().expect("call log lock").push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("call log lock").clone()
        }

        fn fails(&self, call: &str) -> bool {
            self.failure.as_deref() == Some(call)
        }
    }

    impl ArtifactClient for RecordingClient {
        async fn list_kernels(&self) -> Result<Vec<Kernel>, taumaru_microvm::SdkError> {
            Err(taumaru_microvm::SdkError::Migration(
                "catalog is not used by this fake".to_owned(),
            ))
        }

        async fn list_binaries(&self) -> Result<Vec<BinaryPackage>, taumaru_microvm::SdkError> {
            Err(taumaru_microvm::SdkError::Migration(
                "catalog is not used by this fake".to_owned(),
            ))
        }

        async fn list_distributions(&self) -> Result<Vec<Distribution>, taumaru_microvm::SdkError> {
            Err(taumaru_microvm::SdkError::Migration(
                "catalog is not used by this fake".to_owned(),
            ))
        }

        async fn download_kernel<F>(
            &self,
            kernel_id: &str,
            _cancellation: &taumaru_microvm::DownloadCancellation,
            _on_progress: F,
        ) -> Result<DownloadedKernel, taumaru_microvm::SdkError>
        where
            F: FnMut(taumaru_microvm::DownloadProgress) + Send,
        {
            let call = format!("kernel:{kernel_id}");
            self.record(call.clone());
            if self.fails(&call) {
                return Err(taumaru_microvm::SdkError::Migration(
                    "fake kernel failure".to_owned(),
                ));
            }
            if self.block_kernel {
                loop {
                    if _cancellation.is_cancelled() {
                        return Err(taumaru_microvm::SdkError::Cancelled);
                    }
                    tokio::task::yield_now().await;
                }
            }
            Ok(DownloadedKernel {
                kernel: kernel(kernel_id, Architecture::X86_64, "6.2.0", 13),
                file: downloaded_file(ArtifactKind::Kernel, kernel_id, None),
            })
        }

        async fn download_binary<F>(
            &self,
            binary_id: &str,
            _cancellation: &taumaru_microvm::DownloadCancellation,
            _on_progress: F,
        ) -> Result<DownloadedBinary, taumaru_microvm::SdkError>
        where
            F: FnMut(taumaru_microvm::DownloadProgress) + Send,
        {
            let call = format!("binary:{binary_id}");
            self.record(call.clone());
            if self.fails(&call) {
                return Err(taumaru_microvm::SdkError::Migration(
                    "fake runtime failure".to_owned(),
                ));
            }
            Ok(DownloadedBinary {
                binary: binary(
                    binary_id,
                    "2.0.0",
                    vec![binary_file("firecracker", 19), binary_file("firectl", 23)],
                ),
                files: vec![
                    downloaded_file(ArtifactKind::Binary, binary_id, Some("firecracker")),
                    downloaded_file(ArtifactKind::Binary, binary_id, Some("firectl")),
                ],
            })
        }

        async fn download_distribution<F>(
            &self,
            distribution_id: &str,
            _cancellation: &taumaru_microvm::DownloadCancellation,
            _on_progress: F,
        ) -> Result<DownloadedDistribution, taumaru_microvm::SdkError>
        where
            F: FnMut(taumaru_microvm::DownloadProgress) + Send,
        {
            let call = format!("distribution:{distribution_id}");
            self.record(call.clone());
            if self.fails(&call) {
                return Err(taumaru_microvm::SdkError::Migration(
                    "fake distribution failure".to_owned(),
                ));
            }
            Ok(DownloadedDistribution {
                distribution: distribution(
                    distribution_id,
                    Architecture::X86_64,
                    "kernel-a",
                    vec!["kernel-a"],
                    vec![image("image-1", 1), image("image-2", 2)],
                ),
                images: vec![
                    downloaded_file(
                        ArtifactKind::DistributionImage,
                        distribution_id,
                        Some("image-1"),
                    ),
                    downloaded_file(
                        ArtifactKind::DistributionImage,
                        distribution_id,
                        Some("image-2"),
                    ),
                ],
            })
        }
    }

    fn downloaded_file(
        artifact_kind: ArtifactKind,
        artifact_id: &str,
        member_name: Option<&str>,
    ) -> DownloadedFile {
        DownloadedFile {
            artifact_kind,
            artifact_id: artifact_id.to_owned(),
            member_name: member_name.map(str::to_owned),
            absolute_path: PathBuf::from("/tmp/fake"),
            relative_path: PathBuf::from("fake"),
            size_bytes: 1,
            sha256: "0".repeat(64),
            disposition: DownloadDisposition::Downloaded,
        }
    }

    fn kernel(id: &str, architecture: Architecture, version: &str, size_bytes: u64) -> Kernel {
        Kernel {
            id: id.to_owned(),
            name: id.to_owned(),
            display_name: id.to_owned(),
            version: version.to_owned(),
            architecture,
            path: format!("kernels/{id}/vmlinux"),
            url: format!("https://example.invalid/{id}"),
            filename: "vmlinux".to_owned(),
            size_bytes,
            sha256: "0".repeat(64),
            format: "elf".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            elf: None,
            modified_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    fn binary_file(name: &str, size_bytes: u64) -> BinaryFile {
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

    fn binary(id: &str, version: &str, files: Vec<BinaryFile>) -> BinaryPackage {
        BinaryPackage {
            id: id.to_owned(),
            name: "firecracker".to_owned(),
            display_name: id.to_owned(),
            description: None,
            version: version.to_owned(),
            architecture: Architecture::X86_64,
            files,
        }
    }

    fn image(id: &str, size_bytes: u64) -> DistributionImage {
        DistributionImage {
            id: id.to_owned(),
            name: id.to_owned(),
            display_name: id.to_owned(),
            description: id.to_owned(),
            variant: "minimal".to_owned(),
            path: format!("images/{id}"),
            url: format!("https://example.invalid/{id}"),
            filename: format!("{id}.ext4"),
            format: "ext4".to_owned(),
            filesystem: FilesystemMetadata {
                filesystem_type: "ext4".to_owned(),
                uuid: format!("uuid-{id}"),
                block_size: 4096,
                block_count: 1,
                free_blocks: 0,
                inode_count: 1,
                free_inodes: 0,
                features: Vec::new(),
            },
            size_bytes,
            sha256: "0".repeat(64),
            capabilities: Vec::new(),
            mime_type: "application/octet-stream".to_owned(),
            modified_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    fn distribution(
        id: &str,
        architecture: Architecture,
        default_kernel: &str,
        supported_kernels: Vec<&str>,
        images: Vec<DistributionImage>,
    ) -> Distribution {
        Distribution {
            id: id.to_owned(),
            name: id.to_owned(),
            display_name: id.to_owned(),
            description: id.to_owned(),
            distribution: id.to_owned(),
            version: "1.0".to_owned(),
            codename: "test".to_owned(),
            architecture,
            vendor: "Taumaru".to_owned(),
            homepage: "https://example.invalid".to_owned(),
            default_kernel: default_kernel.to_owned(),
            supported_kernels: supported_kernels.into_iter().map(str::to_owned).collect(),
            boot: BootConfiguration {
                root_device: "/dev/vda".to_owned(),
                kernel_args: Vec::new(),
            },
            requirements: DistributionRequirements {
                min_memory_mb: 128,
                min_vcpus: 1,
            },
            images,
        }
    }

    fn catalog() -> RegistryCatalog {
        RegistryCatalog::new(
            vec![
                kernel("kernel-z", Architecture::X86_64, "6.1.0", 11),
                kernel("kernel-a", Architecture::X86_64, "6.2.0", 13),
                kernel("kernel-arm", Architecture::Aarch64, "6.2.0", 17),
            ],
            vec![
                binary(
                    "runtime-old",
                    "1.0.0",
                    vec![binary_file("firecracker", 5), binary_file("firectl", 7)],
                ),
                binary(
                    "runtime-new",
                    "2.0.0",
                    vec![binary_file("firecracker", 19), binary_file("firectl", 23)],
                ),
                binary(
                    "runtime-incomplete",
                    "9.0.0",
                    vec![binary_file("firecracker", 29)],
                ),
            ],
            vec![
                distribution(
                    "distro-z",
                    Architecture::X86_64,
                    "kernel-z",
                    vec!["kernel-z", "kernel-a"],
                    vec![image("image-z", 31)],
                ),
                distribution(
                    "distro-a",
                    Architecture::X86_64,
                    "kernel-a",
                    vec!["kernel-a", "kernel-z"],
                    vec![image("image-a", 37), image("image-a-debug", 41)],
                ),
                distribution(
                    "distro-arm",
                    Architecture::Aarch64,
                    "kernel-arm",
                    vec!["kernel-arm"],
                    vec![image("image-arm", 43)],
                ),
            ],
            Architecture::X86_64,
        )
        .expect("test catalog should be valid")
    }

    #[test]
    fn catalog_filters_distributions_and_kernels_to_the_host_architecture() {
        let catalog = catalog();
        let distributions = catalog.compatible_distributions();

        assert_eq!(
            distributions
                .iter()
                .map(|distribution| distribution.id.as_str())
                .collect::<Vec<_>>(),
            vec!["distro-a", "distro-z"]
        );
        assert_eq!(
            catalog
                .compatible_kernels(distributions[0])
                .expect("compatible kernels should resolve")
                .iter()
                .map(|kernel| kernel.id.as_str())
                .collect::<Vec<_>>(),
            vec!["kernel-a", "kernel-z"]
        );
    }

    #[test]
    fn catalog_rejects_empty_and_duplicate_registry_identifiers() {
        let empty = RegistryCatalog::new(
            vec![kernel("", Architecture::X86_64, "1.0.0", 1)],
            Vec::new(),
            Vec::new(),
            Architecture::X86_64,
        );
        assert!(empty.is_err());

        let duplicate = RegistryCatalog::new(
            vec![
                kernel("same", Architecture::X86_64, "1.0.0", 1),
                kernel("same", Architecture::X86_64, "1.0.0", 1),
            ],
            Vec::new(),
            Vec::new(),
            Architecture::X86_64,
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn explicit_kernel_mappings_are_parsed_and_duplicate_mappings_are_rejected() {
        let values = vec![
            "distro-a=kernel-a".to_owned(),
            "distro-z=kernel-z".to_owned(),
        ];
        let mappings = parse_kernel_mappings(&values).expect("mappings should parse");
        assert_eq!(
            mappings.get("distro-a").map(String::as_str),
            Some("kernel-a")
        );
        assert!(
            parse_kernel_mappings(&[
                "distro-a=kernel-a".to_owned(),
                "distro-a=kernel-z".to_owned()
            ])
            .is_err()
        );
        assert!(parse_kernel_mappings(&["distro-a".to_owned()]).is_err());
        assert!(parse_kernel_mappings(&["=kernel-a".to_owned()]).is_err());
    }

    #[test]
    fn plan_requires_exactly_one_compatible_kernel_mapping_per_distribution() {
        let catalog = catalog();
        assert!(build_plan(&catalog, &["distro-a".to_owned()], &Default::default()).is_err());
        assert!(
            build_plan(
                &catalog,
                &["distro-a".to_owned()],
                &parse_kernel_mappings(&["distro-z=kernel-z".to_owned()]).expect("valid mapping"),
            )
            .is_err()
        );
        assert!(
            build_plan(
                &catalog,
                &["distro-a".to_owned()],
                &parse_kernel_mappings(&["distro-a=kernel-arm".to_owned()]).expect("valid mapping"),
            )
            .is_err()
        );
        assert!(
            build_plan(
                &catalog,
                &["distro-a".to_owned()],
                &parse_kernel_mappings(&["distro-a=missing-kernel".to_owned()])
                    .expect("valid mapping"),
            )
            .is_err()
        );
    }

    #[test]
    fn plan_selects_highest_runtime_deduplicates_kernels_and_orders_members() {
        let catalog = catalog();
        let mappings = parse_kernel_mappings(&[
            "distro-z=kernel-a".to_owned(),
            "distro-a=kernel-a".to_owned(),
        ])
        .expect("valid mappings");
        let plan = build_plan(
            &catalog,
            &["distro-z".to_owned(), "distro-a".to_owned()],
            &mappings,
        )
        .expect("plan should be valid");

        assert_eq!(plan.runtime.package.id, "runtime-new");
        assert_eq!(plan.runtime.expected_bytes, 42);
        assert_eq!(plan.expected_bytes, 42 + 13 + 31 + 37 + 41);
        assert_eq!(
            plan.members,
            vec![
                PlanMember::RuntimeBinary {
                    package_id: "runtime-new".to_owned(),
                    expected_bytes: 42,
                },
                PlanMember::Kernel {
                    kernel_id: "kernel-a".to_owned(),
                    expected_bytes: 13,
                },
                PlanMember::DistributionImages {
                    distribution_id: "distro-a".to_owned(),
                    image_count: 2,
                    expected_bytes: 78,
                },
                PlanMember::DistributionImages {
                    distribution_id: "distro-z".to_owned(),
                    image_count: 1,
                    expected_bytes: 31,
                },
            ]
        );
    }

    #[test]
    fn explicit_and_interactive_selection_inputs_produce_the_same_plan() {
        let catalog = catalog();
        let mappings = parse_kernel_mappings(&[
            "distro-z=kernel-z".to_owned(),
            "distro-a=kernel-a".to_owned(),
        ])
        .expect("valid mappings");
        let interactive_plan = build_plan(
            &catalog,
            &["distro-a".to_owned(), "distro-z".to_owned()],
            &mappings,
        )
        .expect("interactive plan should be valid");
        let explicit_plan = build_plan(
            &catalog,
            &["distro-z".to_owned(), "distro-a".to_owned()],
            &mappings,
        )
        .expect("explicit plan should be valid");

        assert_eq!(interactive_plan.members, explicit_plan.members);
        assert_eq!(
            interactive_plan
                .selections
                .iter()
                .map(|selection| (&selection.distribution.id, &selection.kernel.id))
                .collect::<Vec<_>>(),
            explicit_plan
                .selections
                .iter()
                .map(|selection| (&selection.distribution.id, &selection.kernel.id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn plan_review_is_deterministic_for_reversed_explicit_input() {
        let catalog = catalog();
        let mappings = parse_kernel_mappings(&[
            "distro-z=kernel-z".to_owned(),
            "distro-a=kernel-a".to_owned(),
        ])
        .expect("valid mappings");
        let first = build_plan(
            &catalog,
            &["distro-z".to_owned(), "distro-a".to_owned()],
            &mappings,
        )
        .expect("plan should be valid");
        let second = build_plan(
            &catalog,
            &["distro-a".to_owned(), "distro-z".to_owned()],
            &mappings,
        )
        .expect("plan should be valid");
        let capabilities = crate::context::TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        };

        assert_eq!(
            crate::output::human::format_review(&first, capabilities),
            crate::output::human::format_review(&second, capabilities)
        );
    }

    #[tokio::test]
    async fn executor_downloads_runtime_then_unique_kernels_then_distributions() {
        let catalog = catalog();
        let mappings = parse_kernel_mappings(&[
            "distro-z=kernel-a".to_owned(),
            "distro-a=kernel-a".to_owned(),
        ])
        .expect("valid mappings");
        let plan = build_plan(
            &catalog,
            &["distro-z".to_owned(), "distro-a".to_owned()],
            &mappings,
        )
        .expect("plan should be valid");
        let client = RecordingClient::new();
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;

        let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut sink, || {
            pending::<Result<(), std::io::Error>>()
        })
        .await
        .expect("executor should complete");

        assert_eq!(
            client.calls(),
            vec![
                "binary:runtime-new",
                "kernel:kernel-a",
                "distribution:distro-a",
                "distribution:distro-z",
            ]
        );
        assert!(outcome.is_success());
        assert_eq!(outcome.verified.len(), 4);
    }

    #[tokio::test]
    async fn executor_waits_for_every_image_group_before_reporting_success() {
        let catalog = catalog();
        let mappings =
            parse_kernel_mappings(&["distro-a=kernel-a".to_owned()]).expect("valid mappings");
        let plan = build_plan(&catalog, &["distro-a".to_owned()], &mappings)
            .expect("plan should be valid");
        let client = RecordingClient::new();
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;

        let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut sink, || {
            pending::<Result<(), std::io::Error>>()
        })
        .await
        .expect("executor should complete");

        assert!(outcome.is_success());
        assert_eq!(
            client.calls(),
            vec![
                "binary:runtime-new",
                "kernel:kernel-a",
                "distribution:distro-a",
            ]
        );
        assert_eq!(
            outcome
                .verified
                .iter()
                .filter(|artifact| matches!(
                    artifact,
                    super::VerifiedArtifact::Distribution(result)
                        if result.images.len() == 2
                ))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn runtime_failure_stops_all_dependent_downloads() {
        let catalog = catalog();
        let mappings =
            parse_kernel_mappings(&["distro-a=kernel-a".to_owned()]).expect("valid mappings");
        let plan = build_plan(&catalog, &["distro-a".to_owned()], &mappings)
            .expect("plan should be valid");
        let client = RecordingClient::with_failure("binary:runtime-new");
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;

        let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut sink, || {
            pending::<Result<(), std::io::Error>>()
        })
        .await
        .expect("executor should return an outcome");

        assert_eq!(client.calls(), vec!["binary:runtime-new"]);
        assert!(!outcome.is_success());
        assert_eq!(outcome.exit_code(), 1);
    }

    #[tokio::test]
    async fn failed_kernel_skips_only_its_distributions_and_continues_unrelated_groups() {
        let catalog = catalog();
        let mappings = parse_kernel_mappings(&[
            "distro-a=kernel-a".to_owned(),
            "distro-z=kernel-z".to_owned(),
        ])
        .expect("valid mappings");
        let plan = build_plan(
            &catalog,
            &["distro-a".to_owned(), "distro-z".to_owned()],
            &mappings,
        )
        .expect("plan should be valid");
        let client = RecordingClient::with_failure("kernel:kernel-a");
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;

        let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut sink, || {
            pending::<Result<(), std::io::Error>>()
        })
        .await
        .expect("executor should return an outcome");

        assert_eq!(
            client.calls(),
            vec![
                "binary:runtime-new",
                "kernel:kernel-a",
                "kernel:kernel-z",
                "distribution:distro-z",
            ]
        );
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Skipped { label, .. }
                if label == "distribution/distro-a"
        )));
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Verified { label, .. }
                if label == "distribution/distro-z"
        )));
        assert!(!outcome.is_success());
        assert_eq!(outcome.exit_code(), 1);
    }

    #[tokio::test]
    async fn distribution_failure_retains_verified_runtime_and_kernel_results() {
        let catalog = catalog();
        let mappings =
            parse_kernel_mappings(&["distro-a=kernel-a".to_owned()]).expect("valid mappings");
        let plan = build_plan(&catalog, &["distro-a".to_owned()], &mappings)
            .expect("plan should be valid");
        let client = RecordingClient::with_failure("distribution:distro-a");
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;

        let outcome = execute_plan_with_signals(&client, &plan, &cancellation, &mut sink, || {
            pending::<Result<(), std::io::Error>>()
        })
        .await
        .expect("executor should return an outcome");

        assert_eq!(outcome.verified.len(), 2);
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Failed { label, .. }
                if label == "distribution/distro-a"
        )));
        let summary = crate::output::human::format_summary(&outcome);
        assert!(summary.contains("Failed"));
        assert!(summary.contains("Why:"));
        assert!(summary.contains("Next:"));
        assert_eq!(outcome.exit_code(), 1);
    }

    #[tokio::test]
    async fn signal_cancellation_waits_for_the_current_group_and_starts_no_later_group() {
        let catalog = catalog();
        let mappings =
            parse_kernel_mappings(&["distro-a=kernel-a".to_owned()]).expect("valid mappings");
        let plan = build_plan(&catalog, &["distro-a".to_owned()], &mappings)
            .expect("plan should be valid");
        let client = RecordingClient::with_blocking_kernel();
        let cancellation = taumaru_microvm::DownloadCancellation::new();
        let mut sink = NoopSink;
        let mut signal_count = 0_u8;

        let outcome = execute_plan_with_signals(
            &client,
            &plan,
            &cancellation,
            &mut sink,
            || -> Pin<Box<dyn Future<Output = Result<(), std::io::Error>> + Send>> {
                signal_count = signal_count.saturating_add(1);
                if signal_count == 2 {
                    Box::pin(std::future::ready(Ok(())))
                } else {
                    Box::pin(pending::<Result<(), std::io::Error>>())
                }
            },
        )
        .await
        .expect("executor should return cancellation outcome");

        assert_eq!(
            client.calls(),
            vec!["binary:runtime-new", "kernel:kernel-a"]
        );
        assert_eq!(outcome.exit_code(), 130);
        assert!(outcome.cancelled);
        assert_eq!(outcome.verified.len(), 1);
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Cancelled { label }
                if label == "kernel/kernel-a"
        )));
        assert!(
            !client
                .calls()
                .iter()
                .any(|call| call.starts_with("distribution:"))
        );
    }

    #[test]
    fn cache_dispositions_are_exposed_without_a_cli_cache_decision() {
        let mut downloaded = downloaded_file(ArtifactKind::Kernel, "kernel", None);
        assert_eq!(
            super::availability_for_files(std::slice::from_ref(&downloaded)),
            super::Availability::Downloaded
        );
        downloaded.disposition = DownloadDisposition::AdoptedExisting;
        assert_eq!(
            super::availability_for_files(std::slice::from_ref(&downloaded)),
            super::Availability::Adopted
        );
        downloaded.disposition = DownloadDisposition::SkippedExisting;
        assert_eq!(
            super::availability_for_files(std::slice::from_ref(&downloaded)),
            super::Availability::AlreadyAvailable
        );
    }

    #[test]
    fn progress_normalization_preserves_sdk_bytes_and_adds_only_completed_plan_bytes() {
        let progress = taumaru_microvm::DownloadProgress {
            artifact_kind: ArtifactKind::DistributionImage,
            artifact_id: "distro-a".to_owned(),
            member_name: Some("image-a".to_owned()),
            bytes_received: 20,
            total_bytes: 40,
            aggregate_bytes_received: 20,
            aggregate_total_bytes: 80,
            phase: DownloadPhase::Downloading,
        };

        let view = super::normalize_progress(&progress, 100, 1_000).expect("valid progress");
        assert_eq!(view.current_bytes, 20);
        assert_eq!(view.expected_bytes, 40);
        assert_eq!(view.completed_plan_bytes, 100);
        assert_eq!(view.aggregate_current_bytes, 120);
        assert_eq!(view.aggregate_expected_bytes, 180);
        assert_eq!(view.plan_total_bytes, 1_000);
        assert_eq!(view.phase, DownloadPhase::Downloading);
    }
}
