use inquire::ui::{Color, RenderConfig, StyleSheet, Styled};
use inquire::{Confirm, InquireError, MultiSelect};
use semver::Version;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::io;
use taumaru_microvm::{
    Architecture, BinaryPackage, Distribution, DistributionImage, DownloadCancellation,
    DownloadProgress, DownloadedBinary, DownloadedDistributionImage, DownloadedKernel, Kernel,
    MicroVmSdk, SdkError,
};

use crate::cli::DownloadArgs;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::ProgressSink;

pub(crate) trait ArtifactClient {
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

    async fn download_distribution_image<F>(
        &self,
        distribution_id: &str,
        image_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedDistributionImage, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;
}

pub(crate) struct SdkArtifactClient<'a> {
    sdk: &'a MicroVmSdk,
}

impl<'a> SdkArtifactClient<'a> {
    pub(crate) fn new(sdk: &'a MicroVmSdk) -> Self {
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

    async fn download_distribution_image<F>(
        &self,
        distribution_id: &str,
        image_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: F,
    ) -> Result<DownloadedDistributionImage, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        self.sdk
            .download_distribution_image_with_cancellation(
                distribution_id,
                image_id,
                cancellation,
                on_progress,
            )
            .await
    }
}

pub(crate) struct ProgressForwarder<'a, S: ProgressSink> {
    sink: &'a mut S,
    completed_plan_bytes: u64,
    plan_total_bytes: u64,
    mapping_error: Option<CliError>,
}

impl<'a, S: ProgressSink> ProgressForwarder<'a, S> {
    pub(crate) fn new(sink: &'a mut S, completed_plan_bytes: u64, plan_total_bytes: u64) -> Self {
        Self {
            sink,
            completed_plan_bytes,
            plan_total_bytes,
            mapping_error: None,
        }
    }

    pub(crate) fn forward(&mut self, progress: DownloadProgress) {
        if self.mapping_error.is_some() {
            return;
        }
        match normalize_progress(&progress, self.completed_plan_bytes, self.plan_total_bytes) {
            Ok(view) => self.sink.on_progress(view),
            Err(error) => self.mapping_error = Some(error),
        }
    }

    pub(crate) fn take_error(&mut self) -> Option<CliError> {
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

pub(crate) fn normalize_progress(
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
pub(crate) struct ImageSelection {
    pub(crate) distribution: Distribution,
    pub(crate) image: DistributionImage,
    pub(crate) kernel: Kernel,
    pub(crate) expected_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeBinarySelection {
    pub(crate) packages: Vec<BinaryPackage>,
    pub(crate) file_count: usize,
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
    DistributionImage {
        distribution_id: String,
        image_id: String,
        expected_bytes: u64,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadPlan {
    pub(crate) runtime: RuntimeBinarySelection,
    pub(crate) selections: Vec<ImageSelection>,
    pub(crate) members: Vec<PlanMember>,
    pub(crate) expected_bytes: u64,
}

impl RegistryCatalog {
    pub(crate) fn new(
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

    pub(crate) fn compatible_distributions(&self) -> Vec<&Distribution> {
        self.distributions
            .iter()
            .filter(|distribution| {
                same_architecture(&distribution.architecture, &self.host_architecture)
            })
            .collect()
    }

    pub(crate) fn compatible_images(&self) -> Vec<(&Distribution, &DistributionImage)> {
        let mut pairs = Vec::new();
        for distribution in self.compatible_distributions() {
            for image in &distribution.images {
                pairs.push((distribution, image));
            }
        }
        pairs.sort_by(|left, right| {
            (left.0.id.as_str(), left.1.id.as_str())
                .cmp(&(right.0.id.as_str(), right.1.id.as_str()))
        });
        pairs
    }

    pub(crate) fn default_kernel(&self, distribution: &Distribution) -> Result<Kernel, CliError> {
        if !same_architecture(&distribution.architecture, &self.host_architecture) {
            return Err(CliError::Validation(format!(
                "distribution {} is not published for host architecture {}",
                distribution.id,
                architecture_label(&distribution.architecture)
            )));
        }
        let kernel = self
            .kernels
            .iter()
            .find(|candidate| candidate.id == distribution.default_kernel)
            .ok_or_else(|| {
                CliError::Validation(format!(
                    "distribution {} default kernel {} is unavailable for this host",
                    distribution.id, distribution.default_kernel
                ))
            })?;
        if !same_architecture(&kernel.architecture, &self.host_architecture)
            || !same_architecture(&kernel.architecture, &distribution.architecture)
        {
            return Err(CliError::Validation(format!(
                "distribution {} default kernel {} is not compatible with host architecture {}",
                distribution.id,
                kernel.id,
                architecture_label(&self.host_architecture)
            )));
        }
        Ok(kernel.clone())
    }

    pub(crate) fn select_runtime_packages(&self) -> Result<RuntimeBinarySelection, CliError> {
        let mut candidates = self
            .binaries
            .iter()
            .filter(|package| same_architecture(&package.architecture, &self.host_architecture))
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

        let required_components = ["firecracker", "firectl"];
        let mut packages = Vec::new();
        let mut selected_ids = HashSet::new();
        for required_component in required_components {
            let package = candidates
                .iter()
                .find(|(_, package)| has_runtime_component(package, required_component))
                .map(|(_, package)| *package)
                .ok_or_else(|| {
                    CliError::Validation(format!(
                        "no valid runtime package provides {required_component} for host architecture {}",
                        architecture_label(&self.host_architecture)
                    ))
                })?;
            if selected_ids.insert(package.id.as_str()) {
                packages.push((*package).clone());
            }
        }
        packages.sort_by(|left, right| left.id.cmp(&right.id));

        let mut file_count = 0_usize;
        let mut expected_bytes = 0_u64;
        for package in &packages {
            file_count = file_count.checked_add(package.files.len()).ok_or_else(|| {
                CliError::Validation("runtime file count exceeded the supported range".to_owned())
            })?;
            expected_bytes = checked_size_add(
                expected_bytes,
                binary_package_size(package)?,
                "runtime package",
            )?;
        }
        Ok(RuntimeBinarySelection {
            packages,
            file_count,
            expected_bytes,
        })
    }

    pub(crate) fn distribution(&self, id: &str) -> Option<&Distribution> {
        self.distributions
            .iter()
            .find(|distribution| distribution.id == id)
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

pub(crate) fn same_architecture(left: &Architecture, right: &Architecture) -> bool {
    matches!(
        (left, right),
        (Architecture::X86_64, Architecture::X86_64)
            | (Architecture::Aarch64, Architecture::Aarch64)
            | (Architecture::Arm, Architecture::Arm)
            | (Architecture::Riscv64, Architecture::Riscv64)
            | (Architecture::X86, Architecture::X86)
    )
}

pub(crate) fn architecture_label(architecture: &Architecture) -> &'static str {
    match architecture {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::Riscv64 => "riscv64",
        Architecture::X86 => "x86",
    }
}

pub(crate) fn host_architecture() -> Result<Architecture, CliError> {
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

pub(crate) fn has_runtime_component(package: &BinaryPackage, required_component: &str) -> bool {
    package
        .files
        .iter()
        .any(|file| file.name == required_component)
}

pub(crate) fn binary_package_size(package: &BinaryPackage) -> Result<u64, CliError> {
    package.files.iter().try_fold(0_u64, |total, file| {
        total.checked_add(file.size_bytes).ok_or_else(|| {
            CliError::Validation(format!(
                "runtime package {} exceeds the supported size range",
                package.id
            ))
        })
    })
}

pub(crate) fn parse_image_selections(values: &[String]) -> Result<Vec<(String, String)>, CliError> {
    let mut selections = Vec::with_capacity(values.len());
    for value in values {
        let parts = value.split('=').collect::<Vec<_>>();
        if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
            return Err(CliError::Validation(format!(
                "image selection {value:?} must use DISTRIBUTION_ID=IMAGE_ID"
            )));
        }
        selections.push((parts[0].to_owned(), parts[1].to_owned()));
    }
    Ok(selections)
}

pub(crate) fn build_plan(
    catalog: &RegistryCatalog,
    selected_images: &[(String, String)],
) -> Result<DownloadPlan, CliError> {
    if selected_images.is_empty() {
        return Err(CliError::Validation(
            "at least one image must be selected".to_owned(),
        ));
    }

    let mut pairs = selected_images.to_vec();
    pairs.sort();
    pairs.dedup();
    let mut selections = Vec::with_capacity(pairs.len());
    for (distribution_id, image_id) in pairs {
        let distribution = catalog.distribution(&distribution_id).ok_or_else(|| {
            CliError::Validation(format!(
                "distribution {distribution_id} is unavailable for this host"
            ))
        })?;
        if !same_architecture(&distribution.architecture, &catalog.host_architecture) {
            return Err(CliError::Validation(format!(
                "distribution {distribution_id} is not published for host architecture {}",
                architecture_label(&distribution.architecture)
            )));
        }
        let image = distribution
            .images
            .iter()
            .find(|candidate| candidate.id == image_id)
            .ok_or_else(|| {
                CliError::Validation(format!(
                    "image {image_id} is not published by distribution {distribution_id}"
                ))
            })?;
        let kernel = catalog.default_kernel(distribution)?;
        selections.push(ImageSelection {
            distribution: distribution.clone(),
            image: image.clone(),
            kernel,
            expected_bytes: image.size_bytes,
        });
    }

    let runtime = catalog.select_runtime_packages()?;
    let mut unique_kernels = BTreeMap::new();
    for selection in &selections {
        unique_kernels
            .entry(selection.kernel.id.clone())
            .or_insert_with(|| selection.kernel.clone());
    }

    let mut members = Vec::with_capacity(
        runtime
            .packages
            .len()
            .saturating_add(unique_kernels.len())
            .saturating_add(selections.len()),
    );
    let mut expected_bytes = 0_u64;
    for package in &runtime.packages {
        let package_bytes = binary_package_size(package)?;
        expected_bytes = checked_size_add(expected_bytes, package_bytes, "runtime package")?;
        members.push(PlanMember::RuntimeBinary {
            package_id: package.id.clone(),
            expected_bytes: package_bytes,
        });
    }
    for kernel in unique_kernels.values() {
        expected_bytes = checked_size_add(expected_bytes, kernel.size_bytes, "kernel")?;
        members.push(PlanMember::Kernel {
            kernel_id: kernel.id.clone(),
            expected_bytes: kernel.size_bytes,
        });
    }
    for selection in &selections {
        expected_bytes = checked_size_add(
            expected_bytes,
            selection.expected_bytes,
            "distribution image",
        )?;
        members.push(PlanMember::DistributionImage {
            distribution_id: selection.distribution.id.clone(),
            image_id: selection.image.id.clone(),
            expected_bytes: selection.expected_bytes,
        });
    }
    Ok(DownloadPlan {
        runtime,
        selections,
        members,
        expected_bytes,
    })
}

pub(crate) fn checked_size_add(total: u64, value: u64, kind: &str) -> Result<u64, CliError> {
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
    DistributionImage(Box<DownloadedDistributionImage>),
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

pub(crate) enum OperationResult<T> {
    Finished(Result<T, SdkError>),
    Cancelled,
}

pub(crate) async fn call_with_signal<T, O, Sig>(
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
        append_cancelled_after_runtime(plan, &mut outcome, 0);
        return Ok(outcome);
    }

    for (runtime_index, package) in plan.runtime.packages.iter().enumerate() {
        if cancellation.is_cancelled() {
            outcome.cancelled = true;
            append_cancelled_after_runtime(plan, &mut outcome, runtime_index);
            return Ok(outcome);
        }
        let runtime_id = package.id.clone();
        let runtime_bytes = binary_package_size(package)?;
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
                outcome.add_available_bytes(runtime_bytes)?;
                completed_plan_bytes =
                    checked_size_add(completed_plan_bytes, runtime_bytes, "completed runtime")?;
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
                append_cancelled_after_runtime(plan, &mut outcome, runtime_index + 1);
                return Ok(outcome);
            }
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
        let distribution_id = selection.distribution.id.clone();
        let image_id = selection.image.id.clone();
        let label = format!("distribution/{distribution_id}/{image_id}");
        if failed_kernels.contains(&selection.kernel.id) {
            outcome.groups.push(MemberOutcome::Skipped {
                label,
                reason: format!("kernel/{} failed", selection.kernel.id),
            });
            continue;
        }
        if cancellation.is_cancelled() {
            outcome.cancelled = true;
            outcome.groups.push(MemberOutcome::Cancelled { label });
            append_cancelled_after_image(plan, &mut outcome, index + 1);
            return Ok(outcome);
        }
        let mut image_progress =
            ProgressForwarder::new(sink, completed_plan_bytes, plan.expected_bytes);
        let image_call = client.download_distribution_image(
            &distribution_id,
            &image_id,
            cancellation,
            |progress| image_progress.forward(progress),
        );
        let image_result = call_with_signal(image_call, cancellation, signal_factory()).await?;
        if let Some(error) = image_progress.take_error() {
            return Err(error);
        }
        match image_result {
            OperationResult::Finished(Ok(result)) => {
                outcome.add_available_bytes(selection.expected_bytes)?;
                completed_plan_bytes = checked_size_add(
                    completed_plan_bytes,
                    selection.expected_bytes,
                    "completed image",
                )?;
                outcome.groups.push(MemberOutcome::Verified {
                    label,
                    availability: availability_for_files(std::slice::from_ref(&result.file)),
                });
                outcome
                    .verified
                    .push(VerifiedArtifact::DistributionImage(Box::new(result)));
            }
            OperationResult::Finished(Err(error)) => {
                outcome.groups.push(MemberOutcome::Failed {
                    label,
                    reason: error.to_string(),
                });
            }
            OperationResult::Cancelled => {
                outcome.cancelled = true;
                outcome.groups.push(MemberOutcome::Cancelled { label });
                append_cancelled_after_image(plan, &mut outcome, index + 1);
                return Ok(outcome);
            }
        }
    }
    Ok(outcome)
}

pub(crate) fn availability_for_files(files: &[taumaru_microvm::DownloadedFile]) -> Availability {
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

fn append_cancelled_after_runtime(
    plan: &DownloadPlan,
    outcome: &mut DownloadOutcome,
    runtime_start: usize,
) {
    for member in plan.members.iter().skip(runtime_start) {
        match member {
            PlanMember::RuntimeBinary { package_id, .. } => {
                outcome.groups.push(MemberOutcome::Cancelled {
                    label: format!("runtime/{package_id}"),
                })
            }
            PlanMember::Kernel { kernel_id, .. } => outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("kernel/{kernel_id}"),
            }),
            PlanMember::DistributionImage {
                distribution_id,
                image_id,
                ..
            } => outcome.groups.push(MemberOutcome::Cancelled {
                label: format!("distribution/{distribution_id}/{image_id}"),
            }),
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
    append_cancelled_after_image(plan, outcome, 0);
}

fn append_cancelled_after_image(
    plan: &DownloadPlan,
    outcome: &mut DownloadOutcome,
    image_start: usize,
) {
    for selection in plan.selections.iter().skip(image_start) {
        outcome.groups.push(MemberOutcome::Cancelled {
            label: format!(
                "distribution/{}/{}",
                selection.distribution.id, selection.image.id
            ),
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

pub(crate) async fn load_catalog<C: ArtifactClient>(
    client: &C,
) -> Result<RegistryCatalog, CliError> {
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
    let selections = parse_image_selections(&arguments.images)?;
    build_plan(catalog, &selections)
}

pub(crate) fn prompt_render_config(color: bool) -> RenderConfig<'static> {
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
) -> Result<Vec<(String, String)>, CliError> {
    let render_config = prompt_render_config(capabilities.color);
    let image_options = catalog
        .compatible_images()
        .into_iter()
        .map(|(distribution, image)| {
            let capabilities_text = if image.capabilities.is_empty() {
                String::new()
            } else {
                format!(" · {}", image.capabilities.join(", "))
            };
            SelectionOption {
                id: format!("{}={}", distribution.id, image.id),
                label: format!(
                    "{} / {} ({} · {}{})",
                    distribution.id,
                    image.display_name,
                    image.variant,
                    format_image_bytes(image.size_bytes),
                    capabilities_text
                ),
            }
        })
        .collect::<Vec<_>>();
    if image_options.is_empty() {
        return Err(CliError::Validation(
            "no host-compatible images are available for this host".to_owned(),
        ));
    }
    let selected = MultiSelect::new("Choose images to prepare", image_options)
        .with_help_message("↑↓ move  ·  space select  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(render_config)
        .prompt()
        .map_err(prompt_error)?;
    if selected.is_empty() {
        return Err(CliError::Validation(
            "at least one image must be selected".to_owned(),
        ));
    }
    parse_image_selections(
        &selected
            .iter()
            .map(|option| option.id.clone())
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn format_image_bytes(size_bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = size_bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size_bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn prompt_error(error: InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::Prompt("cancelled".to_owned())
    } else {
        CliError::Prompt(message)
    }
}

pub(crate) fn escalated_child_command(images: &[String], non_interactive: bool) -> Vec<OsString> {
    let mut command = vec![OsString::from("artifacts"), OsString::from("download")];
    if non_interactive {
        command.push(OsString::from("--non-interactive"));
    }
    for image in images {
        command.push(OsString::from("--image"));
        command.push(OsString::from(image));
    }
    command
}

pub(crate) async fn run(context: &CliContext, arguments: DownloadArgs) -> Result<u8, CliError> {
    let explicit = arguments.non_interactive || !arguments.images.is_empty();
    if !explicit && !context.terminal.interactive {
        return Err(CliError::Validation(
            "an interactive terminal is required when selections are omitted; use --non-interactive with --image DISTRIBUTION_ID=IMAGE_ID".to_owned(),
        ));
    }
    if explicit && arguments.non_interactive && arguments.images.is_empty() {
        return Err(CliError::Validation(
            "--non-interactive requires at least one --image DISTRIBUTION_ID=IMAGE_ID".to_owned(),
        ));
    }
    if arguments.non_interactive
        && let Some(exit) = crate::privilege::require_privileged(
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
        let selections = prompt_selections(&catalog, context.terminal)?;
        build_plan(&catalog, &selections)?
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

    if !arguments.non_interactive {
        let selected_images: Vec<String> = plan
            .selections
            .iter()
            .map(|selection| format!("{}={}", selection.distribution.id, selection.image.id))
            .collect();
        let home = crate::context::resolve_home().ok();
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            false,
            home,
            &[],
            escalated_child_command(&selected_images, true),
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
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
        DownloadedBinary, DownloadedDistributionImage, DownloadedFile, DownloadedKernel,
        FilesystemMetadata, Kernel,
    };

    use crate::output::ProgressSink;

    use super::{
        ArtifactClient, PlanMember, RegistryCatalog, build_explicit_plan, build_plan,
        execute_plan_with_signals, parse_image_selections,
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

        async fn download_distribution_image<F>(
            &self,
            distribution_id: &str,
            image_id: &str,
            _cancellation: &taumaru_microvm::DownloadCancellation,
            _on_progress: F,
        ) -> Result<DownloadedDistributionImage, taumaru_microvm::SdkError>
        where
            F: FnMut(taumaru_microvm::DownloadProgress) + Send,
        {
            let call = format!("image:{distribution_id}/{image_id}");
            self.record(call.clone());
            if self.fails(&call) {
                return Err(taumaru_microvm::SdkError::Migration(
                    "fake image failure".to_owned(),
                ));
            }
            Ok(DownloadedDistributionImage {
                distribution: distribution(
                    distribution_id,
                    Architecture::X86_64,
                    "kernel-a",
                    vec!["kernel-a"],
                    vec![image(image_id, 1)],
                ),
                image: image(image_id, 1),
                file: downloaded_file(
                    ArtifactKind::DistributionImage,
                    distribution_id,
                    Some(image_id),
                ),
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
        binary_with_name(id, "firecracker", version, files)
    }

    fn binary_with_name(
        id: &str,
        name: &str,
        version: &str,
        files: Vec<BinaryFile>,
    ) -> BinaryPackage {
        BinaryPackage {
            id: id.to_owned(),
            name: name.to_owned(),
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
                    vec![binary_file("jailer", 29)],
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

    fn split_runtime_catalog() -> RegistryCatalog {
        let mut catalog = catalog();
        catalog.binaries = vec![
            binary(
                "firecracker-1.17.0-x86_64",
                "1.17.0",
                vec![binary_file("firecracker", 19), binary_file("jailer", 23)],
            ),
            binary_with_name(
                "firectl-0.2.0-x86_64",
                "firectl",
                "0.2.0",
                vec![binary_file("firectl", 17)],
            ),
        ];
        catalog
    }

    #[test]
    fn catalog_flattens_images_sorted_by_distribution_then_image() {
        let catalog = catalog();
        let pairs = catalog.compatible_images();

        assert_eq!(
            pairs
                .iter()
                .map(|(distribution, image)| (distribution.id.as_str(), image.id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("distro-a", "image-a"),
                ("distro-a", "image-a-debug"),
                ("distro-z", "image-z"),
            ]
        );
        assert!(
            catalog
                .default_kernel(catalog.distribution("distro-a").expect("distro-a exists"))
                .expect("default kernel resolves")
                .id
                == "kernel-a"
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
    fn image_selections_require_scoped_pairs() {
        let selections =
            parse_image_selections(&["distro-a=image-a".to_owned(), "distro-z=image-z".to_owned()])
                .expect("scoped selections should parse");
        assert_eq!(
            selections,
            vec![
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-z".to_owned(), "image-z".to_owned()),
            ]
        );
        assert!(parse_image_selections(&["image-a".to_owned()]).is_err());
        assert!(parse_image_selections(&["=image-a".to_owned()]).is_err());
        assert!(parse_image_selections(&["distro-a=".to_owned()]).is_err());
    }

    #[test]
    fn plan_rejects_unknown_mismatched_and_incompatible_images() {
        let catalog = catalog();
        assert!(build_plan(&catalog, &[],).is_err());
        assert!(
            build_plan(
                &catalog,
                &[("distro-a".to_owned(), "missing-image".to_owned())],
            )
            .is_err()
        );
        assert!(build_plan(&catalog, &[("distro-a".to_owned(), "image-arm".to_owned())],).is_err());
        assert!(build_plan(&catalog, &[("no-distro".to_owned(), "image-a".to_owned())],).is_err());
        assert!(
            build_plan(
                &catalog,
                &[("distro-arm".to_owned(), "image-arm".to_owned())],
            )
            .is_err()
        );
    }

    #[test]
    fn plan_deduplicates_repeats_resolves_defaults_and_orders_members() {
        let catalog = catalog();
        let plan = build_plan(
            &catalog,
            &[
                ("distro-z".to_owned(), "image-z".to_owned()),
                ("distro-a".to_owned(), "image-a-debug".to_owned()),
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-a".to_owned(), "image-a".to_owned()),
            ],
        )
        .expect("plan should be valid");

        assert_eq!(plan.runtime.packages[0].id, "runtime-new");
        assert_eq!(plan.runtime.file_count, 2);
        assert_eq!(plan.runtime.expected_bytes, 42);
        assert_eq!(
            plan.selections
                .iter()
                .map(|selection| (
                    selection.distribution.id.as_str(),
                    selection.image.id.as_str(),
                    selection.kernel.id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("distro-a", "image-a", "kernel-a"),
                ("distro-a", "image-a-debug", "kernel-a"),
                ("distro-z", "image-z", "kernel-z"),
            ]
        );
        assert_eq!(plan.expected_bytes, 42 + 13 + 11 + 37 + 41 + 31);
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
                PlanMember::Kernel {
                    kernel_id: "kernel-z".to_owned(),
                    expected_bytes: 11,
                },
                PlanMember::DistributionImage {
                    distribution_id: "distro-a".to_owned(),
                    image_id: "image-a".to_owned(),
                    expected_bytes: 37,
                },
                PlanMember::DistributionImage {
                    distribution_id: "distro-a".to_owned(),
                    image_id: "image-a-debug".to_owned(),
                    expected_bytes: 41,
                },
                PlanMember::DistributionImage {
                    distribution_id: "distro-z".to_owned(),
                    image_id: "image-z".to_owned(),
                    expected_bytes: 31,
                },
            ]
        );
    }

    #[test]
    fn explicit_image_plan_matches_prompted_selection_order() {
        let catalog = catalog();
        let prompted = build_plan(
            &catalog,
            &[
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-z".to_owned(), "image-z".to_owned()),
            ],
        )
        .expect("prompted plan should be valid");
        let explicit = build_explicit_plan(
            &catalog,
            &crate::cli::DownloadArgs {
                images: vec!["distro-z=image-z".to_owned(), "distro-a=image-a".to_owned()],
                non_interactive: true,
            },
        )
        .expect("explicit plan should be valid");

        assert_eq!(prompted.members, explicit.members);
        assert_eq!(
            prompted
                .selections
                .iter()
                .map(|selection| (&selection.distribution.id, &selection.image.id))
                .collect::<Vec<_>>(),
            explicit
                .selections
                .iter()
                .map(|selection| (&selection.distribution.id, &selection.image.id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn plan_review_is_deterministic_for_reversed_explicit_input() {
        let catalog = catalog();
        let first = build_plan(
            &catalog,
            &[
                ("distro-z".to_owned(), "image-z".to_owned()),
                ("distro-a".to_owned(), "image-a".to_owned()),
            ],
        )
        .expect("plan should be valid");
        let second = build_plan(
            &catalog,
            &[
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-z".to_owned(), "image-z".to_owned()),
            ],
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
    async fn executor_downloads_runtime_then_unique_kernels_then_images() {
        let catalog = catalog();
        let plan = build_plan(
            &catalog,
            &[
                ("distro-z".to_owned(), "image-z".to_owned()),
                ("distro-a".to_owned(), "image-a-debug".to_owned()),
                ("distro-a".to_owned(), "image-a".to_owned()),
            ],
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
                "kernel:kernel-z",
                "image:distro-a/image-a",
                "image:distro-a/image-a-debug",
                "image:distro-z/image-z",
            ]
        );
        assert!(outcome.is_success());
        assert_eq!(outcome.verified.len(), 6);
    }

    #[tokio::test]
    async fn executor_downloads_split_runtime_packages_before_kernels() {
        let catalog = split_runtime_catalog();
        let plan = build_plan(&catalog, &[("distro-a".to_owned(), "image-a".to_owned())])
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
                "binary:firecracker-1.17.0-x86_64",
                "binary:firectl-0.2.0-x86_64",
                "kernel:kernel-a",
                "image:distro-a/image-a",
            ]
        );
        assert!(outcome.is_success());
        assert_eq!(outcome.verified.len(), 4);
    }

    #[tokio::test]
    async fn split_runtime_package_failure_stops_kernel_and_distribution_downloads() {
        let catalog = split_runtime_catalog();
        let plan = build_plan(&catalog, &[("distro-a".to_owned(), "image-a".to_owned())])
            .expect("plan should be valid");
        let client = RecordingClient::with_failure("binary:firectl-0.2.0-x86_64");
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
                "binary:firecracker-1.17.0-x86_64",
                "binary:firectl-0.2.0-x86_64",
            ]
        );
        assert_eq!(outcome.verified.len(), 1);
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn executor_downloads_every_selected_image_before_reporting_success() {
        let catalog = catalog();
        let plan = build_plan(
            &catalog,
            &[
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-a".to_owned(), "image-a-debug".to_owned()),
            ],
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

        assert!(outcome.is_success());
        assert_eq!(
            client.calls(),
            vec![
                "binary:runtime-new",
                "kernel:kernel-a",
                "image:distro-a/image-a",
                "image:distro-a/image-a-debug",
            ]
        );
        assert_eq!(
            outcome
                .verified
                .iter()
                .filter(|artifact| matches!(
                    artifact,
                    super::VerifiedArtifact::DistributionImage(_)
                ))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn runtime_failure_stops_all_dependent_downloads() {
        let catalog = catalog();
        let plan = build_plan(&catalog, &[("distro-a".to_owned(), "image-a".to_owned())])
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
    async fn failed_kernel_skips_only_its_images_and_continues_unrelated_groups() {
        let catalog = catalog();
        let plan = build_plan(
            &catalog,
            &[
                ("distro-a".to_owned(), "image-a".to_owned()),
                ("distro-a".to_owned(), "image-a-debug".to_owned()),
                ("distro-z".to_owned(), "image-z".to_owned()),
            ],
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
                "image:distro-z/image-z",
            ]
        );
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Skipped { label, .. }
                if label == "distribution/distro-a/image-a"
        )));
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Skipped { label, .. }
                if label == "distribution/distro-a/image-a-debug"
        )));
        assert!(outcome.groups.iter().any(|group| matches!(
            group,
            super::MemberOutcome::Verified { label, .. }
                if label == "distribution/distro-z/image-z"
        )));
        assert!(!outcome.is_success());
        assert_eq!(outcome.exit_code(), 1);
    }

    #[tokio::test]
    async fn image_failure_retains_verified_runtime_and_kernel_results() {
        let catalog = catalog();
        let plan = build_plan(&catalog, &[("distro-a".to_owned(), "image-a".to_owned())])
            .expect("plan should be valid");
        let client = RecordingClient::with_failure("image:distro-a/image-a");
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
                if label == "distribution/distro-a/image-a"
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
        let plan = build_plan(&catalog, &[("distro-a".to_owned(), "image-a".to_owned())])
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
        assert!(!client.calls().iter().any(|call| call.starts_with("image:")));
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

    #[test]
    fn escalated_child_replays_image_selection_non_interactively() {
        let command = super::escalated_child_command(
            &["distro-a=image-a".to_owned(), "distro-a=image-b".to_owned()],
            true,
        );
        let rendered: Vec<String> = command
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            [
                "artifacts",
                "download",
                "--non-interactive",
                "--image",
                "distro-a=image-a",
                "--image",
                "distro-a=image-b"
            ]
        );
    }
}
