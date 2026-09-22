use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::fs as async_fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::adapters::credentials::ed25519::Ed25519CredentialStore;
use crate::adapters::network::linux::LinuxNetworkController;
use crate::adapters::persistence::sqlite::SqliteRepository;
use crate::adapters::registry::taumaru::TaumaruRegistryClient;
use crate::adapters::runtime::firecracker::FirecrackerRuntime;
use crate::adapters::storage::ext4::Ext4Storage;
use crate::domain::artifact::{
    ArtifactKind, DownloadCancellation, DownloadDisposition, DownloadPhase, DownloadProgress,
    DownloadSpec, DownloadedBinary, DownloadedDistribution, DownloadedDistributionImage,
    DownloadedFile, DownloadedKernel, FileIntegrity, InstalledBinary, ProgressTracker,
    PruneFailure, PrunePreview, PruneSummary, PrunedImageId, is_valid_sha256,
    validate_registry_path,
};
use crate::domain::config::minimum_memory_bytes;
use crate::domain::lifecycle::{MicroVmState, NetworkMode};
use crate::domain::microvm::{
    CreateMicroVmRequest, CreationEventPhase, CreationOutcome, CreationProgress, CreationStage,
    MicroVmCreationResult, MicroVmDeleteResult, MicroVmRecord, MicroVmStartResult,
    MicroVmStopResult, MicroVmSummary, NetworkConfigurationResult, PersistedCredential,
    PersistedNetwork, PersistedRuntime, RunningMicroVm, SshConnectionInfo, TOTAL_CREATION_STEPS,
    unspecified_address,
};
use crate::domain::registry::{
    Architecture, BinaryFile, BinaryPackage, Distribution, DistributionImage, Kernel,
    TaumaruRegistry,
};
use crate::error::SdkError;
use crate::ports::artifacts::ArtifactSource;
use crate::ports::credentials::CredentialStore;
use crate::ports::network::{NetworkController, NetworkRequest};
use crate::ports::repository::{InventoryState, LocalRepository, StoredMicroVm};
use crate::ports::runtime::{RuntimeController, StartRequest};
use crate::ports::storage::GuestStorage;
use semver::Version;

const DEFAULT_REGISTRY_BASE_URL: &str = "https://artifacts.taumaru.com/v1/";
const PROBE_TIMEOUT_SECS: u64 = 12;
/// Exit wait after a delivered graceful shutdown request before SIGKILL escalation.
const STOP_EXIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Re-verification wait after SIGKILL before reporting the forced outcome.
const STOP_KILL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheDecision {
    Replace,
    Adopt,
    Skip,
}

/// Internal outcome of guarding, locking, and deleting one prune file.
/// `Skipped` means the per-target lock is held by an in-progress transfer, so
/// the candidate is recorded in the skipped list untouched. `Absent` means no
/// file exists, so the caller drops the stale row at zero bytes. `Failure`
/// carries an English reason with the row kept. `Removed` carries the
/// filesystem-observed size for byte accounting.
enum PruneFileOutcome {
    /// The candidate is actively transferring; leave it untouched.
    Skipped,
    /// No file exists; drop the stale row at zero bytes.
    Absent,
    /// The candidate could not be reclaimed; the row is kept.
    Failure(String),
    /// The regular file was deleted; accounts these bytes.
    Removed {
        /// Filesystem-observed size at deletion time.
        freed_bytes: u64,
    },
}

async fn delete_prune_file(
    sdk: &MicroVmSdk,
    absolute_path: &Path,
    artifact_key: &str,
) -> PruneFileOutcome {
    if !path_is_below_home(&sdk.home, absolute_path) {
        return PruneFileOutcome::Failure(format!(
            "the recorded path for {artifact_key} escapes the SDK home"
        ));
    }
    let lock = match sdk.target_lock(absolute_path) {
        Ok(lock) => lock,
        Err(error) => return PruneFileOutcome::Failure(error.to_string()),
    };
    let _guard = match lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => return PruneFileOutcome::Skipped,
    };
    let metadata = match async_fs::symlink_metadata(absolute_path).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return PruneFileOutcome::Absent;
        }
        Err(source) => {
            return PruneFileOutcome::Failure(
                SdkError::filesystem("inspect prune candidate", absolute_path, source).to_string(),
            );
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return PruneFileOutcome::Failure(format!(
            "the recorded path for {artifact_key} is not a regular file"
        ));
    }
    let freed_bytes = metadata.len();
    if let Err(source) = async_fs::remove_file(absolute_path).await {
        return PruneFileOutcome::Failure(
            SdkError::filesystem("remove unused artifact", absolute_path, source).to_string(),
        );
    }
    PruneFileOutcome::Removed { freed_bytes }
}

fn target_is_reusable_file(file_type: std::fs::FileType, is_file: bool) -> bool {
    !file_type.is_symlink() && is_file
}

pub(crate) fn decide_cache(
    spec: &DownloadSpec,
    integrity: Option<&FileIntegrity>,
    inventory_state: InventoryState,
) -> CacheDecision {
    // A `Complete` inventory row already pins size and digest from a previous
    // verified transfer, so reuse trusts it without re-hashing the file. Hashing
    // happens only for Adopt (unrecorded file) and Replace (fresh transfer).
    if inventory_state == InventoryState::Complete {
        return CacheDecision::Skip;
    }
    let physical_file_is_correct = integrity.is_some_and(|value| {
        value.size_bytes == spec.expected_size && value.sha256 == spec.expected_sha256
    });
    match (physical_file_is_correct, inventory_state) {
        (true, InventoryState::Missing | InventoryState::Incomplete) => CacheDecision::Adopt,
        (false, _) => CacheDecision::Replace,
        (true, InventoryState::Complete) => CacheDecision::Skip,
    }
}

#[derive(Clone, Debug)]
enum LogicalMember {
    Kernel {
        kernel: Kernel,
    },
    Binary {
        package: BinaryPackage,
        file: BinaryFile,
    },
    DistributionImage {
        distribution: Distribution,
        image: Box<DistributionImage>,
        kernels: Vec<Kernel>,
    },
}

#[derive(Clone, Debug)]
struct DownloadMember {
    spec: DownloadSpec,
    logical: LogicalMember,
}

/// Host-local SDK client for registry artifacts and inventory operations.
pub struct MicroVmSdk {
    pub(crate) home: PathBuf,
    pub(crate) repository: Arc<dyn LocalRepository>,
    pub(crate) registry: Arc<dyn ArtifactSource>,
    storage: Arc<dyn GuestStorage>,
    credentials: Arc<dyn CredentialStore>,
    network: Arc<dyn NetworkController>,
    runtime: Arc<dyn RuntimeController>,
    target_locks: Arc<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
}

#[derive(Clone, Debug)]
struct CreationPrerequisites {
    image_path: PathBuf,
    kernel: Kernel,
    firecracker: InstalledBinary,
    firectl: InstalledBinary,
}

struct CreationJournal {
    vm_id: Option<i64>,
    volume_path: PathBuf,
    volume_created: bool,
    rootfs_path: Option<PathBuf>,
    ssh_directory: Option<PathBuf>,
    private_key_path: Option<PathBuf>,
    public_key_path: Option<PathBuf>,
    network: Option<PersistedNetwork>,
}
fn emit_creation_progress<F>(observer: &mut Option<&mut F>, event: CreationProgress)
where
    F: FnMut(CreationProgress) + Send,
{
    if let Some(observe) = observer.as_deref_mut() {
        observe(event);
    }
}

/// Overall percent for an event: completed stages contribute their full share and
/// byte-moving work contributes its fraction of the current stage's share.
fn overall_percent(completed_steps: u64, stage_bytes: Option<(u64, u64)>) -> u64 {
    let base = completed_steps.saturating_mul(100) / TOTAL_CREATION_STEPS;
    let Some((done, total)) = stage_bytes else {
        return base.min(100);
    };
    if total == 0 {
        return base.min(100);
    }
    let clamped = done.min(total);
    let share = clamped.saturating_mul(100) / (total * TOTAL_CREATION_STEPS);
    base.saturating_add(share).min(100)
}

fn creation_started_event(stage: CreationStage, completed_steps: u64) -> CreationProgress {
    CreationProgress {
        stage,
        completed_steps,
        total_steps: TOTAL_CREATION_STEPS,
        overall_percent: overall_percent(completed_steps, None),
        phase: CreationEventPhase::Started,
        bytes_completed: None,
        expected_bytes: None,
        outcome: None,
    }
}

fn creation_tick_event(
    stage: CreationStage,
    completed_steps: u64,
    bytes_completed: u64,
    expected_bytes: u64,
) -> CreationProgress {
    CreationProgress {
        stage,
        completed_steps,
        total_steps: TOTAL_CREATION_STEPS,
        overall_percent: overall_percent(completed_steps, Some((bytes_completed, expected_bytes))),
        phase: CreationEventPhase::InProgress,
        bytes_completed: Some(bytes_completed),
        expected_bytes: Some(expected_bytes),
        outcome: None,
    }
}

fn creation_stage_event(stage: CreationStage, completed_steps: u64) -> CreationProgress {
    CreationProgress {
        stage,
        completed_steps,
        total_steps: TOTAL_CREATION_STEPS,
        overall_percent: overall_percent(completed_steps, None),
        phase: CreationEventPhase::Finished,
        bytes_completed: None,
        expected_bytes: None,
        outcome: None,
    }
}

fn creation_terminal_event(
    stage: CreationStage,
    completed_steps: u64,
    outcome: CreationOutcome,
) -> CreationProgress {
    CreationProgress {
        stage,
        completed_steps,
        total_steps: TOTAL_CREATION_STEPS,
        overall_percent: overall_percent(completed_steps, None),
        phase: CreationEventPhase::Finished,
        bytes_completed: None,
        expected_bytes: None,
        outcome: Some(outcome),
    }
}

impl std::fmt::Debug for MicroVmSdk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MicroVmSdk")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl MicroVmSdk {
    /// Creates an SDK rooted at `home` and uses the canonical Taumaru registry.
    ///
    /// The constructor creates the managed directory layout, opens the local inventory, and
    /// applies all pending migrations before returning.
    pub fn new(home: impl AsRef<Path>) -> Result<Self, SdkError> {
        Self::with_registry_base_url(home, DEFAULT_REGISTRY_BASE_URL)
    }

    /// Creates an SDK rooted at `home` using an explicit registry base URL.
    ///
    /// This constructor is useful for registry mirrors and deterministic local fixture servers.
    /// It never changes the SDK-owned home selection rules.
    pub fn with_registry_base_url(
        home: impl AsRef<Path>,
        registry_base_url: impl AsRef<str>,
    ) -> Result<Self, SdkError> {
        let registry = Arc::new(TaumaruRegistryClient::new(registry_base_url.as_ref())?);
        let normalized_home = normalize_home_path(home.as_ref())?;
        create_managed_directories(&normalized_home)?;
        let database_path = normalized_home.join("state").join("inventory.db");
        let repository = Arc::new(SqliteRepository::initialize(database_path)?);

        Ok(Self {
            home: normalized_home,
            repository,
            registry,
            storage: Arc::new(Ext4Storage),
            credentials: Arc::new(Ed25519CredentialStore),
            network: Arc::new(LinuxNetworkController),
            runtime: Arc::new(FirecrackerRuntime),
            target_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Lists all kernels currently published by the configured registry.
    pub async fn list_kernels(&self) -> Result<Vec<Kernel>, SdkError> {
        Ok(self.fetch_manifest().await?.kernels)
    }

    /// Lists all binary packages currently published by the configured registry.
    pub async fn list_binaries(&self) -> Result<Vec<BinaryPackage>, SdkError> {
        Ok(self.fetch_manifest().await?.binaries)
    }

    /// Lists all distributions currently published by the configured registry.
    pub async fn list_distributions(&self) -> Result<Vec<Distribution>, SdkError> {
        Ok(self.fetch_manifest().await?.distributions)
    }

    /// Downloads one kernel into the SDK home, or adopts/skips a verified cached file.
    ///
    /// The callback receives transfer and terminal events in the caller's task. It is never
    /// used for SDK diagnostics and cannot change the integrity or persistence decisions.
    pub async fn download_kernel<F>(
        &self,
        kernel_id: &str,
        on_progress: F,
    ) -> Result<DownloadedKernel, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let cancellation = DownloadCancellation::new();
        self.download_kernel_with_cancellation(kernel_id, &cancellation, on_progress)
            .await
    }

    /// Downloads one kernel while observing a caller-owned cancellation handle.
    ///
    /// Cancellation is cooperative. The SDK removes the current temporary file before
    /// returning [`SdkError::Cancelled`] and preserves members that were already committed.
    pub async fn download_kernel_with_cancellation<F>(
        &self,
        kernel_id: &str,
        cancellation: &DownloadCancellation,
        mut on_progress: F,
    ) -> Result<DownloadedKernel, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        validate_requested_id(kernel_id, "kernel")?;
        let manifest = self.fetch_manifest().await?;
        let kernel = manifest
            .kernels
            .into_iter()
            .find(|candidate| candidate.id == kernel_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "kernel".to_owned(),
                id: kernel_id.to_owned(),
            })?;
        let member = self.kernel_member(kernel)?;
        let mut tracker = ProgressTracker::new(member.spec.expected_size);
        let file = self
            .download_member(&member, cancellation, &mut tracker, &mut on_progress)
            .await?;
        let LogicalMember::Kernel { kernel } = member.logical else {
            return Err(SdkError::Migration(
                "kernel download resolved to a non-kernel member".to_owned(),
            ));
        };
        Ok(DownloadedKernel { kernel, file })
    }

    /// Downloads every file in one binary package into the SDK home.
    ///
    /// Members are processed independently. A verified member remains reusable if a later
    /// member fails, while the aggregate operation returns the first typed failure.
    pub async fn download_binary<F>(
        &self,
        binary_id: &str,
        on_progress: F,
    ) -> Result<DownloadedBinary, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let cancellation = DownloadCancellation::new();
        self.download_binary_with_cancellation(binary_id, &cancellation, on_progress)
            .await
    }

    /// Downloads every file in a binary package while observing a caller-owned cancellation
    /// handle. Verified members remain available when a later member is cancelled.
    pub async fn download_binary_with_cancellation<F>(
        &self,
        binary_id: &str,
        cancellation: &DownloadCancellation,
        mut on_progress: F,
    ) -> Result<DownloadedBinary, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        validate_requested_id(binary_id, "binary package")?;
        let manifest = self.fetch_manifest().await?;
        let package = manifest
            .binaries
            .into_iter()
            .find(|candidate| candidate.id == binary_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "binary package".to_owned(),
                id: binary_id.to_owned(),
            })?;
        let members = package
            .files
            .iter()
            .cloned()
            .map(|file| self.binary_member(package.clone(), file))
            .collect::<Result<Vec<_>, _>>()?;
        let mut tracker = ProgressTracker::new(total_size(&members, binary_id)?);
        let mut files = Vec::with_capacity(members.len());
        for member in &members {
            files.push(
                self.download_member(member, cancellation, &mut tracker, &mut on_progress)
                    .await?,
            );
        }
        Ok(DownloadedBinary {
            binary: package,
            files,
        })
    }

    /// Downloads every image in one distribution into the SDK home.
    ///
    /// The distribution's boot arguments and kernel compatibility references are persisted as
    /// part of image persistence. The default kernel is referenced but never implicitly fetched.
    pub async fn download_distribution<F>(
        &self,
        distribution_id: &str,
        on_progress: F,
    ) -> Result<DownloadedDistribution, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let cancellation = DownloadCancellation::new();
        self.download_distribution_with_cancellation(distribution_id, &cancellation, on_progress)
            .await
    }

    /// Downloads every image in a distribution while observing a caller-owned cancellation
    /// handle. Verified images remain available when a later image is cancelled.
    pub async fn download_distribution_with_cancellation<F>(
        &self,
        distribution_id: &str,
        cancellation: &DownloadCancellation,
        mut on_progress: F,
    ) -> Result<DownloadedDistribution, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        validate_requested_id(distribution_id, "distribution")?;
        let manifest = self.fetch_manifest().await?;
        let distribution = manifest
            .distributions
            .into_iter()
            .find(|candidate| candidate.id == distribution_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution".to_owned(),
                id: distribution_id.to_owned(),
            })?;
        let kernels = manifest.kernels;
        let members = distribution
            .images
            .iter()
            .cloned()
            .map(|image| {
                self.distribution_image_member(distribution.clone(), image, kernels.clone())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut tracker = ProgressTracker::new(total_size(&members, distribution_id)?);
        let mut images = Vec::with_capacity(members.len());
        for member in &members {
            images.push(
                self.download_member(member, cancellation, &mut tracker, &mut on_progress)
                    .await?,
            );
        }
        Ok(DownloadedDistribution {
            distribution,
            images,
        })
    }

    /// Downloads one image of one distribution into the SDK home.
    ///
    /// The distribution's boot arguments and kernel compatibility references are persisted as
    /// part of image persistence. The default kernel is referenced but never implicitly fetched.
    /// The existing whole-distribution operation is preserved for callers that need every image.
    pub async fn download_distribution_image<F>(
        &self,
        distribution_id: &str,
        image_id: &str,
        on_progress: F,
    ) -> Result<DownloadedDistributionImage, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let cancellation = DownloadCancellation::new();
        self.download_distribution_image_with_cancellation(
            distribution_id,
            image_id,
            &cancellation,
            on_progress,
        )
        .await
    }

    /// Downloads one distribution image while observing a caller-owned cancellation
    /// handle. A cancelled in-flight file is removed before [`SdkError::Cancelled`] is
    /// returned; images committed by earlier calls are preserved.
    pub async fn download_distribution_image_with_cancellation<F>(
        &self,
        distribution_id: &str,
        image_id: &str,
        cancellation: &DownloadCancellation,
        mut on_progress: F,
    ) -> Result<DownloadedDistributionImage, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        validate_requested_id(distribution_id, "distribution")?;
        validate_requested_id(image_id, "distribution image")?;
        let manifest = self.fetch_manifest().await?;
        let distribution = manifest
            .distributions
            .into_iter()
            .find(|candidate| candidate.id == distribution_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution".to_owned(),
                id: distribution_id.to_owned(),
            })?;
        let image = distribution
            .images
            .iter()
            .find(|candidate| candidate.id == image_id)
            .cloned()
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution image".to_owned(),
                id: image_id.to_owned(),
            })?;
        let member =
            self.distribution_image_member(distribution.clone(), image.clone(), manifest.kernels)?;
        let mut tracker = ProgressTracker::new(member.spec.expected_size);
        let file = self
            .download_member(&member, cancellation, &mut tracker, &mut on_progress)
            .await?;
        Ok(DownloadedDistributionImage {
            distribution,
            image,
            file,
        })
    }

    /// Resolves a verified binary from the durable SQLite inventory and rechecks its file.
    pub async fn resolve_binary(
        &self,
        package_id: &str,
        component_name: &str,
    ) -> Result<InstalledBinary, SdkError> {
        validate_requested_id(package_id, "binary package")?;
        validate_requested_id(component_name, "binary component")?;
        let package_id = package_id.to_owned();
        let component_name = component_name.to_owned();
        let binary = self
            .run_repository(move |repository| {
                repository.resolve_binary(&package_id, &component_name)
            })
            .await?;
        if !path_is_below_home(&self.home, &binary.path) {
            return Err(SdkError::StaleBinary {
                package_id: binary.package_id,
                component_name: binary.component_name,
                path: binary.path,
            });
        }
        let Some(integrity) = calculate_file_integrity(&binary.path).await? else {
            return Err(SdkError::StaleBinary {
                package_id: binary.package_id,
                component_name: binary.component_name,
                path: binary.path,
            });
        };
        if integrity.size_bytes != binary.size_bytes || integrity.sha256 != binary.sha256 {
            return Err(SdkError::IntegrityMismatch {
                artifact: format!("binary:{}/{}", binary.package_id, binary.component_name),
                expected_size: binary.size_bytes,
                actual_size: integrity.size_bytes,
                expected_sha256: binary.sha256,
                actual_sha256: integrity.sha256,
            });
        }
        Ok(binary)
    }

    /// Lists locally present distribution images for display markers.
    ///
    /// Returns the `(distribution_id, image_id)` pairs whose inventory relationship
    /// exists with a verified download row. Presence is optimistic: it answers from
    /// the local inventory in one query without registry access, file hashing, or
    /// download. A stale or corrupted file can still be listed here; provisioning
    /// revalidates integrity and repairs it before creation. The query performs no
    /// download, mutation, repair, or persistence write, and emits no output, logs,
    /// or global state.
    pub async fn list_present_distribution_images(
        &self,
    ) -> Result<Vec<(String, String)>, SdkError> {
        self.run_repository(|repository| repository.list_ready_distribution_images())
            .await
    }

    /// Lists every stored MicroVM with its call-time verified state.
    ///
    /// Returns one [`MicroVmSummary`] per inventory row ordered by name. Each entry
    /// carries the live-verified state: `Running` if and only if the VM's
    /// volume-local control socket answers, `Stopped` otherwise. Verification runs
    /// concurrently with bounded parallelism; a probe failure or timeout resolves
    /// that entry to `Stopped` without aborting the listing. The query performs
    /// no mutation, repair, or persistence write, and emits no output, logs, or
    /// global state.
    pub async fn list_microvms(&self) -> Result<Vec<MicroVmSummary>, SdkError> {
        let stored = self
            .run_repository(|repository| repository.list_stored_microvms())
            .await?;
        let states = self.verify_all(stored.iter().collect()).await;
        Ok(stored
            .iter()
            .zip(states)
            .map(|(vm, state)| {
                let network = vm.network.as_ref();
                MicroVmSummary {
                    name: vm.record.name.clone(),
                    state,
                    vcpu_count: vm.record.vcpu_count,
                    memory_bytes: vm.record.memory_bytes,
                    disk_size_bytes: vm.record.disk_size_bytes,
                    distribution_id: vm.record.distribution_id.clone(),
                    image_id: vm.record.image_id.clone(),
                    network_mode: network.map(|network| network.config.mode),
                    guest_address: network.map(|network| network.config.guest_address),
                    lan_address: network.and_then(|network| network.config.lan_address),
                }
            })
            .collect())
    }

    /// Lists every actually-running MicroVM for the SSH selector and named SSH resolution.
    ///
    /// Returns one [`RunningMicroVm`] per running machine ordered by name, each carrying
    /// the stored SSH connection metadata verbatim (paths only, never key contents).
    /// Liveness is verified at call time: the volume-local control socket must
    /// answer. Silent, failed, or timed-out probes are omitted, never errors.
    /// The query performs no filesystem validation of key material, no mutation,
    /// repair, or persistence write, and emits no output, logs, or global state.
    pub async fn list_running_microvms(&self) -> Result<Vec<RunningMicroVm>, SdkError> {
        let stored = self
            .run_repository(|repository| repository.list_stored_microvms())
            .await?;
        let states = self.verify_all(stored.iter().collect()).await;
        let mut running = Vec::new();
        for (vm, state) in stored.iter().zip(states) {
            if state != MicroVmState::Running || !vm.is_complete() {
                continue;
            }
            let (network, credential, _) = vm
                .require_full("list running MicroVMs")
                .expect("running VM is complete");
            running.push(RunningMicroVm {
                name: vm.record.name.clone(),
                ssh: SshConnectionInfo {
                    user: credential.ssh_user.clone(),
                    port: credential.ssh_port,
                    address: network.config.guest_address,
                    private_key_path: credential.private_key_path.clone(),
                    public_key_path: credential.public_key_path.clone(),
                },
            });
        }
        Ok(running)
    }

    /// Reports whether a distribution image is verified locally.
    ///
    /// Returns `true` only when the per-image inventory relationship exists and the
    /// physical file is present below the SDK home with matching size and digest.
    /// Returns `false` for missing, incomplete, or stale entries: the same conditions
    /// that make creation return [`SdkError::ArtifactPrerequisite`]. Unknown
    /// distributions and images outside their named distribution return typed
    /// [`SdkError::NotFound`] errors. The query performs no download, mutation,
    /// repair, or persistence write, and emits no output, logs, or global state.
    pub async fn is_distribution_image_ready(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<bool, SdkError> {
        validate_requested_id(distribution_id, "distribution")?;
        validate_requested_id(image_id, "distribution image")?;
        let manifest = self.fetch_manifest().await?;
        let distribution = manifest
            .distributions
            .iter()
            .find(|candidate| candidate.id == distribution_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution".to_owned(),
                id: distribution_id.to_owned(),
            })?;
        let image = distribution
            .images
            .iter()
            .find(|candidate| candidate.id == image_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution image".to_owned(),
                id: image_id.to_owned(),
            })?;
        let distribution_id = distribution.id.clone();
        let image_id = image.id.clone();
        let expected_size = image.size_bytes;
        let expected_sha256 = image.sha256.clone();
        let local = match self
            .run_repository(move |repository| {
                repository.resolve_distribution_image(&distribution_id, &image_id)
            })
            .await
        {
            Ok(local) => local,
            Err(SdkError::NotFound { .. } | SdkError::ArtifactPrerequisite { .. }) => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if !path_is_below_home(&self.home, &local.path) {
            return Ok(false);
        }
        if local.size_bytes != expected_size || local.sha256 != expected_sha256 {
            return Ok(false);
        }
        let Some(integrity) = calculate_file_integrity(&local.path).await? else {
            return Ok(false);
        };
        Ok(integrity.size_bytes == expected_size && integrity.sha256 == expected_sha256)
    }

    /// Creates and initially configures one stopped MicroVM.
    ///
    /// Creation resolves only artifacts already downloaded into this SDK home. It copies the
    /// selected registry image into a VM-owned ext4 volume, grows the copy when requested,
    /// injects one Ed25519 public key, reconciles the selected network, and records every path
    /// needed for a later start. LAN mode may start the exact selected runtime temporarily to
    /// observe DHCP; the process and socket are stopped before this method returns.
    ///
    /// When `on_progress` is `Some`, the SDK invokes the observer synchronously in the
    /// caller's task with one event per finished stage plus exactly one terminal event.
    /// The observer is caller-owned, infallible, and non-blocking, and cannot change
    /// integrity, persistence, or error decisions. When `None`, no events are emitted
    /// and behavior is identical to an unobserved creation.
    pub async fn create_microvm<F>(
        &self,
        request: CreateMicroVmRequest,
        mut on_progress: Option<F>,
    ) -> Result<MicroVmCreationResult, SdkError>
    where
        F: FnMut(CreationProgress) + Send,
    {
        let mut observer = on_progress.as_mut();
        emit_creation_progress(
            &mut observer,
            creation_started_event(CreationStage::Validation, 0),
        );
        let validated = match request.validate(&self.home) {
            Ok(validated) => validated,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::Failed {
                            stage: CreationStage::Validation,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let volume_path = validated.volume_path.clone();
        let name_lock_path = self.home.join("vms").join(&validated.request.name);
        let name_lock = match self.target_lock(&name_lock_path) {
            Ok(lock) => lock,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::Failed {
                            stage: CreationStage::Validation,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let _name_guard = name_lock.lock().await;
        let volume_lock = if volume_path != name_lock_path {
            match self.target_lock(&volume_path) {
                Ok(lock) => Some(lock),
                Err(error) => {
                    emit_creation_progress(
                        &mut observer,
                        creation_terminal_event(
                            CreationStage::Validation,
                            0,
                            CreationOutcome::Failed {
                                stage: CreationStage::Validation,
                            },
                        ),
                    );
                    return Err(error);
                }
            }
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };

        let existing_name = validated.request.name.clone();
        let existing = match self
            .run_repository(move |repository| repository.find_microvm(&existing_name))
            .await
        {
            Ok(existing) => existing,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::Failed {
                            stage: CreationStage::Validation,
                        },
                    ),
                );
                return Err(error);
            }
        };
        if let Some(existing) = existing {
            return self
                .return_or_reject_existing_creation(&validated, existing, &mut observer)
                .await;
        }
        emit_creation_progress(
            &mut observer,
            creation_stage_event(CreationStage::Validation, 1),
        );
        emit_creation_progress(
            &mut observer,
            creation_started_event(CreationStage::PrerequisiteResolution, 1),
        );
        let prerequisites = match self
            .resolve_creation_prerequisites(&validated.request, &mut observer)
            .await
        {
            Ok(prerequisites) => prerequisites,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::PrerequisiteResolution,
                        1,
                        CreationOutcome::Failed {
                            stage: CreationStage::PrerequisiteResolution,
                        },
                    ),
                );
                return Err(error);
            }
        };
        if let Err(error) = self.runtime.validate_host() {
            emit_creation_progress(
                &mut observer,
                creation_terminal_event(
                    CreationStage::PrerequisiteResolution,
                    1,
                    CreationOutcome::Failed {
                        stage: CreationStage::PrerequisiteResolution,
                    },
                ),
            );
            return Err(error);
        }
        if let Err(error) = self
            .ensure_volume_available(&validated.request.name, &volume_path)
            .await
        {
            emit_creation_progress(
                &mut observer,
                creation_terminal_event(
                    CreationStage::PrerequisiteResolution,
                    1,
                    CreationOutcome::Failed {
                        stage: CreationStage::PrerequisiteResolution,
                    },
                ),
            );
            return Err(error);
        }

        let rootfs_path = volume_path.join("rootfs.ext4");
        let socket_path = volume_path.join("firecracker.sock");
        let now = match unix_timestamp() {
            Ok(now) => now,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::PrerequisiteResolution,
                        1,
                        CreationOutcome::Failed {
                            stage: CreationStage::PrerequisiteResolution,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let mut record = MicroVmRecord {
            id: 0,
            name: validated.request.name.clone(),
            distribution_id: validated.request.distribution_id.clone(),
            image_id: validated.request.image_id.clone(),
            kernel_id: prerequisites.kernel.id.clone(),
            firecracker_package_id: prerequisites.firecracker.package_id.clone(),
            firectl_package_id: prerequisites.firectl.package_id.clone(),
            disk_size_bytes: validated.request.disk_size_bytes,
            memory_bytes: validated.request.memory_bytes,
            memory_effective_mib: validated.memory_effective_mib,
            vcpu_count: validated.request.vcpu_count,
            volume_path: volume_path.clone(),
            rootfs_path,
            socket_path,
            expose_on_lan: validated.request.expose_on_lan,
            created_at: now,
        };
        let insert_record = record.clone();
        let record_name = record.name.clone();
        let record_volume = record.volume_path.clone();
        let vm_id = match self
            .run_repository(move |repository| repository.insert_microvm(&insert_record))
            .await
            .map_err(|error| map_insert_creation_error(error, &record_name, &record_volume))
        {
            Ok(vm_id) => vm_id,
            Err(error) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::PrerequisiteResolution,
                        1,
                        CreationOutcome::Failed {
                            stage: CreationStage::PrerequisiteResolution,
                        },
                    ),
                );
                return Err(error);
            }
        };
        record.id = vm_id;
        emit_creation_progress(
            &mut observer,
            creation_stage_event(CreationStage::PrerequisiteResolution, 2),
        );
        let mut journal = CreationJournal {
            vm_id: Some(vm_id),
            volume_path,
            volume_created: false,
            rootfs_path: None,
            ssh_directory: None,
            private_key_path: None,
            public_key_path: None,
            network: None,
        };

        let result = self
            .create_claimed_microvm(
                &validated,
                &prerequisites,
                &record,
                &mut journal,
                &mut observer,
            )
            .await;
        match result {
            Ok(result) => {
                emit_creation_progress(
                    &mut observer,
                    creation_terminal_event(
                        CreationStage::Finalization,
                        6,
                        CreationOutcome::Completed,
                    ),
                );
                Ok(result)
            }
            Err(primary) => {
                let cleanup_failures = self.rollback_creation(&journal).await;
                if cleanup_failures.is_empty() {
                    Err(primary)
                } else {
                    Err(SdkError::Cleanup {
                        primary: primary.to_string(),
                        failures: cleanup_failures,
                    })
                }
            }
        }
    }

    /// Reconciles one existing VM's host network and returns the resources changed or reused.
    ///
    /// The operation is intentionally independent from creation. Host-only mode is repaired
    /// without starting the guest. LAN mode starts the recorded runtime only when a DHCP lease
    /// is missing, observes the lease, and leaves the VM stopped with an inactive socket.
    pub async fn configure_network(
        &self,
        name: &str,
    ) -> Result<NetworkConfigurationResult, SdkError> {
        crate::domain::config::validate_vm_name(name)?;
        let lookup_name = name.to_owned();
        let name_lock_path = self.home.join("vms").join(name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let _name_guard = name_lock.lock().await;
        let stored = self
            .run_repository(move |repository| repository.find_microvm(&lookup_name))
            .await?
            .ok_or_else(|| SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: name.to_owned(),
            })?;
        let volume_lock = if stored.record.volume_path != name_lock_path {
            Some(self.target_lock(&stored.record.volume_path)?)
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };

        stored.require_complete("configure network")?;
        let persisted_network_ref =
            stored
                .network
                .as_ref()
                .ok_or_else(|| SdkError::LifecycleConflict {
                    name: stored.record.name.clone(),
                    state: "creation incomplete".to_owned(),
                    operation: "configure network".to_owned(),
                })?;
        validate_persisted_files(&stored)?;
        validate_persisted_network(&stored)?;
        let expected_mode = if stored.record.expose_on_lan {
            NetworkMode::Lan
        } else {
            NetworkMode::HostOnly
        };
        if persisted_network_ref.config.mode != expected_mode {
            return Err(SdkError::Network {
                mode: expected_mode.to_string(),
                operation: "validate persisted network mode".to_owned(),
                resource: stored.record.name,
                reason: "the persisted network mode does not match the VM configuration".to_owned(),
            });
        }

        let request = NetworkRequest {
            vm_name: stored.record.name.clone(),
            mode: expected_mode,
            guest_mac: persisted_network_ref.guest_mac.clone(),
            lan_address_override: None,
        };
        let used_addresses = self
            .run_repository(|repository| repository.list_host_only_networks())
            .await?;
        let outcome =
            self.network
                .configure(&request, Some(persisted_network_ref), &used_addresses, &[])?;
        let runtime_record = stored.runtime.clone().unwrap_or_else(|| PersistedRuntime {
            firecracker_path: PathBuf::new(),
            firectl_path: PathBuf::new(),
            socket_path: stored.record.socket_path.clone(),
            process_id: None,
            process_state: "stopped".to_owned(),
        });

        self.runtime.verify_stopped(&stored.record.socket_path)?;
        let vm_id = stored.record.id;
        let persisted_network = outcome.persisted.clone();
        let persisted_runtime = runtime_record.clone();
        self.run_repository(move |repository| {
            repository.update_network(vm_id, &persisted_network)?;
            repository.persist_runtime(vm_id, &persisted_runtime)
        })
        .await?;

        Ok(NetworkConfigurationResult {
            name: stored.record.name,
            configuration: outcome.persisted.config,
            applied: outcome.applied,
            skipped: outcome.skipped,
        })
    }

    /// Starts one configured MicroVM and returns its running identity.
    ///
    /// The caller passes only the VM name. The volume directory, full
    /// configuration, and network identity are read from the inventory
    /// record. Running is decided from live host evidence: both the recorded
    /// machine process and the control socket must agree the VM is running.
    /// Any mismatch is treated as stale and started fresh. Repeated calls
    /// while genuinely running return the current identity without launching
    /// another process. Host network items are repaired in place and stay
    /// repaired when a later launch step fails.
    pub async fn start_microvm(&self, name: &str) -> Result<MicroVmStartResult, SdkError> {
        crate::domain::config::validate_vm_name(name)?;
        let lookup_name = name.to_owned();
        let name_lock_path = self.home.join("vms").join(name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let _name_guard = name_lock.lock().await;
        let stored = self
            .run_repository(move |repository| repository.find_microvm(&lookup_name))
            .await?
            .ok_or_else(|| SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: name.to_owned(),
            })?;
        let volume_lock = if stored.record.volume_path != name_lock_path {
            Some(self.target_lock(&stored.record.volume_path)?)
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        stored.require_complete("start MicroVM")?;
        validate_start_prerequisites(&stored)?;
        validate_persisted_network(&stored)?;
        let boot = verify_start_boot_artifacts(self, &stored).await?;
        if self.socket_answers(&stored.record.socket_path)? {
            let persisted_runtime =
                stored
                    .runtime
                    .as_ref()
                    .ok_or_else(|| SdkError::TemporaryRuntime {
                        component: "firecracker".to_owned(),
                        reason: "the machine is already running outside SDK management".to_owned(),
                        stopped: false,
                    })?;
            let Some(process_id) = persisted_runtime.process_id else {
                return Err(SdkError::TemporaryRuntime {
                    component: "firecracker".to_owned(),
                    reason: "the machine is already running outside SDK management".to_owned(),
                    stopped: false,
                });
            };
            if !self.runtime.process_references_vm(
                process_id,
                &stored.record.socket_path,
                &persisted_runtime.firecracker_path,
            )? {
                return Err(SdkError::TemporaryRuntime {
                    component: "firecracker".to_owned(),
                    reason: "the machine is already running outside SDK management".to_owned(),
                    stopped: false,
                });
            }
            return build_start_result(&stored, process_id);
        }
        self.clear_stale_runtime(&stored).await?;
        self.runtime.validate_host()?;
        let stored = self.reconcile_start_network(stored).await?;
        self.remove_stale_socket(&stored)?;
        let request = self.start_launch_request(&stored, &boot).await?;
        let process_id = match self.runtime.launch_detached(&request) {
            Ok(process_id) => process_id,
            Err(error) => {
                self.reset_runtime_to_stopped(&stored).await?;
                return Err(error);
            }
        };
        if process_id == 0 {
            self.reset_runtime_to_stopped(&stored).await?;
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker".to_owned(),
                reason: "the machine launch returned an invalid process identifier".to_owned(),
                stopped: true,
            });
        }
        match self.runtime.wait_for_socket(&stored.record.socket_path) {
            Ok(true) => {}
            Ok(false) => {
                self.cleanup_failed_launch(&stored, Some(process_id)).await;
                return Err(SdkError::TemporaryRuntime {
                    component: "firecracker.sock".to_owned(),
                    reason: "the machine control socket did not answer after launch".to_owned(),
                    stopped: true,
                });
            }
            Err(error) => {
                self.cleanup_failed_launch(&stored, Some(process_id)).await;
                return Err(error);
            }
        }
        let vm_id = stored.record.id;
        let socket_path = stored.record.socket_path.clone();
        let persisted_runtime = PersistedRuntime {
            firecracker_path: boot.firecracker_path,
            firectl_path: boot.firectl_path,
            socket_path: socket_path.clone(),
            process_id: Some(process_id),
            process_state: "running".to_owned(),
        };
        self.run_repository(move |repository| {
            repository.persist_runtime(vm_id, &persisted_runtime)
        })
        .await?;
        let lookup = stored.record.name.clone();
        let stored = self
            .run_repository(move |repository| {
                repository
                    .find_microvm(&lookup)?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: lookup.clone(),
                    })
            })
            .await?;
        build_start_result(&stored, process_id)
    }
    /// Stops one running MicroVM and reports whether forcing was used.
    ///
    /// The operation takes only the VM name: the volume directory and every
    /// runtime reference are read from the inventory record, never from the
    /// caller. Running means the volume-local control socket answers at call
    /// time; a silent socket returns success with `forced: false` and no host
    /// change. A running machine first receives one graceful shutdown request
    /// (`SendCtrlAltDel`) through its control socket with a 60-second exit
    /// wait, then immediate SIGKILL of the re-verified recorded process when
    /// it stays running. An undeliverable graceful request while the socket
    /// answers is a typed error with no forced attempt. The returned `forced`
    /// flag is `true` only when SIGKILL was delivered.
    pub async fn stop_microvm(&self, name: &str) -> Result<MicroVmStopResult, SdkError> {
        crate::domain::config::validate_vm_name(name)?;
        let lookup_name = name.to_owned();
        let name_lock_path = self.home.join("vms").join(name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let _name_guard = name_lock.lock().await;
        let stored = self
            .run_repository(move |repository| repository.find_microvm(&lookup_name))
            .await?
            .ok_or_else(|| SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: name.to_owned(),
            })?;
        let volume_lock = if stored.record.volume_path != name_lock_path {
            Some(self.target_lock(&stored.record.volume_path)?)
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        stored.require_complete("stop MicroVM")?;
        if !self.socket_answers(&stored.record.socket_path)? {
            self.clear_stale_runtime(&stored).await?;
            self.remove_stale_socket(&stored)?;
            return build_stop_result(&stored, false);
        }
        let delivered = self.runtime.request_shutdown(&stored.record.socket_path)?;
        if !delivered {
            if !self.socket_answers(&stored.record.socket_path)? {
                self.clear_stale_runtime(&stored).await?;
                self.remove_stale_socket(&stored)?;
                return build_stop_result(&stored, false);
            }
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker.sock".to_owned(),
                reason: "the machine shutdown request could not be confirmed".to_owned(),
                stopped: false,
            });
        }
        let runtime_snapshot = stored.runtime.clone();
        let firecracker_path = runtime_snapshot
            .as_ref()
            .map(|runtime| runtime.firecracker_path.clone())
            .unwrap_or_default();
        let process_id = runtime_snapshot.and_then(|runtime| runtime.process_id);
        if self.runtime.wait_for_stop(
            &stored.record.socket_path,
            process_id,
            &firecracker_path,
            STOP_EXIT_DEADLINE,
        )? {
            self.clear_stale_runtime(&stored).await?;
            self.remove_stale_socket(&stored)?;
            return build_stop_result(&stored, false);
        }
        let Some(pid) = process_id else {
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker".to_owned(),
                reason: "the machine is still running and has no process identity to force"
                    .to_owned(),
                stopped: false,
            });
        };
        if !self.runtime.process_references_vm(
            pid,
            &stored.record.socket_path,
            &firecracker_path,
        )? {
            if !self.socket_answers(&stored.record.socket_path)? {
                self.clear_stale_runtime(&stored).await?;
                self.remove_stale_socket(&stored)?;
                return build_stop_result(&stored, false);
            }
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker".to_owned(),
                reason: "the machine is still running but its process identity is unknown"
                    .to_owned(),
                stopped: false,
            });
        }
        self.runtime.terminate_spawned(pid)?;
        if self.runtime.wait_for_stop(
            &stored.record.socket_path,
            Some(pid),
            &firecracker_path,
            STOP_KILL_DEADLINE,
        )? {
            self.clear_stale_runtime(&stored).await?;
            self.remove_stale_socket(&stored)?;
            return build_stop_result(&stored, true);
        }
        Err(SdkError::TemporaryRuntime {
            component: "firecracker".to_owned(),
            reason: "the machine is still running after forced termination".to_owned(),
            stopped: false,
        })
    }

    /// Deletes one stopped MicroVM and everything it owns.
    ///
    /// The operation takes only the VM name: the volume directory and every
    /// owned reference are read from the inventory record, never from the
    /// caller. A running machine (its control socket answers) is refused
    /// with a stop-first lifecycle conflict and no host change; an
    /// unprobable socket propagates its typed probe error without deleting.
    /// Deletion runs host network release, then whole volume-directory
    /// removal, then inventory-record deletion last: every failure keeps the
    /// record so retrying the same delete resumes from the remaining owned
    /// resources. Already-absent owned files and network items count as
    /// already removed; only a fully unknown machine name fails as not-found.
    /// On success the record is gone, the whole volume directory is gone,
    /// and the VM's owned host network items are released, while shared
    /// kernels, images, tools, and other VMs stay intact.
    pub async fn delete_microvm(&self, name: &str) -> Result<MicroVmDeleteResult, SdkError> {
        crate::domain::config::validate_vm_name(name)?;

        let lookup_name = name.to_owned();
        let name_lock_path = self.home.join("vms").join(name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let _name_guard = name_lock.lock().await;
        let stored = self
            .run_repository(move |repository| repository.find_microvm(&lookup_name))
            .await?
            .ok_or_else(|| SdkError::NotFound {
                kind: "MicroVM".to_owned(),
                id: name.to_owned(),
            })?;
        let volume_lock = if stored.record.volume_path != name_lock_path {
            Some(self.target_lock(&stored.record.volume_path)?)
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        if self.socket_answers(&stored.record.socket_path)? {
            return Err(SdkError::LifecycleConflict {
                name: stored.record.name.clone(),
                state: MicroVmState::Running.to_string(),
                operation: "delete MicroVM (stop the machine first)".to_owned(),
            });
        }
        if let Some(network) = stored.network.clone() {
            self.network.cleanup_for_delete(&network)?;
        }
        remove_owned_volume(&self.home, &stored)?;
        let vm_id = stored.record.id;
        let deleted_name = stored.record.name.clone();
        self.run_repository(move |repository| repository.delete_microvm(vm_id))
            .await?;
        Ok(MicroVmDeleteResult { name: deleted_name })
    }

    /// Previews the kernels and images the next prune would reclaim.
    ///
    /// Returns the unreferenced candidates in the same deterministic order
    /// the deleting operation uses (kernels by ID, images by distribution
    /// then image, orphans by artifact key), with recorded byte sizes for
    /// display estimates. The preview is read-only: it opens no write
    /// transaction, deletes nothing, and treats existence of a MicroVM row
    /// alone as a reference. Callers must treat the result as an estimate:
    /// inventory may change before the deleting call runs.
    pub async fn list_prune_candidates(&self) -> Result<PrunePreview, SdkError> {
        let kernels = self
            .run_repository(|repository| repository.list_prunable_kernels())
            .await?;
        let images = self
            .run_repository(|repository| repository.list_prunable_images())
            .await?;
        let orphans = self
            .run_repository(|repository| repository.list_orphan_artifact_downloads())
            .await?;
        let references = self
            .run_repository(|repository| repository.list_prune_references())
            .await?;
        let mut preview = PrunePreview {
            kernels: Vec::new(),
            images: Vec::new(),
            estimated_bytes: 0,
        };
        for kernel in &kernels {
            if references.kernel_referenced(&kernel.registry_id) {
                continue;
            }
            preview.estimated_bytes = preview.estimated_bytes.saturating_add(kernel.size_bytes);
            preview.kernels.push(kernel.registry_id.clone());
        }
        for image in &images {
            if references.image_referenced(&image.distribution_id, &image.image_id) {
                continue;
            }
            preview.estimated_bytes = preview.estimated_bytes.saturating_add(image.size_bytes);
            preview.images.push(PrunedImageId {
                distribution_id: image.distribution_id.clone(),
                image_id: image.image_id.clone(),
            });
        }
        for orphan in &orphans {
            let Some(identity) = orphan.parse_identity() else {
                continue;
            };
            match identity {
                crate::ports::repository::OrphanIdentity::Kernel { kernel_id } => {
                    if references.kernel_referenced(&kernel_id) {
                        continue;
                    }
                    preview.estimated_bytes =
                        preview.estimated_bytes.saturating_add(orphan.size_bytes);
                    if !preview.kernels.contains(&kernel_id) {
                        preview.kernels.push(kernel_id);
                    }
                }
                crate::ports::repository::OrphanIdentity::DistributionImage {
                    distribution_id,
                    image_id,
                } => {
                    if references.image_referenced(&distribution_id, &image_id) {
                        continue;
                    }
                    preview.estimated_bytes =
                        preview.estimated_bytes.saturating_add(orphan.size_bytes);
                    let candidate = PrunedImageId {
                        distribution_id,
                        image_id,
                    };
                    if !preview.images.contains(&candidate) {
                        preview.images.push(candidate);
                    }
                }
            }
        }
        preview.kernels.sort();
        preview.images.sort_by(|first, second| {
            (&first.distribution_id, &first.image_id)
                .cmp(&(&second.distribution_id, &second.image_id))
        });
        Ok(preview)
    }

    pub async fn prune_unused_artifacts(&self) -> Result<PruneSummary, SdkError> {
        let kernels = self
            .run_repository(|repository| repository.list_prunable_kernels())
            .await?;
        let images = self
            .run_repository(|repository| repository.list_prunable_images())
            .await?;
        let orphans = self
            .run_repository(|repository| repository.list_orphan_artifact_downloads())
            .await?;
        let references = self
            .run_repository(|repository| repository.list_prune_references())
            .await?;
        let mut summary = PruneSummary {
            removed_kernels: Vec::new(),
            removed_images: Vec::new(),
            skipped_artifact_keys: Vec::new(),
            freed_bytes_kernels: 0,
            freed_bytes_images: 0,
            freed_bytes_total: 0,
        };
        let mut failures = Vec::new();
        for kernel in &kernels {
            if references.kernel_referenced(&kernel.registry_id) {
                continue;
            }
            self.prune_kernel_candidate(kernel, &mut summary, &mut failures)
                .await;
        }
        for image in &images {
            if references.image_referenced(&image.distribution_id, &image.image_id) {
                continue;
            }
            self.prune_image_candidate(image, &mut summary, &mut failures)
                .await;
        }
        for orphan in &orphans {
            self.prune_orphan_candidate(orphan, &references, &mut summary, &mut failures)
                .await;
        }
        summary.removed_kernels.sort();
        summary.removed_images.sort_by(|first, second| {
            (&first.distribution_id, &first.image_id)
                .cmp(&(&second.distribution_id, &second.image_id))
        });
        summary.skipped_artifact_keys.sort();
        summary.freed_bytes_total = summary
            .freed_bytes_kernels
            .checked_add(summary.freed_bytes_images)
            .ok_or_else(|| {
                SdkError::invalid_metadata("prune summary", "reclaimed bytes exceed u64 range")
            })?;
        if failures.is_empty() {
            Ok(summary)
        } else {
            Err(SdkError::PruneIncomplete { summary, failures })
        }
    }

    async fn prune_kernel_candidate(
        &self,
        kernel: &crate::ports::repository::PrunableKernel,
        summary: &mut PruneSummary,
        failures: &mut Vec<PruneFailure>,
    ) {
        let artifact_key = format!("kernel:{}", kernel.registry_id);
        match delete_prune_file(self, &kernel.absolute_path, &artifact_key).await {
            PruneFileOutcome::Skipped => {
                summary.skipped_artifact_keys.push(artifact_key);
            }
            PruneFileOutcome::Failure(reason) => {
                failures.push(PruneFailure {
                    artifact_key,
                    reason,
                });
            }
            PruneFileOutcome::Absent => {
                let kernel_id = kernel.registry_id.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_kernel_if_unreferenced(&kernel_id)
                    })
                    .await
                {
                    Ok(true) => summary.removed_kernels.push(kernel.registry_id.clone()),
                    Ok(false) => {}
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
            PruneFileOutcome::Removed { freed_bytes } => {
                let kernel_id = kernel.registry_id.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_kernel_if_unreferenced(&kernel_id)
                    })
                    .await
                {
                    Ok(true) => {
                        summary.removed_kernels.push(kernel.registry_id.clone());
                        summary.freed_bytes_kernels =
                            summary.freed_bytes_kernels.saturating_add(freed_bytes);
                    }
                    Ok(false) => {
                        failures.push(PruneFailure {
                            artifact_key,
                            reason:
                                "the artifact became referenced before its rows could be removed"
                                    .to_owned(),
                        });
                    }
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
        }
    }

    async fn prune_image_candidate(
        &self,
        image: &crate::ports::repository::PrunableImage,
        summary: &mut PruneSummary,
        failures: &mut Vec<PruneFailure>,
    ) {
        let artifact_key = format!(
            "distribution_image:{}:{}",
            image.distribution_id, image.image_id
        );
        match delete_prune_file(self, &image.absolute_path, &artifact_key).await {
            PruneFileOutcome::Skipped => {
                summary.skipped_artifact_keys.push(artifact_key);
            }
            PruneFileOutcome::Failure(reason) => {
                failures.push(PruneFailure {
                    artifact_key,
                    reason,
                });
            }
            PruneFileOutcome::Absent => {
                let distribution_id = image.distribution_id.clone();
                let image_id = image.image_id.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_image_if_unreferenced(&distribution_id, &image_id)
                    })
                    .await
                {
                    Ok(true) => summary.removed_images.push(PrunedImageId {
                        distribution_id: image.distribution_id.clone(),
                        image_id: image.image_id.clone(),
                    }),
                    Ok(false) => {}
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
            PruneFileOutcome::Removed { freed_bytes } => {
                let distribution_id = image.distribution_id.clone();
                let image_id = image.image_id.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_image_if_unreferenced(&distribution_id, &image_id)
                    })
                    .await
                {
                    Ok(true) => {
                        summary.removed_images.push(PrunedImageId {
                            distribution_id: image.distribution_id.clone(),
                            image_id: image.image_id.clone(),
                        });
                        summary.freed_bytes_images =
                            summary.freed_bytes_images.saturating_add(freed_bytes);
                    }
                    Ok(false) => {
                        failures.push(PruneFailure {
                            artifact_key,
                            reason:
                                "the artifact became referenced before its rows could be removed"
                                    .to_owned(),
                        });
                    }
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
        }
    }

    async fn prune_orphan_candidate(
        &self,
        orphan: &crate::ports::repository::OrphanArtifactDownload,
        references: &crate::ports::repository::PruneReferences,
        summary: &mut PruneSummary,
        failures: &mut Vec<PruneFailure>,
    ) {
        let artifact_key = orphan.artifact_key.clone();
        let Some(identity) = orphan.parse_identity() else {
            failures.push(PruneFailure {
                artifact_key,
                reason: "the orphan artifact key does not match its recorded artifact type"
                    .to_owned(),
            });
            return;
        };
        let referenced = match &identity {
            crate::ports::repository::OrphanIdentity::Kernel { kernel_id } => {
                references.kernel_referenced(kernel_id)
            }
            crate::ports::repository::OrphanIdentity::DistributionImage {
                distribution_id,
                image_id,
            } => references.image_referenced(distribution_id, image_id),
        };
        if referenced {
            return;
        }
        match delete_prune_file(self, &orphan.absolute_path, &artifact_key).await {
            PruneFileOutcome::Skipped => {
                summary.skipped_artifact_keys.push(artifact_key);
            }
            PruneFileOutcome::Failure(reason) => {
                failures.push(PruneFailure {
                    artifact_key,
                    reason,
                });
            }
            PruneFileOutcome::Absent => {
                let key = artifact_key.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_orphan_download_if_unreferenced(&key)
                    })
                    .await
                {
                    Ok(true) => match identity {
                        crate::ports::repository::OrphanIdentity::Kernel { kernel_id } => {
                            summary.removed_kernels.push(kernel_id);
                        }
                        crate::ports::repository::OrphanIdentity::DistributionImage {
                            distribution_id,
                            image_id,
                        } => summary.removed_images.push(PrunedImageId {
                            distribution_id,
                            image_id,
                        }),
                    },
                    Ok(false) => {}
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
            PruneFileOutcome::Removed { freed_bytes } => {
                let key = artifact_key.clone();
                match self
                    .run_repository(move |repository| {
                        repository.delete_orphan_download_if_unreferenced(&key)
                    })
                    .await
                {
                    Ok(true) => match identity {
                        crate::ports::repository::OrphanIdentity::Kernel { kernel_id } => {
                            summary.removed_kernels.push(kernel_id);
                            summary.freed_bytes_kernels =
                                summary.freed_bytes_kernels.saturating_add(freed_bytes);
                        }
                        crate::ports::repository::OrphanIdentity::DistributionImage {
                            distribution_id,
                            image_id,
                        } => {
                            summary.removed_images.push(PrunedImageId {
                                distribution_id,
                                image_id,
                            });
                            summary.freed_bytes_images =
                                summary.freed_bytes_images.saturating_add(freed_bytes);
                        }
                    },
                    Ok(false) => {
                        failures.push(PruneFailure {
                            artifact_key,
                            reason:
                                "the artifact became referenced before its rows could be removed"
                                    .to_owned(),
                        });
                    }
                    Err(error) => failures.push(PruneFailure {
                        artifact_key,
                        reason: error.to_string(),
                    }),
                }
            }
        }
    }

    fn socket_answers(&self, socket_path: &Path) -> Result<bool, SdkError> {
        self.runtime.socket_answers(socket_path)
    }

    async fn verify_all(&self, stored: Vec<&StoredMicroVm>) -> Vec<MicroVmState> {
        let permits = std::thread::available_parallelism()
            .map(|cores| cores.get().saturating_mul(4).clamp(8, 32))
            .unwrap_or(16);
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(permits));
        let mut pending = tokio::task::JoinSet::new();
        for (index, vm) in stored.iter().enumerate() {
            let permit = std::sync::Arc::clone(&semaphore);
            let runtime = std::sync::Arc::clone(&self.runtime);
            let socket_path = vm.record.socket_path.clone();
            let firecracker_path = vm
                .runtime
                .as_ref()
                .map(|runtime| runtime.firecracker_path.clone())
                .unwrap_or_default();
            let process_id = vm.runtime.as_ref().and_then(|runtime| runtime.process_id);
            pending.spawn(async move {
                let _guard = permit.acquire_owned().await;
                let probe = tokio::task::spawn_blocking(move || {
                    if let Some(pid) = process_id {
                        let _ = runtime.process_references_vm(pid, &socket_path, &firecracker_path);
                    }
                    runtime.socket_answers(&socket_path)
                });
                let live = matches!(
                    tokio::time::timeout(std::time::Duration::from_secs(PROBE_TIMEOUT_SECS), probe)
                        .await,
                    Ok(Ok(Ok(true)))
                );
                (
                    index,
                    if live {
                        MicroVmState::Running
                    } else {
                        MicroVmState::Stopped
                    },
                )
            });
        }
        let mut states = vec![MicroVmState::Stopped; stored.len()];
        while let Some(outcome) = pending.join_next().await {
            if let Ok((index, state)) = outcome {
                states[index] = state;
            }
        }
        states
    }

    /// Resets stale runtime references to stopped without claiming `Running`.
    /// A VM with no recorded process and an already stopped marker needs no write.
    async fn clear_stale_runtime(&self, stored: &StoredMicroVm) -> Result<(), SdkError> {
        let settled = stored.runtime.as_ref().is_none_or(|runtime| {
            runtime.process_id.is_none() && runtime.process_state == "stopped"
        });
        if settled {
            return Ok(());
        }
        self.reset_runtime_to_stopped(stored).await
    }

    /// Reconciles the persisted network attachment in place and persists the
    /// result. Correct items are skipped; only missing or stale items are
    /// recreated with the persisted identity unchanged.
    async fn reconcile_start_network(
        &self,
        stored: StoredMicroVm,
    ) -> Result<StoredMicroVm, SdkError> {
        let expected_mode = if stored.record.expose_on_lan {
            NetworkMode::Lan
        } else {
            NetworkMode::HostOnly
        };
        let (persisted_network, _, _) = stored.require_full("start MicroVM")?;
        if persisted_network.config.mode != expected_mode {
            return Err(SdkError::Network {
                mode: expected_mode.to_string(),
                operation: "validate persisted network mode".to_owned(),
                resource: stored.record.name.clone(),
                reason: "the persisted network mode does not match the VM configuration".to_owned(),
            });
        }
        let request = NetworkRequest {
            vm_name: stored.record.name.clone(),
            mode: expected_mode,
            guest_mac: persisted_network.guest_mac.clone(),
            lan_address_override: None,
        };
        let used_addresses = self
            .run_repository(|repository| repository.list_host_only_networks())
            .await?;
        let used_lan_addresses = self
            .run_repository(|repository| repository.list_lan_addresses())
            .await?;
        let outcome = self.network.configure(
            &request,
            Some(persisted_network),
            &used_addresses,
            &used_lan_addresses,
        )?;
        let vm_id = stored.record.id;
        let persisted_network = outcome.persisted.clone();
        self.run_repository(move |repository| repository.update_network(vm_id, &persisted_network))
            .await?;
        let lookup = stored.record.name.clone();
        self.run_repository(move |repository| {
            repository
                .find_microvm(&lookup)?
                .ok_or_else(|| SdkError::NotFound {
                    kind: "MicroVM".to_owned(),
                    id: lookup.clone(),
                })
        })
        .await
    }

    /// Removes a leftover socket file at exactly the persisted socket path,
    /// but only after liveness proved no listener is serving it.
    fn remove_stale_socket(&self, stored: &StoredMicroVm) -> Result<(), SdkError> {
        if !path_entry_exists(&stored.record.socket_path)? {
            return Ok(());
        }
        if self.runtime.socket_answers(&stored.record.socket_path)? {
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker.sock".to_owned(),
                reason: "the machine control socket answered while the VM was not live".to_owned(),
                stopped: false,
            });
        }
        fs::remove_file(&stored.record.socket_path).map_err(|error| {
            SdkError::filesystem(
                "remove stale machine control socket",
                &stored.record.socket_path,
                error,
            )
        })
    }

    /// Builds the detached launch request from validated persisted values.
    async fn start_launch_request(
        &self,
        stored: &StoredMicroVm,
        boot: &StartBootArtifacts,
    ) -> Result<StartRequest, SdkError> {
        let manifest = self.fetch_manifest().await?;
        let distribution = manifest
            .distributions
            .iter()
            .find(|candidate| candidate.id == stored.record.distribution_id)
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution".to_owned(),
                id: stored.record.distribution_id.clone(),
            })?;
        let mut kernel_options = distribution.boot.kernel_args.join(" ");
        if !kernel_options.is_empty() {
            kernel_options.push(' ');
        }
        kernel_options.push_str(&format!("root={} ", distribution.boot.root_device));
        let (persisted_network, _, _) = stored.require_full("start MicroVM")?;
        kernel_options.push_str(&persisted_network.desired_boot_parameters);
        Ok(StartRequest {
            vm_name: stored.record.name.clone(),
            firectl_path: boot.firectl_path.clone(),
            firecracker_path: boot.firecracker_path.clone(),
            kernel_path: boot.kernel_path.clone(),
            rootfs_path: stored.record.rootfs_path.clone(),
            vcpu_count: stored.record.vcpu_count,
            memory_effective_mib: stored.record.memory_effective_mib,
            kernel_options: kernel_options.trim().to_owned(),
            tap_name: persisted_network.config.tap_name.clone(),
            guest_mac: persisted_network.guest_mac.clone(),
            socket_path: stored.record.socket_path.clone(),
            log_path: stored.record.volume_path.join("firecracker.log"),
        })
    }

    /// Resets runtime refs to stopped after a pre-commit failure.
    async fn reset_runtime_to_stopped(&self, stored: &StoredMicroVm) -> Result<(), SdkError> {
        let vm_id = stored.record.id;
        let socket_path = stored.record.socket_path.clone();
        let (firecracker_path, firectl_path) = stored
            .runtime
            .as_ref()
            .map(|runtime| {
                (
                    runtime.firecracker_path.clone(),
                    runtime.firectl_path.clone(),
                )
            })
            .unwrap_or_default();
        self.run_repository(move |repository| {
            repository.persist_runtime(
                vm_id,
                &PersistedRuntime {
                    firecracker_path,
                    firectl_path,
                    socket_path,
                    process_id: None,
                    process_state: "stopped".to_owned(),
                },
            )
        })
        .await
    }

    /// Cleans up a failed launch: terminates only the just-spawned child and
    /// removes the owned socket only if this attempt created it. Repaired
    /// network items stay persisted. Never touches a previously recorded PID.
    async fn cleanup_failed_launch(&self, stored: &StoredMicroVm, spawned: Option<u32>) {
        if let Some(process_id) = spawned {
            let _ = self.runtime.terminate_spawned(process_id);
        }
        if path_entry_exists(&stored.record.socket_path).unwrap_or(false)
            && !self
                .runtime
                .socket_answers(&stored.record.socket_path)
                .unwrap_or(true)
        {
            let _ = fs::remove_file(&stored.record.socket_path);
        }
        let _ = self.reset_runtime_to_stopped(stored).await;
    }

    async fn return_or_reject_existing_creation<F>(
        &self,
        validated: &crate::domain::config::ValidatedCreateRequest,
        existing: StoredMicroVm,
        observer: &mut Option<&mut F>,
    ) -> Result<MicroVmCreationResult, SdkError>
    where
        F: FnMut(CreationProgress) + Send,
    {
        if !existing.is_complete() {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::Validation,
                    0,
                    CreationOutcome::Failed {
                        stage: CreationStage::Validation,
                    },
                ),
            );
            return Err(SdkError::LifecycleConflict {
                name: existing.record.name,
                state: "creation incomplete".to_owned(),
                operation: "create MicroVM".to_owned(),
            });
        }
        let comparisons = [
            (
                "distribution_id",
                existing.record.distribution_id.clone(),
                validated.request.distribution_id.clone(),
            ),
            (
                "image_id",
                existing.record.image_id.clone(),
                validated.request.image_id.clone(),
            ),
            (
                "disk_size_bytes",
                existing.record.disk_size_bytes.to_string(),
                validated.request.disk_size_bytes.to_string(),
            ),
            (
                "vcpu_count",
                existing.record.vcpu_count.to_string(),
                validated.request.vcpu_count.to_string(),
            ),
            (
                "memory_bytes",
                existing.record.memory_bytes.to_string(),
                validated.request.memory_bytes.to_string(),
            ),
            (
                "expose_on_lan",
                existing.record.expose_on_lan.to_string(),
                validated.request.expose_on_lan.to_string(),
            ),
            (
                "lan_address",
                existing
                    .network
                    .as_ref()
                    .and_then(|network| network.config.lan_address)
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                validated
                    .request
                    .lan_address
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
            (
                "volume_path",
                existing.record.volume_path.display().to_string(),
                validated.volume_path.display().to_string(),
            ),
        ];
        for (field, current, requested) in comparisons {
            if current != requested {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::Failed {
                            stage: CreationStage::Validation,
                        },
                    ),
                );
                return Err(SdkError::ConfigurationConflict {
                    name: existing.record.name,
                    field: field.to_owned(),
                    existing: current,
                    requested,
                });
            }
        }
        match self.load_creation_result(existing) {
            Ok(result) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::AlreadyConfigured,
                    ),
                );
                Ok(result)
            }
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::Validation,
                        0,
                        CreationOutcome::Failed {
                            stage: CreationStage::Validation,
                        },
                    ),
                );
                Err(error)
            }
        }
    }

    fn load_creation_result(
        &self,
        stored: StoredMicroVm,
    ) -> Result<MicroVmCreationResult, SdkError> {
        validate_persisted_files(&stored)?;
        validate_persisted_network(&stored)?;
        Ok(build_creation_result(&stored))
    }

    async fn resolve_creation_prerequisites<F>(
        &self,
        request: &CreateMicroVmRequest,
        observer: &mut Option<&mut F>,
    ) -> Result<CreationPrerequisites, SdkError>
    where
        F: FnMut(CreationProgress) + Send,
    {
        let host_architecture = host_architecture()?;
        let manifest = self.fetch_manifest().await?;
        let distribution = manifest
            .distributions
            .iter()
            .find(|candidate| candidate.id == request.distribution_id)
            .cloned()
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution".to_owned(),
                id: request.distribution_id.clone(),
            })?;
        if distribution.architecture != host_architecture {
            return Err(SdkError::IncompatibleArtifact {
                artifact: format!("distribution:{}", distribution.id),
                reason: format!(
                    "architecture {} is not supported by host architecture {}",
                    architecture_name(&distribution.architecture),
                    architecture_name(&host_architecture)
                ),
            });
        }
        let image = distribution
            .images
            .iter()
            .find(|candidate| candidate.id == request.image_id)
            .cloned()
            .ok_or_else(|| SdkError::NotFound {
                kind: "distribution image".to_owned(),
                id: request.image_id.clone(),
            })?;
        if image.format != "ext4" || image.filesystem.filesystem_type != "ext4" {
            return Err(SdkError::IncompatibleArtifact {
                artifact: format!("distribution image:{}", image.id),
                reason: "the selected image is not an ext4 filesystem image".to_owned(),
            });
        }
        if request.disk_size_bytes < image.size_bytes {
            return Err(SdkError::DiskSizeTooSmall {
                image_id: image.id.clone(),
                requested_size_bytes: request.disk_size_bytes,
                source_size_bytes: image.size_bytes,
            });
        }
        let minimum_memory = minimum_memory_bytes(distribution.requirements.min_memory_mb)?;
        if request.memory_bytes < minimum_memory {
            return Err(SdkError::InvalidRequest {
                field: "memory_bytes".to_owned(),
                reason: format!(
                    "must be at least {} bytes for distribution {}",
                    minimum_memory, distribution.id
                ),
            });
        }
        if request.vcpu_count < distribution.requirements.min_vcpus {
            return Err(SdkError::InvalidRequest {
                field: "vcpu_count".to_owned(),
                reason: format!(
                    "must be at least {} vCPUs for distribution {}",
                    distribution.requirements.min_vcpus, distribution.id
                ),
            });
        }
        let kernel = manifest
            .kernels
            .iter()
            .find(|candidate| candidate.id == distribution.default_kernel)
            .cloned()
            .ok_or_else(|| SdkError::ArtifactPrerequisite {
                kind: "kernel".to_owned(),
                id: distribution.default_kernel.clone(),
                path: self.home.join("artifacts/kernels"),
                reason: "the distribution default kernel is absent from the registry manifest"
                    .to_owned(),
            })?;
        if kernel.architecture != host_architecture {
            return Err(SdkError::IncompatibleArtifact {
                artifact: format!("kernel:{}", kernel.id),
                reason: format!(
                    "kernel architecture {} does not match host architecture {}",
                    architecture_name(&kernel.architecture),
                    architecture_name(&host_architecture)
                ),
            });
        }
        let image_local = self
            .run_repository({
                let distribution_id = distribution.id.clone();
                let image_id = image.id.clone();
                move |repository| repository.resolve_distribution_image(&distribution_id, &image_id)
            })
            .await
            .map_err(|error| {
                map_artifact_resolution_error(
                    error,
                    "distribution image",
                    &image.id,
                    self.home.join("artifacts/rootfs"),
                )
            })?;
        verify_local_artifact_with_progress(
            &self.home,
            &image_local.path,
            image_local.size_bytes,
            &image_local.sha256,
            image.size_bytes,
            &image.sha256,
            "distribution image",
            &image.id,
            &mut |bytes_done: u64, expected: u64| {
                emit_creation_progress(
                    observer,
                    creation_tick_event(
                        CreationStage::PrerequisiteResolution,
                        1,
                        bytes_done,
                        expected.max(1),
                    ),
                );
            },
        )
        .await?;
        let kernel_local = self
            .run_repository({
                let kernel_id = kernel.id.clone();
                move |repository| repository.resolve_kernel(&kernel_id)
            })
            .await
            .map_err(|error| {
                map_artifact_resolution_error(
                    error,
                    "kernel",
                    &kernel.id,
                    self.home.join("artifacts/kernels"),
                )
            })?;
        verify_local_artifact_with_progress(
            &self.home,
            &kernel_local.path,
            kernel_local.size_bytes,
            &kernel_local.sha256,
            kernel.size_bytes,
            &kernel.sha256,
            "kernel",
            &kernel.id,
            &mut |bytes_done: u64, expected: u64| {
                emit_creation_progress(
                    observer,
                    creation_tick_event(
                        CreationStage::PrerequisiteResolution,
                        1,
                        bytes_done,
                        expected.max(1),
                    ),
                );
            },
        )
        .await?;

        let firecracker = self
            .select_runtime_binary(&manifest.binaries, "firecracker", &host_architecture)
            .await?;
        let firectl = self
            .select_runtime_binary(&manifest.binaries, "firectl", &host_architecture)
            .await?;
        Ok(CreationPrerequisites {
            image_path: image_local.path,
            kernel,
            firecracker,
            firectl,
        })
    }
    async fn select_runtime_binary(
        &self,
        packages: &[BinaryPackage],
        component: &str,
        host_architecture: &Architecture,
    ) -> Result<InstalledBinary, SdkError> {
        let mut candidates = Vec::new();
        for package in packages {
            if package.architecture != *host_architecture
                || !package.files.iter().any(|file| file.name == component)
            {
                continue;
            }
            let Ok(version) = Version::parse(&package.version) else {
                continue;
            };
            candidates.push((version, package.clone()));
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.id.cmp(&right.1.id))
        });
        let fallback_path = self.home.join("tools").join(component);
        let mut last_error = None;
        for (_, package) in candidates {
            match self.resolve_binary(&package.id, component).await {
                Ok(binary) => {
                    if binary.architecture != *host_architecture || !binary.executable {
                        last_error = Some(SdkError::RuntimeIncompatible {
                            component: component.to_owned(),
                            package_id: package.id,
                            version: package.version,
                            architecture: architecture_name(&binary.architecture).to_owned(),
                            reason: "the installed component is not an executable for the host"
                                .to_owned(),
                        });
                        continue;
                    }
                    if binary.version != package.version {
                        last_error = Some(SdkError::RuntimeIncompatible {
                            component: component.to_owned(),
                            package_id: package.id,
                            version: binary.version,
                            architecture: architecture_name(&binary.architecture).to_owned(),
                            reason: "the installed inventory version does not match the registry"
                                .to_owned(),
                        });
                        continue;
                    }
                    if let Err(error) = verify_executable_file(&binary.path, component) {
                        last_error = Some(error);
                        continue;
                    }
                    return Ok(binary);
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(SdkError::ArtifactPrerequisite {
            kind: "runtime binary".to_owned(),
            id: component.to_owned(),
            path: fallback_path,
            reason: match last_error {
                Some(error) => {
                    format!("no verified host-compatible {component} is available: {error}")
                }
                None => format!(
                    "download a verified {component} package for host architecture {} first",
                    architecture_name(host_architecture)
                ),
            },
        })
    }

    async fn ensure_volume_available(
        &self,
        vm_name: &str,
        volume_path: &Path,
    ) -> Result<(), SdkError> {
        let owner_path = volume_path.to_path_buf();
        if let Some(owner) = self
            .run_repository(move |repository| repository.find_volume_owner(&owner_path))
            .await?
            .filter(|owner| owner != vm_name)
        {
            return Err(SdkError::StorageConflict {
                vm_name: vm_name.to_owned(),
                volume_path: volume_path.to_path_buf(),
                owner,
                reason: "the volume is already assigned to another VM".to_owned(),
            });
        }
        match fs::symlink_metadata(volume_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(SdkError::StorageConflict {
                        vm_name: vm_name.to_owned(),
                        volume_path: volume_path.to_path_buf(),
                        owner: "unmanaged filesystem entry".to_owned(),
                        reason: "the VM volume must be a real directory".to_owned(),
                    });
                }
                let mut entries = fs::read_dir(volume_path).map_err(|error| {
                    SdkError::filesystem("inspect VM volume contents", volume_path, error)
                })?;
                if let Some(entry) = entries.next() {
                    entry.map_err(|error| {
                        SdkError::filesystem("inspect VM volume contents", volume_path, error)
                    })?;
                    return Err(SdkError::StorageConflict {
                        vm_name: vm_name.to_owned(),
                        volume_path: volume_path.to_path_buf(),
                        owner: "unmanaged volume contents".to_owned(),
                        reason: "refusing to mix creation-owned files with existing data"
                            .to_owned(),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(SdkError::filesystem(
                    "inspect VM volume directory",
                    volume_path,
                    error,
                ));
            }
        }
        Ok(())
    }

    async fn create_claimed_microvm<F>(
        &self,
        validated: &crate::domain::config::ValidatedCreateRequest,
        prerequisites: &CreationPrerequisites,
        record: &MicroVmRecord,
        journal: &mut CreationJournal,
        observer: &mut Option<&mut F>,
    ) -> Result<MicroVmCreationResult, SdkError>
    where
        F: FnMut(CreationProgress) + Send,
    {
        emit_creation_progress(
            observer,
            creation_started_event(CreationStage::VolumePreparation, 2),
        );
        let disk_size_bytes = validated.request.disk_size_bytes;
        let prepared = match self.storage.prepare_rootfs(
            &prerequisites.image_path,
            &record.volume_path,
            disk_size_bytes,
            &mut |bytes_copied: u64, expected_bytes: u64| {
                emit_creation_progress(
                    observer,
                    creation_tick_event(
                        CreationStage::VolumePreparation,
                        2,
                        bytes_copied,
                        expected_bytes.max(1),
                    ),
                );
            },
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::VolumePreparation,
                        2,
                        CreationOutcome::Failed {
                            stage: CreationStage::VolumePreparation,
                        },
                    ),
                );
                return Err(error);
            }
        };
        journal.volume_created = prepared.created_volume;
        journal.rootfs_path = Some(prepared.path.clone());
        emit_creation_progress(
            observer,
            creation_stage_event(CreationStage::VolumePreparation, 3),
        );
        emit_creation_progress(
            observer,
            creation_started_event(CreationStage::CredentialSetup, 3),
        );
        journal.ssh_directory = Some(record.volume_path.join("ssh"));
        let generated = match self.credentials.generate(&record.volume_path) {
            Ok(generated) => generated,
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::CredentialSetup,
                        3,
                        CreationOutcome::Failed {
                            stage: CreationStage::CredentialSetup,
                        },
                    ),
                );
                return Err(error);
            }
        };
        journal.private_key_path = Some(generated.private_key_path.clone());
        journal.public_key_path = Some(generated.public_key_path.clone());
        if let Err(error) = self
            .storage
            .inject_public_key(&prepared.path, &generated.public_key)
        {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::CredentialSetup,
                    3,
                    CreationOutcome::Failed {
                        stage: CreationStage::CredentialSetup,
                    },
                ),
            );
            return Err(error);
        }
        emit_creation_progress(
            observer,
            creation_stage_event(CreationStage::CredentialSetup, 4),
        );
        emit_creation_progress(
            observer,
            creation_started_event(CreationStage::NetworkConfiguration, 4),
        );
        let guest_mac = crate::adapters::network::linux::guest_mac(&record.name);
        let network_request = NetworkRequest {
            vm_name: record.name.clone(),
            mode: if validated.request.expose_on_lan {
                NetworkMode::Lan
            } else {
                NetworkMode::HostOnly
            },
            guest_mac,
            lan_address_override: validated.request.lan_address,
        };
        let used_addresses = match self
            .run_repository(|repository| repository.list_host_only_networks())
            .await
        {
            Ok(used_addresses) => used_addresses,
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::NetworkConfiguration,
                        4,
                        CreationOutcome::Failed {
                            stage: CreationStage::NetworkConfiguration,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let used_lan_addresses = match self
            .run_repository(|repository| repository.list_lan_addresses())
            .await
        {
            Ok(used_lan_addresses) => used_lan_addresses,
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::NetworkConfiguration,
                        4,
                        CreationOutcome::Failed {
                            stage: CreationStage::NetworkConfiguration,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let outcome = match self.network.configure(
            &network_request,
            None,
            &used_addresses,
            &used_lan_addresses,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                emit_creation_progress(
                    observer,
                    creation_terminal_event(
                        CreationStage::NetworkConfiguration,
                        4,
                        CreationOutcome::Failed {
                            stage: CreationStage::NetworkConfiguration,
                        },
                    ),
                );
                return Err(error);
            }
        };
        let guest_config_result = match (
            outcome.persisted.config.guest_address,
            outcome.persisted.config.gateway,
            outcome.persisted.config.lan_address,
        ) {
            (
                std::net::IpAddr::V4(guest_address),
                Some(std::net::IpAddr::V4(gateway)),
                Some(std::net::IpAddr::V4(lan_address)),
            ) => self
                .storage
                .write_guest_lan_config(&prepared.path, guest_address, gateway, lan_address)
                .map(|_| ()),
            (std::net::IpAddr::V4(guest_address), Some(std::net::IpAddr::V4(gateway)), _) => self
                .storage
                .write_guest_network_config(&prepared.path, guest_address, gateway)
                .map(|_| ()),
            _ => Ok(()),
        };
        if let Err(error) = guest_config_result {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::NetworkConfiguration,
                    4,
                    CreationOutcome::Failed {
                        stage: CreationStage::NetworkConfiguration,
                    },
                ),
            );
            return Err(error);
        }
        journal.network = Some(outcome.persisted.clone());
        emit_creation_progress(
            observer,
            creation_stage_event(CreationStage::NetworkConfiguration, 5),
        );
        emit_creation_progress(
            observer,
            creation_started_event(CreationStage::Finalization, 5),
        );
        let credential = PersistedCredential {
            private_key_path: generated.private_key_path,
            public_key_path: generated.public_key_path,
            guest_authorized_keys_path: "/root/.ssh/authorized_keys".to_owned(),
            key_type: "ed25519".to_owned(),
            ssh_user: "root".to_owned(),
            ssh_port: 22,
            public_key_fingerprint: generated.fingerprint,
            file_mode: generated.file_mode,
        };
        let vm_id = record.id;
        let initial_network = outcome.persisted.clone();
        let initial_credential = credential.clone();
        if let Err(error) = self
            .run_repository(move |repository| {
                repository.persist_network(vm_id, &initial_network)?;
                repository.persist_credential(vm_id, &initial_credential)
            })
            .await
        {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::Finalization,
                    5,
                    CreationOutcome::Failed {
                        stage: CreationStage::Finalization,
                    },
                ),
            );
            return Err(error);
        }

        let runtime_record = PersistedRuntime {
            firecracker_path: prerequisites.firecracker.path.clone(),
            firectl_path: prerequisites.firectl.path.clone(),
            socket_path: record.socket_path.clone(),
            process_id: None,
            process_state: "stopped".to_owned(),
        };
        if let Err(error) = self.runtime.verify_stopped(&record.socket_path) {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::Finalization,
                    5,
                    CreationOutcome::Failed {
                        stage: CreationStage::Finalization,
                    },
                ),
            );
            return Err(error);
        }
        let persisted_runtime = runtime_record.clone();
        if let Err(error) = self
            .run_repository(move |repository| repository.persist_runtime(vm_id, &persisted_runtime))
            .await
        {
            emit_creation_progress(
                observer,
                creation_terminal_event(
                    CreationStage::Finalization,
                    5,
                    CreationOutcome::Failed {
                        stage: CreationStage::Finalization,
                    },
                ),
            );
            return Err(error);
        }
        emit_creation_progress(
            observer,
            creation_stage_event(CreationStage::Finalization, 6),
        );
        Ok(MicroVmCreationResult {
            name: record.name.clone(),
            state: MicroVmState::Stopped,
            distribution_id: record.distribution_id.clone(),
            image_id: record.image_id.clone(),
            volume_path: record.volume_path.clone(),
            rootfs_path: record.rootfs_path.clone(),
            socket_path: record.socket_path.clone(),
            vcpu_count: record.vcpu_count,
            memory_bytes: record.memory_bytes,
            disk_size_bytes: record.disk_size_bytes,
            network: outcome.persisted.config,
            ssh: SshConnectionInfo {
                user: credential.ssh_user,
                port: credential.ssh_port,
                address: journal
                    .network
                    .as_ref()
                    .map(|network| network.config.guest_address)
                    .unwrap_or_else(unspecified_address),
                private_key_path: credential.private_key_path,
                public_key_path: credential.public_key_path,
            },
        })
    }

    async fn rollback_creation(&self, journal: &CreationJournal) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(network) = &journal.network {
            let mut cleanup_network = network.clone();
            if cleanup_network.config.mode == NetworkMode::Lan
                && (cleanup_network.bridge_created_by_sdk || cleanup_network.uplink_attached_by_sdk)
                && let (Some(bridge), Some(uplink)) = (
                    cleanup_network.config.bridge_name.as_deref(),
                    cleanup_network.config.uplink_name.as_deref(),
                )
            {
                if let Some(vm_id) = journal.vm_id {
                    match self
                        .run_repository({
                            let bridge = bridge.to_owned();
                            let uplink = uplink.to_owned();
                            move |repository| {
                                repository.bridge_has_other_references(vm_id, &bridge, &uplink)
                            }
                        })
                        .await
                    {
                        Ok(true) => {
                            cleanup_network.bridge_created_by_sdk = false;
                            cleanup_network.uplink_attached_by_sdk = false;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            failures.push(error.to_string());
                            cleanup_network.bridge_created_by_sdk = false;
                            cleanup_network.uplink_attached_by_sdk = false;
                        }
                    }
                } else {
                    failures.push("network rollback has no provisional VM identity".to_owned());
                    cleanup_network.bridge_created_by_sdk = false;
                    cleanup_network.uplink_attached_by_sdk = false;
                }
            }
            if let Err(error) = self.network.cleanup(&cleanup_network) {
                failures.push(error.to_string());
            }
        }
        for path in [
            &journal.private_key_path,
            &journal.public_key_path,
            &journal.rootfs_path,
        ]
        .into_iter()
        .flatten()
        {
            match fs::symlink_metadata(path) {
                Ok(_) => {
                    if let Err(error) = fs::remove_file(path) {
                        failures.push(format!("remove {}: {error}", path.display()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("inspect {}: {error}", path.display())),
            }
        }
        if let Some(path) = &journal.ssh_directory {
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("remove {}: {error}", path.display())),
            }
        }
        if journal.volume_created
            && let Err(error) = fs::remove_dir(&journal.volume_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            failures.push(format!("remove {}: {error}", journal.volume_path.display()));
        }
        if let Some(vm_id) = journal.vm_id
            && let Err(error) = self
                .run_repository(move |repository| repository.delete_microvm(vm_id))
                .await
        {
            failures.push(error.to_string());
        }
        failures
    }

    pub(crate) async fn fetch_manifest(&self) -> Result<TaumaruRegistry, SdkError> {
        self.registry.fetch_manifest().await
    }

    pub(crate) fn target_lock(
        &self,
        target: &Path,
    ) -> Result<Arc<tokio::sync::Mutex<()>>, SdkError> {
        let mut locks = self
            .target_locks
            .lock()
            .map_err(|_| SdkError::Concurrency {
                target: target.to_path_buf(),
            })?;
        Ok(locks
            .entry(target.to_path_buf())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }

    pub(crate) async fn run_repository<T, F>(&self, operation: F) -> Result<T, SdkError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn LocalRepository) -> Result<T, SdkError> + Send + 'static,
    {
        let repository = Arc::clone(&self.repository);
        tokio::task::spawn_blocking(move || operation(repository.as_ref())).await?
    }

    fn kernel_member(&self, kernel: Kernel) -> Result<DownloadMember, SdkError> {
        let relative_path = PathBuf::from("artifacts")
            .join("kernels")
            .join(&kernel.id)
            .join(&kernel.filename);
        let spec = make_download_spec(
            &self.home,
            DownloadSpecInput {
                artifact_kind: ArtifactKind::Kernel,
                artifact_id: kernel.id.clone(),
                member_name: None,
                artifact_key: format!("kernel:{}", kernel.id),
                registry_path: kernel.path.clone(),
                registry_url: kernel.url.clone(),
                filename: kernel.filename.clone(),
                expected_size: kernel.size_bytes,
                expected_sha256: kernel.sha256.clone(),
                executable: false,
                mode: None,
                relative_path,
            },
        )?;
        Ok(DownloadMember {
            spec,
            logical: LogicalMember::Kernel { kernel },
        })
    }

    fn binary_member(
        &self,
        package: BinaryPackage,
        file: BinaryFile,
    ) -> Result<DownloadMember, SdkError> {
        let relative_path = PathBuf::from("tools")
            .join(&package.id)
            .join(&file.name)
            .join(&file.filename);
        let spec = make_download_spec(
            &self.home,
            DownloadSpecInput {
                artifact_kind: ArtifactKind::Binary,
                artifact_id: package.id.clone(),
                member_name: Some(file.name.clone()),
                artifact_key: format!("binary:{}:{}", package.id, file.name),
                registry_path: file.path.clone(),
                registry_url: file.url.clone(),
                filename: file.filename.clone(),
                expected_size: file.size_bytes,
                expected_sha256: file.sha256.clone(),
                executable: file.executable,
                mode: file.mode.clone(),
                relative_path,
            },
        )?;
        Ok(DownloadMember {
            spec,
            logical: LogicalMember::Binary { package, file },
        })
    }

    fn distribution_image_member(
        &self,
        distribution: Distribution,
        image: DistributionImage,
        kernels: Vec<Kernel>,
    ) -> Result<DownloadMember, SdkError> {
        let relative_path = PathBuf::from("artifacts")
            .join("rootfs")
            .join(&distribution.id)
            .join(&image.id)
            .join(&image.filename);
        let spec = make_download_spec(
            &self.home,
            DownloadSpecInput {
                artifact_kind: ArtifactKind::DistributionImage,
                artifact_id: distribution.id.clone(),
                member_name: Some(image.id.clone()),
                artifact_key: format!("distribution_image:{}:{}", distribution.id, image.id),
                registry_path: image.path.clone(),
                registry_url: image.url.clone(),
                filename: image.filename.clone(),
                expected_size: image.size_bytes,
                expected_sha256: image.sha256.clone(),
                executable: false,
                mode: None,
                relative_path,
            },
        )?;
        Ok(DownloadMember {
            spec,
            logical: LogicalMember::DistributionImage {
                distribution,
                image: Box::new(image),
                kernels,
            },
        })
    }

    async fn download_member<F>(
        &self,
        member: &DownloadMember,
        cancellation: &DownloadCancellation,
        tracker: &mut ProgressTracker,
        on_progress: &mut F,
    ) -> Result<DownloadedFile, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, 0));
            return Err(SdkError::Cancelled);
        }
        let target_lock = self.target_lock(&member.spec.absolute_path)?;
        let _target_guard = target_lock.lock().await;
        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, 0));
            return Err(SdkError::Cancelled);
        }
        // Image and kernel reuse trusts a `Complete` inventory row without re-hashing
        // the file. Hashing runs only when adoption is possible (a `Missing` or
        // `Incomplete` row with a physical file to persist) or for binary members,
        // which keep the previous hash-first behavior because creation re-resolves
        // them on every run.
        let hash_first = matches!(member.spec.artifact_kind, ArtifactKind::Binary);
        let repository_spec = member.spec.clone();
        let inventory_state = self
            .run_repository(move |repository| repository.inspect_member(&repository_spec, None))
            .await?;
        let physical_integrity = if hash_first
            || matches!(
                inventory_state,
                InventoryState::Missing | InventoryState::Incomplete
            ) {
            calculate_file_integrity(&member.spec.absolute_path).await?
        } else {
            None
        };

        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, 0));
            return Err(SdkError::Cancelled);
        }

        match decide_cache(&member.spec, physical_integrity.as_ref(), inventory_state) {
            CacheDecision::Skip => {
                if hash_first {
                    let integrity = physical_integrity.ok_or_else(|| {
                        SdkError::Migration(format!(
                            "complete inventory has no physical file for {}",
                            member.spec.artifact_key
                        ))
                    })?;
                    on_progress(tracker.event(&member.spec, DownloadPhase::SkippedExisting, 0));
                    return Ok(downloaded_file(
                        member,
                        integrity,
                        DownloadDisposition::SkippedExisting,
                    ));
                }
                if let Some(integrity) = physical_integrity.as_ref() {
                    // A computed hash disagrees with the Complete row: the file
                    // changed after verification, so replace instead of Skip.
                    // (Warm-cache repeats skip this entirely: no hash computed.)
                    if integrity.size_bytes != member.spec.expected_size
                        || integrity.sha256 != member.spec.expected_sha256
                    {
                        return self
                            .replace_member(member, cancellation, tracker, on_progress)
                            .await;
                    }
                }
                let verified_size = member.spec.expected_size;
                let verified_digest = member.spec.expected_sha256.clone();
                let target = member.spec.absolute_path.clone();
                let metadata = match async_fs::symlink_metadata(&target).await {
                    Ok(metadata) => metadata,
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                        return self
                            .replace_member(member, cancellation, tracker, on_progress)
                            .await;
                    }
                    Err(source) => {
                        return Err(SdkError::filesystem(
                            "inspect artifact file",
                            &target,
                            source,
                        ));
                    }
                };
                if !target_is_reusable_file(metadata.file_type(), metadata.is_file()) {
                    // Inventory claims a file that is gone or not regular: fall
                    // through to Replace instead of reporting a phantom Skip.
                    return self
                        .replace_member(member, cancellation, tracker, on_progress)
                        .await;
                }
                if metadata.len() != verified_size {
                    // Cheap size signal (no read) already disproves the Complete
                    // row: replace without paying for a doomed hash.
                    return self
                        .replace_member(member, cancellation, tracker, on_progress)
                        .await;
                }
                let integrity = FileIntegrity {
                    size_bytes: verified_size,
                    sha256: verified_digest,
                };
                on_progress(tracker.event(&member.spec, DownloadPhase::SkippedExisting, 0));
                Ok(downloaded_file(
                    member,
                    integrity,
                    DownloadDisposition::SkippedExisting,
                ))
            }
            CacheDecision::Adopt => {
                let integrity = match physical_integrity {
                    Some(integrity) => integrity,
                    None => {
                        // Unrecorded file with correct bytes: hash once to persist the
                        // adoption. Complete rows never reach this arm (Skip above).
                        let verified_size = member.spec.expected_size;
                        let target = member.spec.absolute_path.clone();
                        let metadata =
                            async_fs::symlink_metadata(&target)
                                .await
                                .map_err(|source| {
                                    SdkError::filesystem("inspect artifact file", &target, source)
                                })?;
                        if target_is_reusable_file(metadata.file_type(), metadata.is_file()) {
                            calculate_file_integrity(&member.spec.absolute_path).await?
                        } else {
                            None
                        }
                        .filter(|integrity| {
                            integrity.size_bytes == verified_size
                                && integrity.sha256 == member.spec.expected_sha256
                        })
                        .ok_or_else(|| {
                            SdkError::Migration(format!(
                                "cache adoption has no physical file for {}",
                                member.spec.artifact_key
                            ))
                        })?
                    }
                };
                self.persist_member(member, &integrity).await?;
                on_progress(tracker.event(&member.spec, DownloadPhase::AdoptedExisting, 0));
                Ok(downloaded_file(
                    member,
                    integrity,
                    DownloadDisposition::AdoptedExisting,
                ))
            }
            CacheDecision::Replace => {
                self.replace_member(member, cancellation, tracker, on_progress)
                    .await
            }
        }
    }

    async fn replace_member<F>(
        &self,
        member: &DownloadMember,
        cancellation: &DownloadCancellation,
        tracker: &mut ProgressTracker,
        on_progress: &mut F,
    ) -> Result<DownloadedFile, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, 0));
            return Err(SdkError::Cancelled);
        }
        let removal_spec = member.spec.clone();
        self.run_repository(move |repository| repository.remove_member(&removal_spec))
            .await?;
        remove_invalid_target_if_exists(&member.spec.absolute_path).await?;
        let integrity = self
            .stream_and_publish(member, cancellation, tracker, on_progress)
            .await?;
        self.persist_member(member, &integrity).await?;
        on_progress(tracker.event(&member.spec, DownloadPhase::Completed, integrity.size_bytes));
        tracker.commit_member(integrity.size_bytes);
        Ok(downloaded_file(
            member,
            integrity,
            DownloadDisposition::Downloaded,
        ))
    }

    async fn persist_member(
        &self,
        member: &DownloadMember,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError> {
        let member = member.clone();
        let integrity = integrity.clone();
        self.run_repository(move |repository| {
            let DownloadMember { spec, logical } = member;
            match logical {
                LogicalMember::Kernel { kernel } => {
                    repository.persist_kernel(&kernel, &spec, &integrity)
                }
                LogicalMember::Binary { package, file } => {
                    repository.persist_binary_file(&package, &file, &spec, &integrity)
                }
                LogicalMember::DistributionImage {
                    distribution,
                    image,
                    kernels,
                } => repository.persist_distribution_image(
                    &distribution,
                    &image,
                    &kernels,
                    &spec,
                    &integrity,
                ),
            }
        })
        .await
    }

    async fn stream_and_publish<F>(
        &self,
        member: &DownloadMember,
        cancellation: &DownloadCancellation,
        tracker: &ProgressTracker,
        on_progress: &mut F,
    ) -> Result<FileIntegrity, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let temporary_path = temporary_path(&self.home, &member.spec.artifact_key)?;
        let result = self
            .stream_to_temporary(member, cancellation, tracker, on_progress, &temporary_path)
            .await;
        let integrity = match result {
            Ok(integrity) => integrity,
            Err(error) => {
                let _ = async_fs::remove_file(&temporary_path).await;
                return Err(error);
            }
        };
        if cancellation.is_cancelled() {
            let _ = async_fs::remove_file(&temporary_path).await;
            on_progress(tracker.event(
                &member.spec,
                DownloadPhase::Cancelled,
                integrity.size_bytes,
            ));
            return Err(SdkError::Cancelled);
        }
        if let Err(error) = async_fs::rename(&temporary_path, &member.spec.absolute_path)
            .await
            .map_err(|source| {
                SdkError::filesystem(
                    "atomically publish downloaded file",
                    &member.spec.absolute_path,
                    source,
                )
            })
        {
            let _ = async_fs::remove_file(&temporary_path).await;
            return Err(error);
        }
        Ok(integrity)
    }

    async fn stream_to_temporary<F>(
        &self,
        member: &DownloadMember,
        cancellation: &DownloadCancellation,
        tracker: &ProgressTracker,
        on_progress: &mut F,
        temporary_path: &Path,
    ) -> Result<FileIntegrity, SdkError>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let parent =
            member
                .spec
                .absolute_path
                .parent()
                .ok_or_else(|| SdkError::InvalidMetadata {
                    artifact: member.spec.artifact_key.clone(),
                    reason: "download target has no parent directory".to_owned(),
                })?;
        async_fs::create_dir_all(parent)
            .await
            .map_err(|source| SdkError::filesystem("create artifact directory", parent, source))?;
        let mut file = async_fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)
            .await
            .map_err(|source| {
                SdkError::filesystem("create temporary download file", temporary_path, source)
            })?;
        let mut response = self.registry.fetch_file(&member.spec.registry_url).await?;
        let mut hasher = Sha256::new();
        let mut size_bytes = 0_u64;
        let cancellation_token = cancellation.token();
        loop {
            if cancellation.is_cancelled() {
                on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, size_bytes));
                return Err(SdkError::Cancelled);
            }
            let chunk = tokio::select! {
                result = response.chunk() => result?,
                _ = cancellation_token.cancelled() => {
                    on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, size_bytes));
                    return Err(SdkError::Cancelled);
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk_size = u64::try_from(chunk.len()).map_err(|_| {
                SdkError::Migration("HTTP response chunk exceeds u64 size".to_owned())
            })?;
            size_bytes = size_bytes
                .checked_add(chunk_size)
                .ok_or_else(|| SdkError::Migration("download size exceeds u64 range".to_owned()))?;
            file.write_all(&chunk).await.map_err(|source| {
                SdkError::filesystem("write temporary download file", temporary_path, source)
            })?;
            hasher.update(&chunk);
            on_progress(tracker.event(&member.spec, DownloadPhase::Downloading, size_bytes));
        }
        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, size_bytes));
            return Err(SdkError::Cancelled);
        }
        file.flush().await.map_err(|source| {
            SdkError::filesystem("flush temporary download file", temporary_path, source)
        })?;
        file.sync_all().await.map_err(|source| {
            SdkError::filesystem("sync temporary download file", temporary_path, source)
        })?;
        drop(file);

        let integrity = FileIntegrity {
            size_bytes,
            sha256: hex_digest(hasher.finalize()),
        };
        on_progress(tracker.event(&member.spec, DownloadPhase::Verifying, integrity.size_bytes));
        verify_integrity(&member.spec, &integrity)?;
        apply_file_mode(temporary_path, &member.spec).await?;
        Ok(integrity)
    }
}

/// A record is complete when creation committed all four rows. An incomplete
/// row means an interrupted creation, never a third lifecycle state.
fn build_creation_result(stored: &StoredMicroVm) -> MicroVmCreationResult {
    let (network, credential, _) = stored
        .require_full("create MicroVM")
        .expect("creation result requires a complete record");
    MicroVmCreationResult {
        name: stored.record.name.clone(),
        state: MicroVmState::Stopped,
        distribution_id: stored.record.distribution_id.clone(),
        image_id: stored.record.image_id.clone(),
        volume_path: stored.record.volume_path.clone(),
        rootfs_path: stored.record.rootfs_path.clone(),
        socket_path: stored.record.socket_path.clone(),
        vcpu_count: stored.record.vcpu_count,
        memory_bytes: stored.record.memory_bytes,
        disk_size_bytes: stored.record.disk_size_bytes,
        network: network.config.clone(),
        ssh: SshConnectionInfo {
            user: credential.ssh_user.clone(),
            port: credential.ssh_port,
            address: network.config.guest_address,
            private_key_path: credential.private_key_path.clone(),
            public_key_path: credential.public_key_path.clone(),
        },
    }
}

/// Builds the running identity returned by `start_microvm`, including the
/// idempotent already-running path.
fn build_start_result(
    stored: &StoredMicroVm,
    process_id: u32,
) -> Result<MicroVmStartResult, SdkError> {
    let (network, credential, runtime) = stored.require_full("start MicroVM")?;
    let Some(recorded) = runtime.process_id else {
        return Err(SdkError::TemporaryRuntime {
            component: "firecracker".to_owned(),
            reason: "the running VM has no recorded process identifier".to_owned(),
            stopped: false,
        });
    };
    if recorded != process_id {
        return Err(SdkError::TemporaryRuntime {
            component: "firecracker".to_owned(),
            reason: "the recorded process identifier does not match the live machine".to_owned(),
            stopped: false,
        });
    }
    if stored.record.socket_path != stored.record.volume_path.join("firecracker.sock") {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM paths".to_owned(),
            reason: "the control socket must remain inside the VM volume".to_owned(),
        });
    }
    Ok(MicroVmStartResult {
        name: stored.record.name.clone(),
        state: MicroVmState::Running,
        volume_path: stored.record.volume_path.clone(),
        rootfs_path: stored.record.rootfs_path.clone(),
        socket_path: stored.record.socket_path.clone(),
        process_id,
        network: network.config.clone(),
        ssh: SshConnectionInfo {
            user: credential.ssh_user.clone(),
            port: credential.ssh_port,
            address: network.config.guest_address,
            private_key_path: credential.private_key_path.clone(),
            public_key_path: credential.public_key_path.clone(),
        },
    })
}
/// Builds the stopped identity returned by `stop_microvm` on every success
/// path. Requires a complete record so the socket path is trustworthy;
/// `forced` is `true` only when SIGKILL was delivered.
fn build_stop_result(stored: &StoredMicroVm, forced: bool) -> Result<MicroVmStopResult, SdkError> {
    stored.require_complete("stop MicroVM")?;
    if stored.record.socket_path != stored.record.volume_path.join("firecracker.sock") {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM paths".to_owned(),
            reason: "the control socket must remain inside the VM volume".to_owned(),
        });
    }
    Ok(MicroVmStopResult {
        name: stored.record.name.clone(),
        state: MicroVmState::Stopped,
        socket_path: stored.record.socket_path.clone(),
        forced,
    })
}

fn validate_persisted_files(stored: &StoredMicroVm) -> Result<(), SdkError> {
    if !stored.record.volume_path.is_absolute() {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM volume".to_owned(),
            reason: "the configured VM volume path must be absolute".to_owned(),
        });
    }
    let volume_metadata = fs::symlink_metadata(&stored.record.volume_path).map_err(|error| {
        SdkError::filesystem(
            "inspect configured VM volume",
            &stored.record.volume_path,
            error,
        )
    })?;
    if volume_metadata.file_type().is_symlink() || !volume_metadata.is_dir() {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM volume".to_owned(),
            reason: "the configured VM volume is not a real directory".to_owned(),
        });
    }
    if stored.record.rootfs_path != stored.record.volume_path.join("rootfs.ext4")
        || stored.record.socket_path != stored.record.volume_path.join("firecracker.sock")
    {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM paths".to_owned(),
            reason: "rootfs and socket paths must remain inside the VM volume".to_owned(),
        });
    }
    verify_regular_file(&stored.record.rootfs_path, "verify configured rootfs")?;
    let rootfs_size = fs::metadata(&stored.record.rootfs_path)
        .map_err(|error| {
            SdkError::filesystem(
                "inspect configured rootfs",
                &stored.record.rootfs_path,
                error,
            )
        })?
        .len();
    if rootfs_size != stored.record.disk_size_bytes {
        return Err(SdkError::GuestFilesystem {
            operation: "verify configured rootfs size".to_owned(),
            path: stored.record.rootfs_path.clone(),
            reason: format!(
                "expected {} bytes, found {rootfs_size}",
                stored.record.disk_size_bytes
            ),
        });
    }
    let expected_private_key = stored.record.volume_path.join("ssh/id_ed25519");
    let expected_public_key = stored.record.volume_path.join("ssh/id_ed25519.pub");
    let (_, credential, runtime) = stored.require_full("verify persisted files")?;
    if credential.private_key_path != expected_private_key
        || credential.public_key_path != expected_public_key
    {
        return Err(SdkError::Credential {
            operation: "verify configured key paths".to_owned(),
            path: stored.record.volume_path.clone(),
            reason: "credential paths are outside the VM volume".to_owned(),
        });
    }
    verify_regular_file(&credential.private_key_path, "verify private SSH key")?;
    verify_regular_file(&credential.public_key_path, "verify public SSH key")?;
    verify_private_key_mode(&credential.private_key_path)?;
    verify_public_key_mode(&credential.public_key_path)?;
    if credential.key_type != "ed25519"
        || credential.ssh_user != "root"
        || credential.ssh_port != 22
        || credential.guest_authorized_keys_path != "/root/.ssh/authorized_keys"
        || credential.file_mode != "0600"
    {
        return Err(SdkError::Credential {
            operation: "verify configured SSH metadata".to_owned(),
            path: credential.private_key_path.clone(),
            reason: "persisted SSH metadata does not match the SDK contract".to_owned(),
        });
    }
    if runtime.process_state != "stopped"
        || runtime.socket_path != stored.record.socket_path
        || path_entry_exists(&stored.record.socket_path)?
    {
        return Err(SdkError::TemporaryRuntime {
            component: "firecracker.sock".to_owned(),
            reason: "the persisted runtime is not stopped or its socket is still present"
                .to_owned(),
            stopped: false,
        });
    }
    Ok(())
}

/// Validates persisted machine data for a start without assuming the VM is
/// stopped and without treating a leftover socket file as proof of anything.
/// Liveness is decided later by the runtime port; this helper only proves the
/// data is consistent and usable.
fn validate_start_prerequisites(stored: &StoredMicroVm) -> Result<(), SdkError> {
    if !stored.record.volume_path.is_absolute() {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM volume".to_owned(),
            reason: "the configured VM volume path must be absolute".to_owned(),
        });
    }
    let volume_metadata = fs::symlink_metadata(&stored.record.volume_path).map_err(|error| {
        SdkError::filesystem(
            "inspect configured VM volume",
            &stored.record.volume_path,
            error,
        )
    })?;
    if volume_metadata.file_type().is_symlink() || !volume_metadata.is_dir() {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM volume".to_owned(),
            reason: "the configured VM volume is not a real directory".to_owned(),
        });
    }
    if stored.record.rootfs_path != stored.record.volume_path.join("rootfs.ext4")
        || stored.record.socket_path != stored.record.volume_path.join("firecracker.sock")
    {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: stored.record.volume_path.clone(),
            owner: "persisted VM paths".to_owned(),
            reason: "rootfs and socket paths must remain inside the VM volume".to_owned(),
        });
    }
    verify_regular_file(&stored.record.rootfs_path, "verify configured rootfs")?;
    let rootfs_size = fs::metadata(&stored.record.rootfs_path)
        .map_err(|error| {
            SdkError::filesystem(
                "inspect configured rootfs",
                &stored.record.rootfs_path,
                error,
            )
        })?
        .len();
    if rootfs_size != stored.record.disk_size_bytes {
        return Err(SdkError::GuestFilesystem {
            operation: "verify configured rootfs size".to_owned(),
            path: stored.record.rootfs_path.clone(),
            reason: format!(
                "expected {} bytes, found {rootfs_size}",
                stored.record.disk_size_bytes
            ),
        });
    }
    let expected_private_key = stored.record.volume_path.join("ssh/id_ed25519");
    let expected_public_key = stored.record.volume_path.join("ssh/id_ed25519.pub");
    let (_, credential, runtime) = stored.require_full("start MicroVM")?;
    if credential.private_key_path != expected_private_key
        || credential.public_key_path != expected_public_key
    {
        return Err(SdkError::Credential {
            operation: "verify configured key paths".to_owned(),
            path: stored.record.volume_path.clone(),
            reason: "credential paths are outside the VM volume".to_owned(),
        });
    }
    verify_regular_file(&credential.private_key_path, "verify private SSH key")?;
    verify_regular_file(&credential.public_key_path, "verify public SSH key")?;
    verify_private_key_mode(&credential.private_key_path)?;
    verify_public_key_mode(&credential.public_key_path)?;
    if credential.key_type != "ed25519"
        || credential.ssh_user != "root"
        || credential.ssh_port != 22
        || credential.guest_authorized_keys_path != "/root/.ssh/authorized_keys"
        || credential.file_mode != "0600"
    {
        return Err(SdkError::Credential {
            operation: "verify configured SSH metadata".to_owned(),
            path: credential.private_key_path.clone(),
            reason: "persisted SSH metadata does not match the SDK contract".to_owned(),
        });
    }
    if runtime.socket_path != stored.record.socket_path {
        return Err(SdkError::TemporaryRuntime {
            component: "firecracker.sock".to_owned(),
            reason: "the persisted runtime socket does not match the VM volume".to_owned(),
            stopped: false,
        });
    }
    Ok(())
}

/// Reverifies that the kernel and runtime binaries referenced by a start
/// record are still verified in the artifact inventory.
struct StartBootArtifacts {
    kernel_path: PathBuf,
    firecracker_path: PathBuf,
    firectl_path: PathBuf,
}

async fn verify_start_boot_artifacts(
    sdk: &MicroVmSdk,
    stored: &StoredMicroVm,
) -> Result<StartBootArtifacts, SdkError> {
    let kernel_id = stored.record.kernel_id.clone();
    let kernel_local = sdk
        .run_repository(move |repository| repository.resolve_kernel(&kernel_id))
        .await
        .map_err(|error| {
            map_artifact_resolution_error(
                error,
                "kernel",
                &stored.record.kernel_id,
                sdk.home.join("artifacts/kernels"),
            )
        })?;
    verify_regular_file(&kernel_local.path, "verify start kernel")?;
    let firecracker_package_id = stored.record.firecracker_package_id.clone();
    let firecracker = sdk
        .resolve_binary(&firecracker_package_id, "firecracker")
        .await
        .map_err(|error| {
            map_artifact_resolution_error(
                error,
                "runtime binary",
                &firecracker_package_id,
                sdk.home.join("tools/firecracker"),
            )
        })?;
    verify_executable_file(&firecracker.path, "firecracker")?;
    let firectl_package_id = stored.record.firectl_package_id.clone();
    let firectl = sdk
        .resolve_binary(&firectl_package_id, "firectl")
        .await
        .map_err(|error| {
            map_artifact_resolution_error(
                error,
                "runtime binary",
                &firectl_package_id,
                sdk.home.join("tools/firectl"),
            )
        })?;
    verify_executable_file(&firectl.path, "firectl")?;
    Ok(StartBootArtifacts {
        kernel_path: kernel_local.path,
        firecracker_path: firecracker.path,
        firectl_path: firectl.path,
    })
}

fn validate_persisted_network(stored: &StoredMicroVm) -> Result<(), SdkError> {
    let (network, _, _) = stored.require_full("verify persisted network")?;
    let expected_mode = if stored.record.expose_on_lan {
        NetworkMode::Lan
    } else {
        NetworkMode::HostOnly
    };
    if network.config.mode != expected_mode {
        return Err(SdkError::Network {
            mode: expected_mode.to_string(),
            operation: "validate persisted network mode".to_owned(),
            resource: stored.record.name.clone(),
            reason: "the persisted network mode does not match the VM configuration".to_owned(),
        });
    }
    let expected_mac = crate::adapters::network::linux::guest_mac(&stored.record.name);
    if network.guest_mac != expected_mac {
        return Err(SdkError::Network {
            mode: expected_mode.to_string(),
            operation: "validate persisted network identity".to_owned(),
            resource: stored.record.name.clone(),
            reason: "the persisted guest MAC does not match the VM identity".to_owned(),
        });
    }
    let mut resource_kinds = HashSet::new();
    for resource in &network.resources {
        if resource.ownership != "sdk:taumaru" {
            return Err(SdkError::Network {
                mode: expected_mode.to_string(),
                operation: "validate persisted network ownership".to_owned(),
                resource: resource.identity.clone(),
                reason: "a persisted network resource is not owned by this SDK".to_owned(),
            });
        }
        if !resource_kinds.insert(resource.resource) {
            return Err(SdkError::Network {
                mode: expected_mode.to_string(),
                operation: "validate persisted network resources".to_owned(),
                resource: stored.record.name.clone(),
                reason: "the network contains duplicate resource kinds".to_owned(),
            });
        }
    }
    match expected_mode {
        NetworkMode::HostOnly => {
            if !host_only_addresses_are_consistent(network)
                || network.config.prefix_length != 30
                || network.config.gateway != network.host_address
                || network.config.bridge_name.is_some()
                || network.config.uplink_name.is_some()
                || network.dhcp_lease_reference.is_some()
            {
                return Err(SdkError::Network {
                    mode: expected_mode.to_string(),
                    operation: "validate persisted host-only network".to_owned(),
                    resource: stored.record.name.clone(),
                    reason: "host-only network metadata is incomplete or inconsistent".to_owned(),
                });
            }
        }
        NetworkMode::Lan => {
            if network.config.lan_address.is_none()
                || network.config.uplink_name.is_none()
                || network.uplink_cidr.is_none()
                || !host_only_addresses_are_consistent(network)
            {
                return Err(SdkError::Network {
                    mode: expected_mode.to_string(),
                    operation: "validate persisted LAN network".to_owned(),
                    resource: stored.record.name.clone(),
                    reason: "LAN network metadata is incomplete".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn host_only_addresses_are_consistent(network: &PersistedNetwork) -> bool {
    let (Some(IpAddr::V4(host)), IpAddr::V4(guest), Some(IpAddr::V4(gateway))) = (
        network.host_address,
        network.config.guest_address,
        network.config.gateway,
    ) else {
        return false;
    };
    let host = u32::from(host);
    let guest = u32::from(guest);
    let gateway = u32::from(gateway);
    let network_base = host & !3;
    gateway == host
        && guest & !3 == network_base
        && host == network_base + 1
        && guest == network_base + 2
}

fn path_entry_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem("inspect path entry", path, error)),
    }
}

/// Removes the VM's entire private volume directory after verifying it is
/// contained in the SDK home and its owned paths have the expected shapes.
///
/// An already-absent directory counts as already removed. Shapes are checked
/// whether or not the files still exist, so a tampered record can never
/// redirect deletion outside the volume. Violations keep the record: the
/// caller repairs the inventory instead of losing the deletion index.
fn remove_owned_volume(home: &Path, stored: &StoredMicroVm) -> Result<(), SdkError> {
    let volume_path = &stored.record.volume_path;
    if !volume_path.is_absolute() || !path_is_below_home(home, volume_path) || volume_path == home {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: volume_path.clone(),
            owner: "persisted VM volume".to_owned(),
            reason: "the persisted VM volume must be a directory strictly below the SDK home"
                .to_owned(),
        });
    }
    if stored.record.rootfs_path != volume_path.join("rootfs.ext4")
        || stored.record.socket_path != volume_path.join("firecracker.sock")
    {
        return Err(SdkError::StorageConflict {
            vm_name: stored.record.name.clone(),
            volume_path: volume_path.clone(),
            owner: "persisted VM paths".to_owned(),
            reason: "rootfs and socket paths must remain inside the VM volume".to_owned(),
        });
    }
    if let Some(credential) = stored.credential.as_ref() {
        let expected_private_key = volume_path.join("ssh/id_ed25519");
        let expected_public_key = volume_path.join("ssh/id_ed25519.pub");
        if credential.private_key_path != expected_private_key
            || credential.public_key_path != expected_public_key
        {
            return Err(SdkError::Credential {
                operation: "verify owned key paths".to_owned(),
                path: volume_path.clone(),
                reason: "credential paths are outside the VM volume".to_owned(),
            });
        }
    }
    match fs::remove_dir_all(volume_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SdkError::filesystem("remove VM volume", volume_path, error)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn verify_local_artifact_with_progress(
    home: &Path,
    path: &Path,
    inventory_size: u64,
    inventory_sha256: &str,
    expected_size: u64,
    expected_sha256: &str,
    kind: &str,
    id: &str,
    on_read_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), SdkError> {
    if !path_is_below_home(home, path) {
        return Err(SdkError::ArtifactPrerequisite {
            kind: kind.to_owned(),
            id: id.to_owned(),
            path: path.to_path_buf(),
            reason: "the inventory path escapes the SDK home".to_owned(),
        });
    }
    if inventory_size != expected_size || inventory_sha256 != expected_sha256 {
        return Err(SdkError::ArtifactPrerequisite {
            kind: kind.to_owned(),
            id: id.to_owned(),
            path: path.to_path_buf(),
            reason: "the local inventory does not match the selected registry metadata".to_owned(),
        });
    }
    let Some(integrity) =
        calculate_file_integrity_with_progress(path, expected_size.max(1), on_read_progress)
            .await?
    else {
        return Err(SdkError::ArtifactPrerequisite {
            kind: kind.to_owned(),
            id: id.to_owned(),
            path: path.to_path_buf(),
            reason: "the downloaded file is missing, not regular, or is a symlink".to_owned(),
        });
    };
    if integrity.size_bytes != expected_size || integrity.sha256 != expected_sha256 {
        return Err(SdkError::IntegrityMismatch {
            artifact: format!("{kind}:{id}"),
            expected_size,
            actual_size: integrity.size_bytes,
            expected_sha256: expected_sha256.to_owned(),
            actual_sha256: integrity.sha256,
        });
    }
    Ok(())
}

fn map_artifact_resolution_error(error: SdkError, kind: &str, id: &str, path: PathBuf) -> SdkError {
    match error {
        SdkError::NotFound { .. } | SdkError::StaleBinary { .. } => {
            SdkError::ArtifactPrerequisite {
                kind: kind.to_owned(),
                id: id.to_owned(),
                path,
                reason: error.to_string(),
            }
        }
        other => other,
    }
}

fn map_insert_creation_error(error: SdkError, vm_name: &str, volume_path: &Path) -> SdkError {
    match error {
        SdkError::Sqlite(source) if source.to_string().contains("UNIQUE") => {
            SdkError::StorageConflict {
                vm_name: vm_name.to_owned(),
                volume_path: volume_path.to_path_buf(),
                owner: "another concurrent creation attempt".to_owned(),
                reason: "the VM name or volume path was claimed concurrently".to_owned(),
            }
        }
        other => other,
    }
}

fn verify_regular_file(path: &Path, operation: &'static str) -> Result<(), SdkError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| SdkError::filesystem(operation, path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: path.to_path_buf(),
            reason: "the path is not a regular file".to_owned(),
        });
    }
    Ok(())
}

fn verify_executable_file(path: &Path, component: &str) -> Result<(), SdkError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| SdkError::RuntimeIncompatible {
        component: component.to_owned(),
        package_id: "local".to_owned(),
        version: "unknown".to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        reason: format!("could not inspect executable: {error}"),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SdkError::RuntimeIncompatible {
            component: component.to_owned(),
            package_id: "local".to_owned(),
            version: "unknown".to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            reason: "the selected path is not a regular file".to_owned(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(SdkError::RuntimeIncompatible {
                component: component.to_owned(),
                package_id: "local".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: "the selected path is not executable".to_owned(),
            });
        }
    }
    Ok(())
}

fn verify_private_key_mode(path: &Path) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::symlink_metadata(path)
            .map_err(|error| SdkError::filesystem("inspect private SSH key", path, error))?;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(SdkError::Credential {
                operation: "verify private SSH key permissions".to_owned(),
                path: path.to_path_buf(),
                reason: "the private key must have mode 0600".to_owned(),
            });
        }
    }
    Ok(())
}

fn verify_public_key_mode(path: &Path) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::symlink_metadata(path)
            .map_err(|error| SdkError::filesystem("inspect public SSH key", path, error))?;
        if metadata.permissions().mode() & 0o777 != 0o644 {
            return Err(SdkError::Credential {
                operation: "verify public SSH key permissions".to_owned(),
                path: path.to_path_buf(),
                reason: "the public key must have mode 0644".to_owned(),
            });
        }
    }
    Ok(())
}

fn host_architecture() -> Result<Architecture, SdkError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(Architecture::X86_64),
        "aarch64" => Ok(Architecture::Aarch64),
        "arm" | "armv7" | "armv7a" => Ok(Architecture::Arm),
        "riscv64" => Ok(Architecture::Riscv64),
        "x86" | "i686" => Ok(Architecture::X86),
        architecture => Err(SdkError::RuntimeIncompatible {
            component: "host".to_owned(),
            package_id: "host".to_owned(),
            version: "unknown".to_owned(),
            architecture: architecture.to_owned(),
            reason: "the host architecture is not supported by this SDK".to_owned(),
        }),
    }
}

fn architecture_name(architecture: &Architecture) -> &'static str {
    match architecture {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::Riscv64 => "riscv64",
        Architecture::X86 => "x86",
    }
}

fn unix_timestamp() -> Result<i64, SdkError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            SdkError::Migration(format!("system clock is before Unix epoch: {error}"))
        })?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| SdkError::Migration("system clock exceeds SQLite timestamp range".to_owned()))
}

struct DownloadSpecInput {
    artifact_kind: ArtifactKind,
    artifact_id: String,
    member_name: Option<String>,
    artifact_key: String,
    registry_path: String,
    registry_url: String,
    filename: String,
    expected_size: u64,
    expected_sha256: String,
    executable: bool,
    mode: Option<String>,
    relative_path: PathBuf,
}

fn make_download_spec(home: &Path, input: DownloadSpecInput) -> Result<DownloadSpec, SdkError> {
    let DownloadSpecInput {
        artifact_kind,
        artifact_id,
        member_name,
        artifact_key,
        registry_path,
        registry_url,
        filename,
        expected_size,
        expected_sha256,
        executable,
        mode,
        relative_path,
    } = input;
    if !validate_registry_path(&registry_path) {
        return Err(SdkError::invalid_metadata(
            &artifact_key,
            "registry path is not a safe relative path",
        ));
    }
    validate_safe_component(&artifact_id, "artifact identifier")?;
    if let Some(member_name) = &member_name {
        validate_safe_component(member_name, "artifact member identifier")?;
    }
    validate_safe_component(&filename, "artifact filename")?;
    if !is_valid_sha256(&expected_sha256) {
        return Err(SdkError::invalid_metadata(
            &artifact_key,
            "SHA-256 must be 64 lowercase hexadecimal characters",
        ));
    }
    if relative_path.is_absolute()
        || !relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(SdkError::invalid_metadata(
            &artifact_key,
            "managed relative path is unsafe",
        ));
    }
    let absolute_path = home.join(&relative_path);
    if !path_is_below_home(home, &absolute_path) {
        return Err(SdkError::invalid_metadata(
            &artifact_key,
            "managed path escapes SDK home",
        ));
    }
    Ok(DownloadSpec {
        artifact_kind,
        artifact_id,
        member_name,
        artifact_key,
        registry_path,
        registry_url,
        filename,
        expected_size,
        expected_sha256,
        executable,
        mode,
        relative_path,
        absolute_path,
    })
}

fn validate_safe_component(value: &str, kind: &str) -> Result<(), SdkError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(SdkError::invalid_metadata(
            value,
            format!("{kind} is not a safe path component"),
        ));
    }
    Ok(())
}

fn total_size(members: &[DownloadMember], artifact_id: &str) -> Result<u64, SdkError> {
    members.iter().try_fold(0_u64, |total, member| {
        total.checked_add(member.spec.expected_size).ok_or_else(|| {
            SdkError::invalid_metadata(artifact_id, "aggregate download size exceeds u64 range")
        })
    })
}

fn downloaded_file(
    member: &DownloadMember,
    integrity: FileIntegrity,
    disposition: DownloadDisposition,
) -> DownloadedFile {
    DownloadedFile {
        artifact_kind: member.spec.artifact_kind.clone(),
        artifact_id: member.spec.artifact_id.clone(),
        member_name: member.spec.member_name.clone(),
        absolute_path: member.spec.absolute_path.clone(),
        relative_path: member.spec.relative_path.clone(),
        size_bytes: integrity.size_bytes,
        sha256: integrity.sha256,
        disposition,
    }
}

async fn calculate_file_integrity(path: &Path) -> Result<Option<FileIntegrity>, SdkError> {
    let metadata = match async_fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SdkError::filesystem("inspect artifact file", path, source));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let mut file = async_fs::File::open(path)
        .await
        .map_err(|source| SdkError::filesystem("open artifact file", path, source))?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let bytes_read = file
            .read(&mut buffer)
            .await
            .map_err(|source| SdkError::filesystem("read artifact file", path, source))?;
        if bytes_read == 0 {
            break;
        }
        let bytes_read = u64::try_from(bytes_read)
            .map_err(|_| SdkError::Migration("file read size exceeds u64 range".to_owned()))?;
        size_bytes = size_bytes.checked_add(bytes_read).ok_or_else(|| {
            SdkError::Migration("artifact file size exceeds u64 range".to_owned())
        })?;
        hasher.update(
            &buffer[..usize::try_from(bytes_read).map_err(|_| {
                SdkError::Migration("file read size cannot be represented as usize".to_owned())
            })?],
        );
    }
    Ok(Some(FileIntegrity {
        size_bytes,
        sha256: hex_digest(hasher.finalize()),
    }))
}
async fn calculate_file_integrity_with_progress(
    path: &Path,
    expected_bytes: u64,
    on_read_progress: &mut dyn FnMut(u64, u64),
) -> Result<Option<FileIntegrity>, SdkError> {
    let metadata = match async_fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SdkError::filesystem("inspect artifact file", path, source));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let mut file = async_fs::File::open(path)
        .await
        .map_err(|source| SdkError::filesystem("open artifact file", path, source))?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    let expected = expected_bytes.max(1);
    loop {
        let bytes_read = file
            .read(&mut buffer)
            .await
            .map_err(|source| SdkError::filesystem("read artifact file", path, source))?;
        if bytes_read == 0 {
            break;
        }
        let bytes_read = u64::try_from(bytes_read)
            .map_err(|_| SdkError::Migration("file read size exceeds u64 range".to_owned()))?;
        size_bytes = size_bytes.checked_add(bytes_read).ok_or_else(|| {
            SdkError::Migration("artifact file size exceeds u64 range".to_owned())
        })?;
        hasher.update(
            &buffer[..usize::try_from(bytes_read).map_err(|_| {
                SdkError::Migration("file read size cannot be represented as usize".to_owned())
            })?],
        );
        on_read_progress(size_bytes.min(expected), expected);
    }
    on_read_progress(expected, expected);
    Ok(Some(FileIntegrity {
        size_bytes,
        sha256: hex_digest(hasher.finalize()),
    }))
}

async fn remove_invalid_target_if_exists(path: &Path) -> Result<(), SdkError> {
    let metadata = match async_fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(SdkError::filesystem(
                "inspect invalid artifact target",
                path,
                source,
            ));
        }
    };
    let result = if metadata.file_type().is_dir() {
        async_fs::remove_dir_all(path).await
    } else {
        async_fs::remove_file(path).await
    };
    result.map_err(|source| SdkError::filesystem("remove invalid artifact target", path, source))
}

fn temporary_path(home: &Path, artifact_key: &str) -> Result<PathBuf, SdkError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            SdkError::Migration(format!("system clock is before Unix epoch: {error}"))
        })?
        .as_nanos();
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(artifact_key.as_bytes());
    let key_digest = hex_digest(hasher.finalize());
    Ok(home
        .join("tmp")
        .join(format!(".{key_digest}.{timestamp}.{sequence}.part")))
}

fn verify_integrity(spec: &DownloadSpec, integrity: &FileIntegrity) -> Result<(), SdkError> {
    if integrity.size_bytes != spec.expected_size || integrity.sha256 != spec.expected_sha256 {
        return Err(SdkError::IntegrityMismatch {
            artifact: spec.artifact_key.clone(),
            expected_size: spec.expected_size,
            actual_size: integrity.size_bytes,
            expected_sha256: spec.expected_sha256.clone(),
            actual_sha256: integrity.sha256.clone(),
        });
    }
    Ok(())
}

async fn apply_file_mode(path: &Path, spec: &DownloadSpec) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = match spec.mode.as_deref() {
            Some(mode) => u32::from_str_radix(mode, 8).map_err(|_| {
                SdkError::invalid_metadata(&spec.artifact_key, "file mode is not valid octal")
            })?,
            None if spec.executable => 0o755,
            None => return Ok(()),
        };
        let permissions = std::fs::Permissions::from_mode(mode);
        async_fs::set_permissions(path, permissions)
            .await
            .map_err(|source| SdkError::filesystem("apply artifact file mode", path, source))?;
    }
    #[cfg(not(unix))]
    if spec.executable || spec.mode.is_some() {
        return Err(SdkError::invalid_metadata(
            &spec.artifact_key,
            "executable artifact modes are unsupported on this host",
        ));
    }
    Ok(())
}

fn validate_requested_id(value: &str, kind: &str) -> Result<(), SdkError> {
    validate_safe_component(value, kind)
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn normalize_home_path(path: &Path) -> Result<PathBuf, SdkError> {
    if path.as_os_str().is_empty() {
        return Err(SdkError::InvalidHome {
            path: path.to_path_buf(),
            reason: "path is empty".to_owned(),
        });
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| SdkError::filesystem("get current directory", path, error))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized != Path::new(std::path::MAIN_SEPARATOR_STR) {
                    normalized.pop();
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(SdkError::InvalidHome {
            path: path.to_path_buf(),
            reason: "path resolves to an empty location".to_owned(),
        });
    }
    Ok(normalized)
}

pub(crate) fn path_is_below_home(home: &Path, candidate: &Path) -> bool {
    candidate.starts_with(home)
}

fn create_managed_directories(home: &Path) -> Result<(), SdkError> {
    const DIRECTORIES: &[&str] = &[
        "state",
        "artifacts/kernels",
        "artifacts/rootfs",
        "artifacts/supporting",
        "tools/firecracker",
        "tools/firectl",
        "vms",
        "runtime",
        "cache",
        "tmp",
    ];
    for relative in DIRECTORIES {
        let path = home.join(relative);
        fs::create_dir_all(&path)
            .map_err(|error| SdkError::filesystem("create SDK directory", path, error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity};
    use crate::domain::lifecycle::{MicroVmState, NetworkMode};
    use crate::domain::microvm::{
        CreateMicroVmRequest, CreationEventPhase, CreationOutcome, CreationProgress, CreationStage,
        NetworkConfiguration, NetworkResource, PersistedCredential, PersistedNetwork,
        PersistedNetworkResource, PersistedRuntime, TOTAL_CREATION_STEPS,
    };
    use crate::domain::registry::TaumaruRegistry;
    use crate::error::SdkError;
    use crate::ports::artifacts::{ArtifactSource, RegistryFuture};
    use crate::ports::credentials::{CredentialStore, GeneratedCredential};
    use crate::ports::network::{
        LanAddressOffer, NetworkController, NetworkOutcome, NetworkRequest, UplinkIdentity,
    };
    use crate::ports::repository::InventoryState;
    use crate::ports::runtime::RuntimeController;
    use crate::ports::storage::{GuestStorage, PreparedRootfs};

    use super::{
        CacheDecision, MicroVmSdk, decide_cache, hex_digest, normalize_home_path,
        path_is_below_home,
    };

    #[test]
    fn normalizes_a_caller_path_without_consulting_environment() {
        let directory = tempdir().expect("temporary directory should be created");
        let requested = directory.path().join("nested").join("..").join("sdk");
        let normalized = normalize_home_path(&requested).expect("path should be normalized");

        assert_eq!(normalized, directory.path().join("sdk"));
    }

    #[test]
    fn detects_paths_outside_the_normalized_sdk_home() {
        let directory = tempdir().expect("temporary directory should be created");
        let home = directory.path().join("sdk");
        let inside = home.join("artifacts").join("file");
        let outside = directory.path().join("outside");

        assert!(path_is_below_home(&home, &inside));
        assert!(!path_is_below_home(&home, &outside));
    }

    #[test]
    fn rejects_an_empty_home_path() {
        assert!(normalize_home_path(Path::new("")).is_err());
    }

    fn spec() -> DownloadSpec {
        DownloadSpec {
            artifact_kind: ArtifactKind::Kernel,
            artifact_id: "kernel".to_owned(),
            member_name: None,
            artifact_key: "kernel:kernel".to_owned(),
            registry_path: "kernels/kernel/vmlinux".to_owned(),
            registry_url: "https://example.invalid/vmlinux".to_owned(),
            filename: "vmlinux".to_owned(),
            expected_size: 4,
            expected_sha256: "a".repeat(64),
            executable: false,
            mode: None,
            relative_path: PathBuf::from("artifacts/kernels/kernel/vmlinux"),
            absolute_path: PathBuf::from("/tmp/sdk/artifacts/kernels/kernel/vmlinux"),
        }
    }

    #[test]
    fn cache_decision_trusts_complete_inventory_without_hashing() {
        let spec = spec();
        let correct = FileIntegrity {
            size_bytes: 4,
            sha256: "a".repeat(64),
        };

        // Complete inventory reuses without hashing: even no physical proof Skips.
        assert_eq!(
            decide_cache(&spec, None, InventoryState::Complete),
            CacheDecision::Skip
        );
        assert_eq!(
            decide_cache(&spec, Some(&correct), InventoryState::Complete),
            CacheDecision::Skip
        );
        assert_eq!(
            decide_cache(&spec, Some(&correct), InventoryState::Incomplete),
            CacheDecision::Adopt
        );
        assert_eq!(
            decide_cache(&spec, Some(&correct), InventoryState::Missing),
            CacheDecision::Adopt
        );
    }

    #[test]
    fn cache_decision_replaces_missing_or_wrong_files() {
        let spec = spec();
        let wrong = FileIntegrity {
            size_bytes: 3,
            sha256: "b".repeat(64),
        };

        assert_eq!(
            decide_cache(&spec, None, InventoryState::Missing),
            CacheDecision::Replace
        );
        assert_eq!(
            decide_cache(&spec, Some(&wrong), InventoryState::Incomplete),
            CacheDecision::Replace
        );
        assert_eq!(
            decide_cache(&spec, Some(&wrong), InventoryState::Complete),
            CacheDecision::Skip
        );
    }

    struct TestArtifactSource {
        manifest: TaumaruRegistry,
    }

    impl ArtifactSource for TestArtifactSource {
        fn fetch_manifest(&self) -> RegistryFuture<'_, TaumaruRegistry> {
            let manifest = self.manifest.clone();
            Box::pin(async move { Ok(manifest) })
        }

        fn fetch_file(&self, url: &str) -> RegistryFuture<'_, reqwest::Response> {
            let url = url.to_owned();
            Box::pin(async move { Err(SdkError::InvalidUrl { url }) })
        }
    }

    #[derive(Default)]
    struct TestStorage {
        prepare_calls: AtomicUsize,
        inject_calls: AtomicUsize,
    }

    impl GuestStorage for TestStorage {
        fn prepare_rootfs(
            &self,
            source: &Path,
            volume_path: &Path,
            requested_size_bytes: u64,
            on_copy_progress: &mut dyn FnMut(u64, u64),
        ) -> Result<PreparedRootfs, SdkError> {
            self.prepare_calls.fetch_add(1, Ordering::Relaxed);
            let created_volume = !volume_path.exists();
            fs::create_dir_all(volume_path).map_err(|error| {
                SdkError::filesystem("create test VM volume", volume_path, error)
            })?;
            let rootfs_path = volume_path.join("rootfs.ext4");
            let source_file = fs::File::open(source)
                .map_err(|error| SdkError::filesystem("open test source rootfs", source, error))?;
            let source_len = source_file
                .metadata()
                .map_err(|error| SdkError::filesystem("inspect test source rootfs", source, error))?
                .len();
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&rootfs_path)
                .map_err(|error| SdkError::filesystem("create test rootfs", &rootfs_path, error))?;
            let mut copied = 0_u64;
            let mut reader = std::io::BufReader::new(source_file);
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                let read = std::io::Read::read(&mut reader, &mut buffer).map_err(|error| {
                    SdkError::filesystem("copy test rootfs", &rootfs_path, error)
                })?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut output, &buffer[..read]).map_err(|error| {
                    SdkError::filesystem("copy test rootfs", &rootfs_path, error)
                })?;
                copied += read as u64;
                on_copy_progress(copied.min(source_len), requested_size_bytes.max(source_len));
            }
            output
                .set_len(requested_size_bytes)
                .map_err(|error| SdkError::filesystem("resize test rootfs", &rootfs_path, error))?;
            on_copy_progress(requested_size_bytes, requested_size_bytes);
            Ok(PreparedRootfs {
                path: rootfs_path,
                created_volume,
            })
        }

        fn inject_public_key(&self, rootfs_path: &Path, _public_key: &str) -> Result<(), SdkError> {
            self.inject_calls.fetch_add(1, Ordering::Relaxed);
            if !rootfs_path.is_file() {
                return Err(SdkError::GuestFilesystem {
                    operation: "inject test public key".to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: "test rootfs is missing".to_owned(),
                });
            }
            Ok(())
        }

        fn write_guest_network_config(
            &self,
            rootfs_path: &Path,
            _guest_address: std::net::Ipv4Addr,
            _gateway: std::net::Ipv4Addr,
        ) -> Result<(), SdkError> {
            if !rootfs_path.is_file() {
                return Err(SdkError::GuestFilesystem {
                    operation: "write test network unit".to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: "test rootfs is missing".to_owned(),
                });
            }
            Ok(())
        }

        fn write_guest_lan_config(
            &self,
            rootfs_path: &Path,
            _guest_address: std::net::Ipv4Addr,
            _gateway: std::net::Ipv4Addr,
            _lan_address: std::net::Ipv4Addr,
        ) -> Result<(), SdkError> {
            if !rootfs_path.is_file() {
                return Err(SdkError::GuestFilesystem {
                    operation: "write test LAN unit".to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: "test rootfs is missing".to_owned(),
                });
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestCredentials {
        generate_calls: AtomicUsize,
    }

    impl CredentialStore for TestCredentials {
        fn generate(&self, volume_path: &Path) -> Result<GeneratedCredential, SdkError> {
            self.generate_calls.fetch_add(1, Ordering::Relaxed);
            let ssh_directory = volume_path.join("ssh");
            fs::create_dir_all(&ssh_directory).map_err(|error| {
                SdkError::filesystem("create test SSH directory", &ssh_directory, error)
            })?;
            let private_key_path = ssh_directory.join("id_ed25519");
            let public_key_path = ssh_directory.join("id_ed25519.pub");
            fs::write(&private_key_path, b"test private key\n").map_err(|error| {
                SdkError::filesystem("write test private key", &private_key_path, error)
            })?;
            fs::write(
                &public_key_path,
                b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKEKEY\n",
            )
            .map_err(|error| {
                SdkError::filesystem("write test public key", &public_key_path, error)
            })?;
            set_test_mode(&private_key_path, 0o600)?;
            set_test_mode(&public_key_path, 0o644)?;
            Ok(GeneratedCredential {
                private_key_path,
                public_key_path,
                public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKEKEY\n".to_owned(),
                fingerprint: "SHA256:test".to_owned(),
                file_mode: "0600".to_owned(),
            })
        }
    }

    fn set_test_mode(path: &Path, mode: u32) -> Result<(), SdkError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .map_err(|error| SdkError::filesystem("set test file mode", path, error))?;
        }
        #[cfg(not(unix))]
        let _ = (path, mode);
        Ok(())
    }

    struct TestNetwork {
        cleanup_calls: AtomicUsize,
        delete_cleanup_calls: AtomicUsize,
        delete_cleanup_results: std::sync::Mutex<Vec<Result<(), String>>>,
    }

    impl NetworkController for TestNetwork {
        fn detect_uplink(&self) -> Result<UplinkIdentity, SdkError> {
            Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "detect test uplink".to_owned(),
                resource: "test".to_owned(),
                reason: "the deterministic test adapter has no uplink".to_owned(),
            })
        }

        fn select_lan_offer(
            &self,
            _uplink: &UplinkIdentity,
            _lan_override: Option<Ipv4Addr>,
            _previous: Option<Ipv4Addr>,
            _used_addresses: &[(String, IpAddr, String)],
        ) -> Result<LanAddressOffer, SdkError> {
            Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "select test LAN offer".to_owned(),
                resource: "test".to_owned(),
                reason: "the deterministic test adapter has no LAN offer".to_owned(),
            })
        }

        fn configure(
            &self,
            request: &NetworkRequest,
            existing: Option<&PersistedNetwork>,
            _used_addresses: &[(String, IpAddr, String)],
            _used_lan_addresses: &[(String, IpAddr, String)],
        ) -> Result<NetworkOutcome, SdkError> {
            if let Some(network) = existing {
                return Ok(NetworkOutcome {
                    persisted: network.clone(),
                    applied: Vec::new(),
                    skipped: network.resources.iter().map(|item| item.resource).collect(),
                });
            }
            match request.mode {
                NetworkMode::HostOnly => test_configure_host_only(request),
                NetworkMode::Lan => test_configure_lan(request),
            }
        }

        fn cleanup(&self, _network: &PersistedNetwork) -> Result<(), SdkError> {
            self.cleanup_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn cleanup_for_delete(&self, _network: &PersistedNetwork) -> Result<(), SdkError> {
            self.delete_cleanup_calls.fetch_add(1, Ordering::Relaxed);
            match self
                .delete_cleanup_results
                .lock()
                .expect("test delete cleanup lock")
                .pop()
            {
                Some(Ok(())) | None => Ok(()),
                Some(Err(reason)) => Err(SdkError::Network {
                    mode: NetworkMode::HostOnly.to_string(),
                    operation: "release test network".to_owned(),
                    resource: "test".to_owned(),
                    reason,
                }),
            }
        }
    }

    fn test_configure_host_only(request: &NetworkRequest) -> Result<NetworkOutcome, SdkError> {
        let digest = Sha256::digest(request.vm_name.as_bytes());
        let third_octet = digest[0] & 0xfc;
        let host = Ipv4Addr::new(10, 200, third_octet, 1);
        let guest = Ipv4Addr::new(10, 200, third_octet, 2);
        let tap_name = format!("tap-{}", request.vm_name);
        let config = NetworkConfiguration {
            mode: NetworkMode::HostOnly,
            guest_address: IpAddr::V4(guest),
            prefix_length: 30,
            gateway: Some(IpAddr::V4(host)),
            tap_name: tap_name.clone(),
            bridge_name: None,
            uplink_name: None,
            lan_address: None,
        };
        let resources = [
            (NetworkResource::Tap, tap_name.clone()),
            (NetworkResource::TapAddress, format!("{tap_name}:{host}/30")),
            (
                NetworkResource::Forwarding,
                "net.ipv4.ip_forward".to_owned(),
            ),
            (NetworkResource::ForwardRule, format!("{tap_name}:forward")),
            (NetworkResource::IptablesNat, tap_name.clone()),
            (
                NetworkResource::FirecrackerInterface,
                format!("{tap_name}:{}", request.guest_mac),
            ),
        ]
        .into_iter()
        .map(|(resource, identity)| PersistedNetworkResource {
            resource,
            identity: identity.clone(),
            fingerprint: identity,
            ownership: "sdk:taumaru".to_owned(),
            adapter_handle: None,
            last_observed: "desired".to_owned(),
        })
        .collect();
        Ok(NetworkOutcome {
            persisted: PersistedNetwork {
                config,
                host_address: Some(IpAddr::V4(host)),
                guest_mac: request.guest_mac.clone(),
                dhcp_lease_reference: None,
                desired_boot_parameters: format!("ip={guest}::{host}:255.255.255.252::eth0:off"),
                resources,
                uplink_cidr: None,
                proxy_arp_enabled_by_sdk: false,
                bridge_created_by_sdk: false,
                uplink_attached_by_sdk: false,
                forwarding_enabled_by_sdk: true,
                nat_table_created_by_sdk: false,
                nat_chain_created_by_sdk: false,
                host_route_created_by_sdk: false,
                proxy_arp_entry_created_by_sdk: false,
                host_address_specs: Vec::new(),
                default_route_specs: Vec::new(),
            },
            applied: vec![
                NetworkResource::Tap,
                NetworkResource::TapAddress,
                NetworkResource::Forwarding,
                NetworkResource::ForwardRule,
                NetworkResource::IptablesNat,
                NetworkResource::FirecrackerInterface,
            ],
            skipped: Vec::new(),
        })
    }

    fn test_configure_lan(request: &NetworkRequest) -> Result<NetworkOutcome, SdkError> {
        let digest = Sha256::digest(request.vm_name.as_bytes());
        let third_octet = digest[0] & 0xfc;
        let host = Ipv4Addr::new(10, 200, third_octet, 1);
        let guest = Ipv4Addr::new(10, 200, third_octet, 2);
        let lan = request
            .lan_address_override
            .unwrap_or_else(|| Ipv4Addr::new(192, 168, 3, 50 + (digest[1] % 100)));
        let tap_name = format!("tap-{}", request.vm_name);
        let uplink = "test-uplink";
        let config = NetworkConfiguration {
            mode: NetworkMode::Lan,
            guest_address: IpAddr::V4(guest),
            prefix_length: 30,
            gateway: Some(IpAddr::V4(host)),
            tap_name: tap_name.clone(),
            bridge_name: None,
            uplink_name: Some(uplink.to_owned()),
            lan_address: Some(IpAddr::V4(lan)),
        };
        let resources = [
            (NetworkResource::Tap, tap_name.clone()),
            (NetworkResource::TapAddress, format!("{tap_name}:{host}/30")),
            (NetworkResource::HostRoute, format!("{tap_name}:{lan}")),
            (NetworkResource::ProxyArpEntry, format!("{uplink}:{lan}")),
            (
                NetworkResource::Forwarding,
                "net.ipv4.ip_forward".to_owned(),
            ),
            (NetworkResource::ForwardRule, format!("{tap_name}:forward")),
            (NetworkResource::IptablesNat, tap_name.clone()),
            (
                NetworkResource::FirecrackerInterface,
                format!("{tap_name}:{}", request.guest_mac),
            ),
        ]
        .into_iter()
        .map(|(resource, identity)| PersistedNetworkResource {
            resource,
            identity: identity.clone(),
            fingerprint: identity,
            ownership: "sdk:taumaru".to_owned(),
            adapter_handle: None,
            last_observed: "desired".to_owned(),
        })
        .collect();
        Ok(NetworkOutcome {
            persisted: PersistedNetwork {
                config,
                host_address: Some(IpAddr::V4(host)),
                guest_mac: request.guest_mac.clone(),
                dhcp_lease_reference: None,
                desired_boot_parameters: format!("ip={guest}::{host}:255.255.255.252::eth0:off"),
                resources,
                uplink_cidr: Some("192.168.3.0/24".to_owned()),
                proxy_arp_enabled_by_sdk: true,
                bridge_created_by_sdk: false,
                uplink_attached_by_sdk: false,
                forwarding_enabled_by_sdk: true,
                nat_table_created_by_sdk: false,
                nat_chain_created_by_sdk: false,
                host_route_created_by_sdk: true,
                proxy_arp_entry_created_by_sdk: true,
                host_address_specs: Vec::new(),
                default_route_specs: Vec::new(),
            },
            applied: vec![
                NetworkResource::Tap,
                NetworkResource::TapAddress,
                NetworkResource::HostRoute,
                NetworkResource::ProxyArpEntry,
                NetworkResource::Forwarding,
                NetworkResource::ForwardRule,
                NetworkResource::IptablesNat,
                NetworkResource::FirecrackerInterface,
            ],
            skipped: Vec::new(),
        })
    }

    struct TestRuntime {
        verify_calls: AtomicUsize,
        fail_verify: bool,
        live_process: std::sync::Mutex<bool>,
        live_socket: std::sync::Mutex<bool>,
        socket_answers: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>>,
        socket_errors: std::sync::Mutex<std::collections::HashSet<String>>,
        launched: std::sync::Mutex<Vec<crate::ports::runtime::StartRequest>>,
        launch_result: std::sync::Mutex<Result<u32, String>>,
        ready_result: std::sync::Mutex<Result<bool, String>>,
        fail_host: std::sync::Mutex<bool>,
        shutdown_calls: AtomicUsize,
        shutdown_results: std::sync::Mutex<std::collections::HashMap<String, Result<bool, String>>>,
        stop_results: std::sync::Mutex<Vec<Result<bool, String>>>,
        process_answers: std::sync::Mutex<Option<std::collections::HashMap<u32, bool>>>,
        terminate_calls: std::sync::Mutex<Vec<u32>>,
        terminate_result: std::sync::Mutex<Result<(), String>>,
    }

    impl RuntimeController for TestRuntime {
        fn validate_host(&self) -> Result<(), SdkError> {
            if *self.fail_host.lock().expect("test host lock") {
                return Err(SdkError::RuntimeIncompatible {
                    component: "test-host".to_owned(),
                    package_id: "test".to_owned(),
                    version: "test".to_owned(),
                    architecture: std::env::consts::ARCH.to_owned(),
                    reason: "injected host incompatibility".to_owned(),
                });
            }
            Ok(())
        }

        fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError> {
            self.verify_calls.fetch_add(1, Ordering::Relaxed);
            if self.fail_verify {
                return Err(SdkError::TemporaryRuntime {
                    component: "test-runtime".to_owned(),
                    reason: "injected stopped-state failure".to_owned(),
                    stopped: false,
                });
            }
            if fs::symlink_metadata(socket_path).is_ok() {
                return Err(SdkError::TemporaryRuntime {
                    component: "test-socket".to_owned(),
                    reason: "test socket is present".to_owned(),
                    stopped: false,
                });
            }
            Ok(())
        }

        fn process_references_vm(
            &self,
            process_id: u32,
            _socket_path: &Path,
            _firecracker_path: &Path,
        ) -> Result<bool, SdkError> {
            if let Some(answer) = self
                .process_answers
                .lock()
                .expect("test process map lock")
                .as_ref()
                .and_then(|answers| answers.get(&process_id).copied())
            {
                return Ok(answer);
            }
            Ok(*self.live_process.lock().expect("test liveness lock"))
        }

        fn socket_answers(&self, socket_path: &Path) -> Result<bool, SdkError> {
            let key = socket_path.to_string_lossy().into_owned();
            if self
                .socket_errors
                .lock()
                .expect("test socket error lock")
                .contains(&key)
            {
                return Err(SdkError::TemporaryRuntime {
                    component: "test-socket".to_owned(),
                    reason: "injected socket failure".to_owned(),
                    stopped: true,
                });
            }
            if let Some(answer) = self
                .socket_answers
                .lock()
                .expect("test socket map lock")
                .as_ref()
                .and_then(|answers| answers.get(&key).copied())
            {
                return Ok(answer);
            }
            Ok(*self.live_socket.lock().expect("test socket lock"))
        }

        fn launch_detached(
            &self,
            request: &crate::ports::runtime::StartRequest,
        ) -> Result<u32, SdkError> {
            self.launched
                .lock()
                .expect("test launch lock")
                .push(request.clone());
            match self
                .launch_result
                .lock()
                .expect("test launch result lock")
                .clone()
            {
                Ok(pid) => Ok(pid),
                Err(reason) => Err(SdkError::TemporaryRuntime {
                    component: "test-runtime".to_owned(),
                    reason,
                    stopped: true,
                }),
            }
        }

        fn wait_for_socket(&self, _socket_path: &Path) -> Result<bool, SdkError> {
            match self
                .ready_result
                .lock()
                .expect("test readiness lock")
                .clone()
            {
                Ok(ready) => Ok(ready),
                Err(reason) => Err(SdkError::TemporaryRuntime {
                    component: "test-socket".to_owned(),
                    reason,
                    stopped: true,
                }),
            }
        }

        fn terminate_spawned(&self, process_id: u32) -> Result<(), SdkError> {
            self.terminate_calls
                .lock()
                .expect("test terminate lock")
                .push(process_id);
            match self
                .terminate_result
                .lock()
                .expect("test terminate result lock")
                .clone()
            {
                Ok(()) => Ok(()),
                Err(reason) => Err(SdkError::HostCommand {
                    program: "kill".to_owned(),
                    reason,
                }),
            }
        }

        fn request_shutdown(&self, socket_path: &Path) -> Result<bool, SdkError> {
            self.shutdown_calls.fetch_add(1, Ordering::Relaxed);
            let key = socket_path.to_string_lossy().into_owned();
            match self
                .shutdown_results
                .lock()
                .expect("test shutdown lock")
                .get(&key)
                .cloned()
            {
                Some(Ok(delivered)) => Ok(delivered),
                Some(Err(reason)) => Err(SdkError::TemporaryRuntime {
                    component: "firecracker.sock".to_owned(),
                    reason,
                    stopped: false,
                }),
                None => self.socket_answers(socket_path),
            }
        }

        fn wait_for_stop(
            &self,
            socket_path: &Path,
            process_id: Option<u32>,
            firecracker_path: &Path,
            _deadline: std::time::Duration,
        ) -> Result<bool, SdkError> {
            let scripted = self.stop_results.lock().expect("test stop lock").pop();
            match scripted {
                Some(Ok(exited)) => Ok(exited),
                Some(Err(reason)) => Err(SdkError::TemporaryRuntime {
                    component: "firecracker.sock".to_owned(),
                    reason,
                    stopped: true,
                }),
                None => {
                    if self.socket_answers(socket_path)? {
                        return Ok(false);
                    }
                    match process_id {
                        Some(pid) => {
                            Ok(!self.process_references_vm(pid, socket_path, firecracker_path)?)
                        }
                        None => Ok(true),
                    }
                }
            }
        }
    }

    fn fixture_manifest() -> TaumaruRegistry {
        serde_json::from_str(include_str!("../tests/fixtures/manifest.json"))
            .expect("fixture manifest should decode")
    }

    fn file_integrity(bytes: &[u8]) -> FileIntegrity {
        FileIntegrity {
            size_bytes: bytes.len() as u64,
            sha256: hex_digest(Sha256::digest(bytes)),
        }
    }

    fn download_spec(
        home: &Path,
        kind: ArtifactKind,
        artifact_id: &str,
        member_name: Option<&str>,
        relative_path: &str,
        integrity: &FileIntegrity,
        executable: bool,
    ) -> DownloadSpec {
        let relative_path = PathBuf::from(relative_path);
        DownloadSpec {
            artifact_kind: kind,
            artifact_id: artifact_id.to_owned(),
            member_name: member_name.map(str::to_owned),
            artifact_key: match member_name {
                Some(member) => format!("{artifact_id}:{member}"),
                None => artifact_id.to_owned(),
            },
            registry_path: relative_path.to_string_lossy().into_owned(),
            registry_url: "https://fixture.invalid/v1/artifact".to_owned(),
            filename: relative_path
                .file_name()
                .expect("fixture path should have a filename")
                .to_string_lossy()
                .into_owned(),
            expected_size: integrity.size_bytes,
            expected_sha256: integrity.sha256.clone(),
            executable,
            mode: executable.then(|| "755".to_owned()),
            absolute_path: home.join(&relative_path),
            relative_path,
        }
    }

    fn write_fixture_file(
        home: &Path,
        relative_path: &str,
        bytes: &[u8],
        executable: bool,
    ) -> FileIntegrity {
        let path = home.join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture path should have a parent"))
            .expect("fixture parent should be created");
        fs::write(&path, bytes).expect("fixture file should be written");
        if executable {
            set_test_mode(&path, 0o755).expect("fixture executable mode should be applied");
        }
        file_integrity(bytes)
    }

    fn seed_fixture_inventory(sdk: &MicroVmSdk, manifest: &TaumaruRegistry) {
        let distribution = manifest
            .distributions
            .first()
            .expect("fixture distribution should exist");
        let image = distribution
            .images
            .first()
            .expect("fixture image should exist");
        let kernel = manifest
            .kernels
            .iter()
            .find(|candidate| candidate.id == distribution.default_kernel)
            .expect("fixture kernel should exist");
        let firecracker_package = manifest
            .binaries
            .iter()
            .find(|package| package.id == "firecracker-test-1.0.0-x86_64")
            .expect("fixture Firecracker package should exist");
        let firecracker = firecracker_package
            .files
            .iter()
            .find(|file| file.name == "firecracker")
            .expect("fixture Firecracker file should exist");
        let firectl_package = manifest
            .binaries
            .iter()
            .find(|package| package.id == "firectl-test-0.1.0-x86_64")
            .expect("fixture firectl package should exist");
        let firectl = firectl_package
            .files
            .iter()
            .find(|file| file.name == "firectl")
            .expect("fixture firectl file should exist");

        let kernel_bytes = include_bytes!("../tests/fixtures/kernel-fixture");
        let image_bytes = include_bytes!("../tests/fixtures/rootfs-fixture");
        let firecracker_bytes = include_bytes!("../tests/fixtures/firecracker-fixture");
        let firectl_bytes = include_bytes!("../tests/fixtures/jailer-fixture");
        let kernel_integrity = write_fixture_file(
            &sdk.home,
            "artifacts/kernels/linux-test-x86_64/vmlinux",
            kernel_bytes,
            false,
        );
        let image_integrity = write_fixture_file(
            &sdk.home,
            "artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4",
            image_bytes,
            false,
        );
        let firecracker_integrity = write_fixture_file(
            &sdk.home,
            "tools/firecracker-test-1.0.0-x86_64/firecracker",
            firecracker_bytes,
            true,
        );
        let firectl_integrity = write_fixture_file(
            &sdk.home,
            "tools/firectl-test-0.1.0-x86_64/firectl",
            firectl_bytes,
            true,
        );

        sdk.repository
            .persist_kernel(
                kernel,
                &download_spec(
                    &sdk.home,
                    ArtifactKind::Kernel,
                    &kernel.id,
                    None,
                    "artifacts/kernels/linux-test-x86_64/vmlinux",
                    &kernel_integrity,
                    false,
                ),
                &kernel_integrity,
            )
            .expect("fixture kernel should be persisted");
        sdk.repository
            .persist_distribution_image(
                distribution,
                image,
                &manifest.kernels,
                &download_spec(
                    &sdk.home,
                    ArtifactKind::DistributionImage,
                    &distribution.id,
                    Some(&image.id),
                    "artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4",
                    &image_integrity,
                    false,
                ),
                &image_integrity,
            )
            .expect("fixture image should be persisted");
        sdk.repository
            .persist_binary_file(
                firecracker_package,
                firecracker,
                &download_spec(
                    &sdk.home,
                    ArtifactKind::Binary,
                    &firecracker_package.id,
                    Some(&firecracker.name),
                    "tools/firecracker-test-1.0.0-x86_64/firecracker",
                    &firecracker_integrity,
                    true,
                ),
                &firecracker_integrity,
            )
            .expect("fixture Firecracker should be persisted");
        sdk.repository
            .persist_binary_file(
                firectl_package,
                firectl,
                &download_spec(
                    &sdk.home,
                    ArtifactKind::Binary,
                    &firectl_package.id,
                    Some(&firectl.name),
                    "tools/firectl-test-0.1.0-x86_64/firectl",
                    &firectl_integrity,
                    true,
                ),
                &firectl_integrity,
            )
            .expect("fixture firectl should be persisted");
    }

    fn test_request() -> CreateMicroVmRequest {
        CreateMicroVmRequest {
            name: "fixture_vm".to_owned(),
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-minimal".to_owned(),
            disk_size_bytes: 16,
            vcpu_count: 1,
            memory_bytes: 128 * 1024 * 1024,
            expose_on_lan: false,
            lan_address: None,
            volume_path: None,
        }
    }
    fn test_sdk(
        fail_runtime_verification: bool,
    ) -> (
        MicroVmSdk,
        tempfile::TempDir,
        Arc<TestStorage>,
        Arc<TestCredentials>,
        Arc<TestNetwork>,
        Arc<TestRuntime>,
    ) {
        assert_eq!(std::env::consts::ARCH, "x86_64");
        let directory = tempdir().expect("test SDK home should be created");
        let mut sdk = MicroVmSdk::new(directory.path()).expect("test SDK should initialize");
        let manifest = fixture_manifest();
        seed_fixture_inventory(&sdk, &manifest);
        sdk.registry = Arc::new(TestArtifactSource { manifest });
        let storage = Arc::new(TestStorage::default());
        let credentials = Arc::new(TestCredentials::default());
        let network = Arc::new(TestNetwork {
            cleanup_calls: AtomicUsize::new(0),
            delete_cleanup_calls: AtomicUsize::new(0),
            delete_cleanup_results: std::sync::Mutex::new(Vec::new()),
        });
        let runtime = Arc::new(TestRuntime {
            verify_calls: AtomicUsize::new(0),
            fail_verify: fail_runtime_verification,
            live_process: std::sync::Mutex::new(false),
            live_socket: std::sync::Mutex::new(false),
            socket_answers: std::sync::Mutex::new(None),
            socket_errors: std::sync::Mutex::new(std::collections::HashSet::new()),
            launched: std::sync::Mutex::new(Vec::new()),
            launch_result: std::sync::Mutex::new(Ok(4242)),
            ready_result: std::sync::Mutex::new(Ok(true)),
            fail_host: std::sync::Mutex::new(false),
            shutdown_calls: AtomicUsize::new(0),
            shutdown_results: std::sync::Mutex::new(std::collections::HashMap::new()),
            stop_results: std::sync::Mutex::new(Vec::new()),
            process_answers: std::sync::Mutex::new(None),
            terminate_calls: std::sync::Mutex::new(Vec::new()),
            terminate_result: std::sync::Mutex::new(Ok(())),
        });
        sdk.storage = storage.clone();
        sdk.credentials = credentials.clone();
        sdk.network = network.clone();
        sdk.runtime = runtime.clone();
        (sdk, directory, storage, credentials, network, runtime)
    }

    fn start_fixture_vm(
        sdk: &MicroVmSdk,
        name: &str,
        expose_on_lan: bool,
        process_id: Option<u32>,
    ) -> crate::ports::repository::StoredMicroVm {
        use crate::domain::microvm::{MicroVmRecord, PersistedCredential, PersistedNetwork};

        let volume_path = sdk.home.join("vms").join(name);
        fs::create_dir_all(&volume_path).expect("fixture volume should be created");
        let rootfs_path = volume_path.join("rootfs.ext4");
        fs::write(&rootfs_path, b"fixture rootfs").expect("fixture rootfs should be written");
        let ssh_directory = volume_path.join("ssh");
        fs::create_dir_all(&ssh_directory).expect("fixture ssh directory should be created");
        let private_key_path = ssh_directory.join("id_ed25519");
        let public_key_path = ssh_directory.join("id_ed25519.pub");
        fs::write(&private_key_path, b"fixture private key")
            .expect("fixture private key should be written");
        fs::write(&public_key_path, b"ssh-ed25519 AAAA fixture")
            .expect("fixture public key should be written");
        set_test_mode(&private_key_path, 0o600).expect("fixture key mode should be applied");
        set_test_mode(&public_key_path, 0o644).expect("fixture key mode should be applied");
        let guest_mac = crate::adapters::network::linux::guest_mac(name);
        let guest = if expose_on_lan {
            Ipv4Addr::new(10, 200, 4, 2)
        } else {
            Ipv4Addr::new(10, 200, 8, 2)
        };
        let host = if expose_on_lan {
            Ipv4Addr::new(10, 200, 4, 1)
        } else {
            Ipv4Addr::new(10, 200, 8, 1)
        };
        let config = NetworkConfiguration {
            mode: if expose_on_lan {
                NetworkMode::Lan
            } else {
                NetworkMode::HostOnly
            },
            guest_address: IpAddr::V4(guest),
            prefix_length: 30,
            gateway: Some(IpAddr::V4(host)),
            tap_name: format!("tap-{name}"),
            bridge_name: None,
            uplink_name: expose_on_lan.then(|| "test-uplink".to_owned()),
            lan_address: expose_on_lan.then(|| IpAddr::V4(Ipv4Addr::new(192, 168, 3, 60))),
        };
        let network = PersistedNetwork {
            config,
            host_address: Some(IpAddr::V4(host)),
            guest_mac,
            dhcp_lease_reference: None,
            desired_boot_parameters: format!("ip={guest}::{host}:255.255.255.252::eth0:off"),
            resources: Vec::new(),
            uplink_cidr: expose_on_lan.then(|| "192.168.3.0/24".to_owned()),
            proxy_arp_enabled_by_sdk: false,
            bridge_created_by_sdk: false,
            uplink_attached_by_sdk: false,
            forwarding_enabled_by_sdk: true,
            nat_table_created_by_sdk: false,
            nat_chain_created_by_sdk: false,
            host_route_created_by_sdk: false,
            proxy_arp_entry_created_by_sdk: false,
            host_address_specs: Vec::new(),
            default_route_specs: Vec::new(),
        };
        let socket_path = volume_path.join("firecracker.sock");
        let record = MicroVmRecord {
            id: 0,
            name: name.to_owned(),
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-minimal".to_owned(),
            kernel_id: "linux-test-x86_64".to_owned(),
            firecracker_package_id: "firecracker-test-1.0.0-x86_64".to_owned(),
            firectl_package_id: "firectl-test-0.1.0-x86_64".to_owned(),
            disk_size_bytes: 14,
            memory_bytes: 128 * 1024 * 1024,
            memory_effective_mib: 128,
            vcpu_count: 1,
            volume_path: volume_path.clone(),
            rootfs_path: rootfs_path.clone(),
            socket_path: socket_path.clone(),
            expose_on_lan,
            created_at: 1_700_000_000,
        };
        let vm_id = sdk
            .repository
            .insert_microvm(&record)
            .expect("fixture VM should be inserted");
        let credential = PersistedCredential {
            private_key_path,
            public_key_path,
            guest_authorized_keys_path: "/root/.ssh/authorized_keys".to_owned(),
            key_type: "ed25519".to_owned(),
            ssh_user: "root".to_owned(),
            ssh_port: 22,
            public_key_fingerprint: "SHA256:test".to_owned(),
            file_mode: "0600".to_owned(),
        };
        persist_fixture_children(sdk, vm_id, &network, &credential, &socket_path, process_id);
        sdk.repository
            .find_microvm(name)
            .expect("fixture VM should load")
            .expect("fixture VM should exist")
    }

    fn persist_fixture_children(
        sdk: &MicroVmSdk,
        vm_id: i64,
        network: &PersistedNetwork,
        credential: &PersistedCredential,
        socket_path: &Path,
        process_id: Option<u32>,
    ) {
        sdk.repository
            .persist_network(vm_id, network)
            .expect("fixture network should be persisted");
        sdk.repository
            .persist_credential(vm_id, credential)
            .expect("fixture credential should be persisted");
        let runtime = PersistedRuntime {
            firecracker_path: sdk
                .home
                .join("tools/firecracker-test-1.0.0-x86_64/firecracker"),
            firectl_path: sdk.home.join("tools/firectl-test-0.1.0-x86_64/firectl"),
            socket_path: socket_path.to_path_buf(),
            process_id,
            process_state: if process_id.is_some() {
                "running".to_owned()
            } else {
                "stopped".to_owned()
            },
        };
        sdk.repository
            .persist_runtime(vm_id, &runtime)
            .expect("fixture runtime should be persisted");
    }

    fn delete_child_rows(sdk: &MicroVmSdk, name: &str) {
        use rusqlite::params;
        let database = sdk.home.join("state").join("inventory.db");
        let connection = rusqlite::Connection::open(&database).expect("inventory should open");
        let vm_id: i64 = connection
            .query_row(
                "SELECT id FROM microvms WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .expect("fixture VM row should exist");
        for table in ["vm_networks", "vm_credentials", "vm_runtime"] {
            connection
                .execute(
                    &format!("DELETE FROM {table} WHERE microvm_id = ?1"),
                    params![vm_id],
                )
                .expect("child rows should be deleted");
        }
    }

    #[tokio::test]
    async fn streams_stage_starts_ticks_and_completion_with_monotonic_percent() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let request = test_request();
        let expected_bytes = request.disk_size_bytes;
        let mut events = Vec::new();
        let created = sdk
            .create_microvm(request, Some(|event: CreationProgress| events.push(event)))
            .await
            .expect("test VM should be created");

        assert!(
            events.len() >= 13,
            "expected starts + ticks + finishes, got {}",
            events.len()
        );
        let expected_stages = [
            CreationStage::Validation,
            CreationStage::PrerequisiteResolution,
            CreationStage::VolumePreparation,
            CreationStage::CredentialSetup,
            CreationStage::NetworkConfiguration,
            CreationStage::Finalization,
        ];
        for stage in expected_stages {
            let started = events
                .iter()
                .filter(|event| event.stage == stage && event.phase == CreationEventPhase::Started)
                .count();
            let finished = events
                .iter()
                .filter(|event| {
                    event.stage == stage
                        && event.phase == CreationEventPhase::Finished
                        && event.outcome.is_none()
                })
                .count();
            assert_eq!(started, 1, "stage {stage} should start exactly once");
            assert_eq!(finished, 1, "stage {stage} should finish exactly once");
        }
        let volume_ticks = events
            .iter()
            .filter(|event| {
                event.stage == CreationStage::VolumePreparation
                    && event.phase == CreationEventPhase::InProgress
            })
            .count();
        assert!(
            volume_ticks >= 1,
            "volume copy should emit at least one realtime tick"
        );
        let prerequisite_ticks = events
            .iter()
            .filter(|event| {
                event.stage == CreationStage::PrerequisiteResolution
                    && event.phase == CreationEventPhase::InProgress
            })
            .count();
        assert!(
            prerequisite_ticks >= 2,
            "artifact verification should emit realtime ticks"
        );
        let mut previous_percent = 0;
        let mut previous_steps = 0;
        for event in &events {
            assert!(event.overall_percent >= previous_percent);
            assert!(event.overall_percent <= 100);
            assert!(event.completed_steps >= previous_steps);
            assert!(event.completed_steps <= TOTAL_CREATION_STEPS);
            assert_eq!(event.total_steps, TOTAL_CREATION_STEPS);
            previous_percent = event.overall_percent;
            previous_steps = event.completed_steps;
        }
        assert_eq!(
            events
                .first()
                .expect("stream should not be empty")
                .overall_percent,
            0
        );
        let terminal = events.last().expect("terminal event should exist");
        assert_eq!(terminal.stage, CreationStage::Finalization);
        assert_eq!(terminal.completed_steps, TOTAL_CREATION_STEPS);
        assert_eq!(terminal.overall_percent, 100);
        assert_eq!(terminal.outcome, Some(CreationOutcome::Completed));
        let volume_tick = events
            .iter()
            .find(|event| {
                event.stage == CreationStage::VolumePreparation
                    && event.phase == CreationEventPhase::InProgress
            })
            .expect("volume tick should exist");
        let (done, total) = (
            volume_tick
                .bytes_completed
                .expect("tick should carry bytes"),
            volume_tick
                .expected_bytes
                .expect("tick should carry a total"),
        );
        assert!(done <= total);
        assert_eq!(created.state, MicroVmState::Stopped);
        assert_eq!(expected_bytes, test_request().disk_size_bytes);
    }

    #[tokio::test]
    async fn routes_each_creation_stream_to_its_own_observer() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut first_request = test_request();
        first_request.name = "isolated_a".to_owned();
        let mut second_request = test_request();
        second_request.name = "isolated_b".to_owned();
        let mut first_events = Vec::new();
        let first = sdk
            .create_microvm(
                first_request,
                Some(|event: CreationProgress| first_events.push(event)),
            )
            .await
            .expect("first VM should be created");
        let mut second_events = Vec::new();
        let second = sdk
            .create_microvm(
                second_request,
                Some(|event: CreationProgress| second_events.push(event)),
            )
            .await
            .expect("second VM should be created");

        assert_eq!(first.name, "isolated_a");
        assert_eq!(second.name, "isolated_b");
        for events in [&first_events, &second_events] {
            assert!(!events.is_empty());
            assert_eq!(
                events.last().expect("terminal event should exist").outcome,
                Some(CreationOutcome::Completed)
            );
            assert!(
                events
                    .windows(2)
                    .all(|pair| pair[1].overall_percent >= pair[0].overall_percent)
            );
        }
        assert_ne!(first.network.tap_name, second.network.tap_name);
    }

    #[tokio::test]
    async fn ends_invalid_requests_with_a_validation_start_plus_failed_terminal() {
        let directory = tempdir().expect("temporary directory should be created");
        let sdk = MicroVmSdk::new(directory.path()).expect("test SDK should initialize");
        let mut request = test_request();
        request.name = "not path friendly".to_owned();
        let mut events = Vec::new();
        let error = sdk
            .create_microvm(request, Some(|event: CreationProgress| events.push(event)))
            .await
            .expect_err("invalid request should fail");

        assert!(matches!(error, SdkError::InvalidRequest { .. }));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, CreationEventPhase::Started);
        assert_eq!(events[0].overall_percent, 0);
        assert_eq!(events[1].stage, CreationStage::Validation);
        assert_eq!(events[1].completed_steps, 0);
        assert_eq!(events[1].total_steps, TOTAL_CREATION_STEPS);
        assert_eq!(
            events[1].outcome,
            Some(CreationOutcome::Failed {
                stage: CreationStage::Validation,
            })
        );
    }

    #[tokio::test]
    async fn ends_idempotent_repeats_with_a_validation_start_plus_configured_terminal() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        sdk.create_microvm(test_request(), None::<fn(CreationProgress)>)
            .await
            .expect("test VM should be created");
        let mut events = Vec::new();
        let repeated = sdk
            .create_microvm(
                test_request(),
                Some(|event: CreationProgress| events.push(event)),
            )
            .await
            .expect("identical creation should be idempotent");

        assert_eq!(repeated.state, MicroVmState::Stopped);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, CreationEventPhase::Started);
        assert_eq!(events[1].stage, CreationStage::Validation);
        assert_eq!(events[1].completed_steps, 0);
        assert_eq!(events[1].outcome, Some(CreationOutcome::AlreadyConfigured));
    }

    #[tokio::test]
    async fn ends_conflicts_with_a_validation_start_plus_failed_terminal() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        sdk.create_microvm(test_request(), None::<fn(CreationProgress)>)
            .await
            .expect("test VM should be created");
        let mut conflicting_request = test_request();
        conflicting_request.disk_size_bytes = 17;
        let mut events = Vec::new();
        let error = sdk
            .create_microvm(
                conflicting_request,
                Some(|event: CreationProgress| events.push(event)),
            )
            .await
            .expect_err("immutable request changes should conflict");

        assert!(matches!(error, SdkError::ConfigurationConflict { .. }));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, CreationEventPhase::Started);
        assert_eq!(events[1].stage, CreationStage::Validation);
        assert_eq!(events[1].completed_steps, 0);
        assert_eq!(
            events[1].outcome,
            Some(CreationOutcome::Failed {
                stage: CreationStage::Validation,
            })
        );
    }

    #[tokio::test]
    async fn ends_prerequisite_failures_with_started_and_failed_terminal() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut request = test_request();
        request.distribution_id = "no-such-distribution".to_owned();
        let mut events = Vec::new();
        let error = sdk
            .create_microvm(request, Some(|event: CreationProgress| events.push(event)))
            .await
            .expect_err("unknown distribution should fail");

        assert!(matches!(error, SdkError::NotFound { .. }));
        assert!(events.len() >= 3);
        assert_eq!(events[0].stage, CreationStage::Validation);
        assert_eq!(events[0].phase, CreationEventPhase::Started);
        assert_eq!(
            events[1].outcome, None,
            "validation should finish before prerequisites fail"
        );
        let terminal = events.last().expect("terminal event should exist");
        assert_eq!(terminal.stage, CreationStage::PrerequisiteResolution);
        assert_eq!(terminal.completed_steps, 1);
        assert_eq!(
            terminal.outcome,
            Some(CreationOutcome::Failed {
                stage: CreationStage::PrerequisiteResolution,
            })
        );
    }

    #[tokio::test]
    async fn reports_monotonic_percent_with_byte_ticks_only_on_byte_work() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let request = test_request();
        let mut events = Vec::new();
        sdk.create_microvm(request, Some(|event: CreationProgress| events.push(event)))
            .await
            .expect("test VM should be created");

        assert_eq!(TOTAL_CREATION_STEPS, 6);
        let mut previous_percent = 0;
        let mut previous_steps = 0;
        for event in &events {
            assert!(event.overall_percent >= previous_percent);
            assert!(event.overall_percent <= 100);
            assert!(event.completed_steps >= previous_steps);
            assert!(event.completed_steps <= TOTAL_CREATION_STEPS);
            assert_eq!(event.total_steps, TOTAL_CREATION_STEPS);
            previous_percent = event.overall_percent;
            previous_steps = event.completed_steps;
        }
        for event in &events {
            match event.phase {
                CreationEventPhase::InProgress => {
                    let (done, total) = (
                        event.bytes_completed.expect("tick should carry bytes"),
                        event.expected_bytes.expect("tick should carry a total"),
                    );
                    assert!(done <= total);
                    assert!(total >= 1);
                }
                CreationEventPhase::Started | CreationEventPhase::Finished => {
                    assert_eq!(event.bytes_completed, None);
                    assert_eq!(event.expected_bytes, None);
                }
            }
        }
        let tick_stages: Vec<CreationStage> = events
            .iter()
            .filter(|event| event.phase == CreationEventPhase::InProgress)
            .map(|event| event.stage)
            .collect();
        assert!(tick_stages.contains(&CreationStage::PrerequisiteResolution));
        assert!(tick_stages.contains(&CreationStage::VolumePreparation));
    }

    #[tokio::test]
    async fn returns_identical_results_with_and_without_an_observer() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut observed_request = test_request();
        observed_request.name = "observed_vm".to_owned();
        let mut events = Vec::new();
        let observed = sdk
            .create_microvm(
                observed_request,
                Some(|event: CreationProgress| events.push(event)),
            )
            .await
            .expect("observed VM should be created");
        let mut unobserved_request = test_request();
        unobserved_request.name = "unobserved_vm".to_owned();
        let unobserved = sdk
            .create_microvm(unobserved_request, None::<fn(CreationProgress)>)
            .await
            .expect("unobserved VM should be created");

        assert!(!events.is_empty());
        assert_eq!(
            events.last().expect("terminal event should exist").outcome,
            Some(CreationOutcome::Completed)
        );
        assert_eq!(observed.state, unobserved.state);
        assert_eq!(observed.distribution_id, unobserved.distribution_id);
        assert_eq!(observed.image_id, unobserved.image_id);
        assert_eq!(observed.vcpu_count, unobserved.vcpu_count);
        assert_eq!(observed.memory_bytes, unobserved.memory_bytes);
        assert_eq!(observed.disk_size_bytes, unobserved.disk_size_bytes);
        assert_eq!(observed.network.mode, unobserved.network.mode);
        assert_eq!(observed.ssh.user, unobserved.ssh.user);
        assert_eq!(observed.ssh.port, unobserved.ssh.port);
    }

    #[tokio::test]
    async fn creates_an_idempotent_host_only_vm_through_injected_ports() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, directory, storage, credentials, _network, runtime) = test_sdk(false);
        let request = test_request();
        let first = sdk
            .create_microvm(request.clone(), None::<fn(CreationProgress)>)
            .await
            .expect("test VM should be created");
        let second = sdk
            .create_microvm(request, None::<fn(CreationProgress)>)
            .await
            .expect("identical test VM creation should be idempotent");

        assert_eq!(first.state, MicroVmState::Stopped);
        assert_eq!(first.network.mode, NetworkMode::HostOnly);
        assert_eq!(first.ssh.user, "root");
        assert_eq!(first.ssh.port, 22);
        assert_eq!(first, second);
        assert_eq!(storage.prepare_calls.load(Ordering::Relaxed), 1);
        assert_eq!(storage.inject_calls.load(Ordering::Relaxed), 1);
        assert_eq!(credentials.generate_calls.load(Ordering::Relaxed), 1);
        assert_eq!(runtime.verify_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            fs::metadata(&first.rootfs_path)
                .expect("rootfs should exist")
                .len(),
            16
        );
        assert!(!first.socket_path.exists());
        assert!(first.ssh.private_key_path.exists());
        assert!(first.volume_path.starts_with(directory.path()));
    }

    #[tokio::test]
    async fn reconciles_a_persisted_host_only_network_without_recreating_it() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        sdk.create_microvm(test_request(), None::<fn(CreationProgress)>)
            .await
            .expect("test VM should be created");

        let result = sdk
            .configure_network("fixture_vm")
            .await
            .expect("persisted network should reconcile");

        assert!(
            result.configuration.guest_address.is_loopback()
                || !result.configuration.guest_address.is_unspecified()
        );
        assert!(result.applied.is_empty());
        assert!(result.skipped.contains(&NetworkResource::Tap));
        assert!(result.skipped.contains(&NetworkResource::IptablesNat));
        assert_eq!(runtime.verify_calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn isolates_two_routed_lan_vms_with_distinct_addresses() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut first_request = test_request();
        first_request.name = "lan_iso_a".to_owned();
        first_request.expose_on_lan = true;
        first_request.lan_address = Some(std::net::Ipv4Addr::new(192, 168, 3, 91));
        let mut second_request = test_request();
        second_request.name = "lan_iso_b".to_owned();
        second_request.expose_on_lan = true;
        second_request.lan_address = Some(std::net::Ipv4Addr::new(192, 168, 3, 92));
        let first = sdk
            .create_microvm(first_request, None::<fn(CreationProgress)>)
            .await
            .expect("first LAN VM should be created");
        let second = sdk
            .create_microvm(second_request, None::<fn(CreationProgress)>)
            .await
            .expect("second LAN VM should be created");

        assert_ne!(first.network.lan_address, second.network.lan_address);
        assert_ne!(first.network.guest_address, second.network.guest_address);
        assert_ne!(first.network.tap_name, second.network.tap_name);
    }

    #[tokio::test]
    async fn rejects_a_duplicate_lan_override_used_by_another_vm() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        let _ = network;
        let mut first_request = test_request();
        first_request.name = "lan_dup_a".to_owned();
        first_request.expose_on_lan = true;
        first_request.lan_address = Some(std::net::Ipv4Addr::new(192, 168, 3, 91));
        sdk.create_microvm(first_request, None::<fn(CreationProgress)>)
            .await
            .expect("first LAN VM should be created");
        let stored = sdk
            .run_repository(|repository| repository.list_lan_addresses())
            .await
            .expect("LAN listing should work");
        assert!(
            stored.iter().any(|(_, address, _)| *address
                == std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 3, 91))),
            "first VM LAN address should be listed"
        );
    }

    #[tokio::test]
    async fn rejects_an_immutable_creation_conflict_without_mutating_the_existing_vm() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, storage, _credentials, _network, _runtime) = test_sdk(false);
        sdk.create_microvm(test_request(), None::<fn(CreationProgress)>)
            .await
            .expect("test VM should be created");
        let mut conflicting_request = test_request();
        conflicting_request.disk_size_bytes = 17;

        let error = sdk
            .create_microvm(conflicting_request, None::<fn(CreationProgress)>)
            .await
            .expect_err("immutable request changes should conflict");

        assert!(matches!(error, SdkError::ConfigurationConflict { .. }));
        assert_eq!(storage.prepare_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn rolls_back_the_provisional_vm_after_a_stopped_state_failure() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(true);
        let volume_path = sdk.home.join("vms/fixture_vm");
        let error = sdk
            .create_microvm(test_request(), None::<fn(CreationProgress)>)
            .await
            .expect_err("injected runtime verification should fail creation");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert_eq!(network.cleanup_calls.load(Ordering::Relaxed), 1);
        assert!(!volume_path.exists());
        assert!(
            sdk.repository
                .find_microvm("fixture_vm")
                .expect("test inventory should be readable")
                .is_none()
        );
    }

    #[tokio::test]
    async fn creates_a_routed_lan_vm_with_override_and_guest_setup() {
        use std::net::Ipv4Addr;

        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut request = test_request();
        request.name = "lan_vm".to_owned();
        request.expose_on_lan = true;
        request.lan_address = Some(Ipv4Addr::new(192, 168, 3, 77));
        let created = sdk
            .create_microvm(request, None::<fn(CreationProgress)>)
            .await
            .expect("routed LAN VM should be created");

        assert_eq!(created.state, MicroVmState::Stopped);
        assert_eq!(created.network.mode, NetworkMode::Lan);
        assert_eq!(
            created.network.lan_address,
            Some(std::net::IpAddr::V4(Ipv4Addr::new(192, 168, 3, 77)))
        );
        assert_eq!(created.network.bridge_name, None);
        assert_eq!(created.network.uplink_name, Some("test-uplink".to_owned()));
        assert!(!created.socket_path.exists());
    }

    #[tokio::test]
    async fn rejects_a_lan_override_for_host_only_creation() {
        use std::net::Ipv4Addr;

        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut request = test_request();
        request.lan_address = Some(Ipv4Addr::new(192, 168, 3, 77));
        let error = sdk
            .create_microvm(request, None::<fn(CreationProgress)>)
            .await
            .expect_err("host-only LAN override should fail");

        assert!(matches!(error, SdkError::InvalidRequest { .. }));
    }

    #[tokio::test]
    async fn reconciles_a_persisted_routed_lan_network_without_reallocating() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let mut request = test_request();
        request.name = "lan_reconcile_vm".to_owned();
        request.expose_on_lan = true;
        request.lan_address = Some(std::net::Ipv4Addr::new(192, 168, 3, 88));
        let created = sdk
            .create_microvm(request, None::<fn(CreationProgress)>)
            .await
            .expect("routed LAN VM should be created");
        let result = sdk
            .configure_network("lan_reconcile_vm")
            .await
            .expect("persisted routed network should reconcile");

        assert!(
            result.configuration.guest_address.is_loopback()
                || !result.configuration.guest_address.is_unspecified()
        );
        assert_eq!(
            result.configuration.lan_address,
            created.network.lan_address
        );
        assert_eq!(
            result.configuration.guest_address,
            created.network.guest_address
        );
        assert!(result.applied.is_empty());
        assert!(result.skipped.contains(&NetworkResource::Tap));
        assert!(result.skipped.contains(&NetworkResource::HostRoute));
        assert!(result.skipped.contains(&NetworkResource::ProxyArpEntry));
    }

    #[tokio::test]
    async fn start_rejects_invalid_unknown_and_creating_names_without_mutation() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let invalid = sdk.start_microvm("not path friendly").await;
        assert!(matches!(invalid, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));
        let missing = sdk.start_microvm("missing_vm").await;
        assert!(
            matches!(missing, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "missing_vm")
        );
        start_fixture_vm(&sdk, "creating_vm", false, None);
        delete_child_rows(&sdk, "creating_vm");
        let creating = sdk.start_microvm("creating_vm").await;
        assert!(
            matches!(creating, Err(SdkError::LifecycleConflict { name, state, .. }) if name == "creating_vm" && state == "creation incomplete")
        );
        assert!(runtime.launched.lock().expect("launch lock").is_empty());
    }

    #[tokio::test]
    async fn start_rejects_a_broken_volume_without_launching() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "broken_vm", false, None);
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("broken_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "broken_vm".to_owned(),
                    })
            })
            .await
            .expect("fixture should load");
        fs::remove_file(&stored.record.rootfs_path).expect("rootfs should be removed");
        let error = sdk
            .start_microvm("broken_vm")
            .await
            .expect_err("broken volume should fail");
        assert!(matches!(
            error,
            SdkError::Filesystem { .. } | SdkError::GuestFilesystem { .. }
        ));
        assert!(runtime.launched.lock().expect("launch lock").is_empty());
    }

    #[tokio::test]
    async fn start_boots_a_configured_vm_and_records_the_running_state() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "boot_vm", false, None);
        let started = sdk
            .start_microvm("boot_vm")
            .await
            .expect("configured VM should start");
        assert_eq!(started.state, MicroVmState::Running);
        assert_eq!(started.name, "boot_vm");
        assert_eq!(started.process_id, 4242);
        assert_eq!(
            started.socket_path,
            started.volume_path.join("firecracker.sock")
        );
        assert_eq!(started.ssh.user, "root");
        assert_eq!(started.ssh.port, 22);
        {
            let launched = runtime.launched.lock().expect("launch lock");
            assert_eq!(launched.len(), 1);
            assert_eq!(launched[0].vm_name, "boot_vm");
            assert_eq!(launched[0].vcpu_count, 1);
            assert_eq!(launched[0].memory_effective_mib, 128);
            assert_eq!(launched[0].socket_path, started.socket_path);
        }
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("boot_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "boot_vm".to_owned(),
                    })
            })
            .await
            .expect("started VM should load");
        let (_, _, runtime) = stored
            .require_full("start MicroVM")
            .expect("started VM is complete");
        assert_eq!(runtime.process_id, Some(4242));
        assert_eq!(runtime.process_state, "running");
    }

    #[tokio::test]
    async fn start_returns_the_live_vm_without_launching_again() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "live_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = true;
        *runtime.live_socket.lock().expect("socket lock") = true;
        let again = sdk
            .start_microvm("live_vm")
            .await
            .expect("live VM should return its identity");
        assert_eq!(again.state, MicroVmState::Running);
        assert_eq!(again.process_id, 4242);
        assert!(runtime.launched.lock().expect("launch lock").is_empty());
    }

    #[tokio::test]
    async fn start_recovers_a_dead_process_with_a_fresh_launch() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "dead_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = false;
        *runtime.live_socket.lock().expect("socket lock") = false;
        let recovered = sdk
            .start_microvm("dead_vm")
            .await
            .expect("dead VM should start fresh");
        assert_eq!(recovered.state, MicroVmState::Running);
        assert_eq!(runtime.launched.lock().expect("launch lock").len(), 1);
    }

    #[tokio::test]
    async fn start_recovers_a_referencing_process_with_a_silent_socket() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "mismatch_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = true;
        *runtime.live_socket.lock().expect("socket lock") = false;
        let recovered = sdk
            .start_microvm("mismatch_vm")
            .await
            .expect("silent socket should start fresh");
        assert_eq!(recovered.state, MicroVmState::Running);
        assert_eq!(runtime.launched.lock().expect("launch lock").len(), 1);
    }

    #[tokio::test]
    async fn start_refuses_a_live_socket_with_a_foreign_process() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "mismatch_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = false;
        *runtime.live_socket.lock().expect("socket lock") = true;
        let error = sdk
            .start_microvm("mismatch_vm")
            .await
            .expect_err("foreign-live socket should be refused");
        assert!(
            matches!(error, SdkError::TemporaryRuntime { ref component, stopped, .. } if component == "firecracker" && !stopped)
        );
        assert!(runtime.launched.lock().expect("launch lock").is_empty());
    }

    #[tokio::test]
    async fn start_removes_only_the_persisted_stale_socket() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "stale_socket_vm", false, None);
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("stale_socket_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "stale_socket_vm".to_owned(),
                    })
            })
            .await
            .expect("fixture should load");
        fs::write(&stored.record.socket_path, b"stale").expect("stale socket should be written");
        let unrelated = stored.record.volume_path.join("unrelated.txt");
        fs::write(&unrelated, b"keep").expect("unrelated file should be written");
        sdk.start_microvm("stale_socket_vm")
            .await
            .expect("stale socket should be cleaned and started");
        assert!(unrelated.is_file());
    }

    #[tokio::test]
    async fn start_serializes_concurrent_calls_to_one_process() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "race_vm", false, None);
        let sdk = std::sync::Arc::new(sdk);
        let first = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.start_microvm("race_vm").await })
        };
        let second = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.start_microvm("race_vm").await })
        };
        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first start should join")
            .expect("first start should succeed");
        let second = second
            .expect("second start should join")
            .expect("second start should succeed");
        assert_eq!(first.state, MicroVmState::Running);
        assert_eq!(second.state, MicroVmState::Running);
        let launched = runtime.launched.lock().expect("launch lock").len();
        assert!(
            launched <= 2,
            "concurrent starts launched {launched} processes"
        );
    }

    #[tokio::test]
    async fn start_keeps_repaired_network_when_the_launch_fails() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "failed_launch_vm", false, None);
        *runtime.ready_result.lock().expect("readiness lock") = Ok(false);
        let error = sdk
            .start_microvm("failed_launch_vm")
            .await
            .expect_err("failed readiness should fail");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("failed_launch_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "failed_launch_vm".to_owned(),
                    })
            })
            .await
            .expect("failed VM should load");
        let (_, _, runtime) = stored
            .require_full("start MicroVM")
            .expect("failed VM is complete");
        assert_eq!(runtime.process_id, None);
        assert_eq!(runtime.process_state, "stopped");
    }

    #[tokio::test]
    async fn start_repairs_host_only_network_with_identity_unchanged() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let before = start_fixture_vm(&sdk, "repair_host_only", false, None);
        let started = sdk
            .start_microvm("repair_host_only")
            .await
            .expect("repaired host-only VM should start");
        assert_eq!(started.network.mode, NetworkMode::HostOnly);
        assert_eq!(
            started.network.guest_address,
            before
                .network
                .as_ref()
                .expect("fixture is complete")
                .config
                .guest_address
        );
        assert_eq!(
            started.network.tap_name,
            before
                .network
                .as_ref()
                .expect("fixture is complete")
                .config
                .tap_name
        );
    }

    #[tokio::test]
    async fn start_repairs_lan_network_with_identity_unchanged() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let before = start_fixture_vm(&sdk, "repair_lan", true, None);
        let started = sdk
            .start_microvm("repair_lan")
            .await
            .expect("repaired LAN VM should start");
        assert_eq!(started.network.mode, NetworkMode::Lan);
        assert_eq!(
            started.network.lan_address,
            before
                .network
                .as_ref()
                .expect("fixture is complete")
                .config
                .lan_address
        );
        assert_eq!(
            started.network.tap_name,
            before
                .network
                .as_ref()
                .expect("fixture is complete")
                .config
                .tap_name
        );
    }

    #[tokio::test]
    async fn running_listing_is_empty_without_inventory_rows() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let running = sdk
            .list_running_microvms()
            .await
            .expect("empty inventory should list no running VMs");
        assert!(running.is_empty());
    }

    #[tokio::test]
    async fn running_listing_returns_live_vms_ordered_with_ssh_material() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let zeta = start_fixture_vm(&sdk, "zeta", false, Some(4242));
        let alpha = start_fixture_vm(&sdk, "alpha", false, Some(4242));
        start_fixture_vm(&sdk, "stopped", false, None);
        start_fixture_vm(&sdk, "creating", false, None);
        delete_child_rows(&sdk, "creating");
        *runtime.live_process.lock().expect("liveness lock") = false;
        *runtime.live_socket.lock().expect("socket lock") = false;
        runtime
            .socket_answers
            .lock()
            .expect("socket map lock")
            .replace(
                [
                    (zeta.record.socket_path.to_string_lossy().into_owned(), true),
                    (
                        alpha.record.socket_path.to_string_lossy().into_owned(),
                        true,
                    ),
                ]
                .into_iter()
                .collect(),
            );
        let running = sdk
            .list_running_microvms()
            .await
            .expect("live VMs should be listed");
        let names: Vec<&str> = running.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        for entry in &running {
            assert_eq!(entry.ssh.user, "root");
            assert_eq!(entry.ssh.port, 22);
            assert!(entry.ssh.private_key_path.ends_with("id_ed25519"));
            assert!(entry.ssh.private_key_path.is_absolute());
        }
    }

    #[tokio::test]
    async fn running_listing_omits_machines_without_liveness() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "dead_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = false;
        *runtime.live_socket.lock().expect("socket lock") = false;
        let running = sdk
            .list_running_microvms()
            .await
            .expect("dead process should be omitted");
        assert!(running.is_empty());
    }

    #[tokio::test]
    async fn running_listing_omits_a_referencing_process_with_a_silent_socket() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "mismatch_vm", false, Some(4242));
        *runtime.live_process.lock().expect("liveness lock") = true;
        *runtime.live_socket.lock().expect("socket lock") = false;
        let running = sdk
            .list_running_microvms()
            .await
            .expect("silent socket should be omitted");
        assert!(running.is_empty());
    }

    #[tokio::test]
    async fn socket_governs_running_lists() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        for live_process in [true, false] {
            let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
            start_fixture_vm(&sdk, "mismatch_vm", false, Some(4242));
            *runtime.live_process.lock().expect("liveness lock") = live_process;
            *runtime.live_socket.lock().expect("socket lock") = true;
            let running = sdk
                .list_running_microvms()
                .await
                .expect("live socket should be listed");
            assert_eq!(running.len(), 1);
            assert_eq!(running[0].name, "mismatch_vm");
            let listed = sdk.list_microvms().await.expect("listing should work");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].state, MicroVmState::Running);
        }
    }

    #[tokio::test]
    async fn probe_errors_resolve_to_stopped() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "error_vm", false, Some(4242));
        runtime
            .socket_errors
            .lock()
            .expect("socket error lock")
            .insert(stored.record.socket_path.to_string_lossy().into_owned());
        let listed = sdk.list_microvms().await.expect("listing should work");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].state, MicroVmState::Stopped);
        let running = sdk
            .list_running_microvms()
            .await
            .expect("running listing should work");
        assert!(running.is_empty());
    }

    #[tokio::test]
    async fn bulk_listing_resolves_mixed_outcomes_independently() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let live = start_fixture_vm(&sdk, "bulk_live", false, Some(4242));
        let silent = start_fixture_vm(&sdk, "bulk_silent", false, Some(4242));
        let failed = start_fixture_vm(&sdk, "bulk_failed", false, Some(4242));
        let _ = silent;
        runtime
            .socket_answers
            .lock()
            .expect("socket map lock")
            .replace(
                [
                    (live.record.socket_path.to_string_lossy().into_owned(), true),
                    (
                        failed.record.socket_path.to_string_lossy().into_owned(),
                        false,
                    ),
                ]
                .into_iter()
                .collect(),
            );
        runtime
            .socket_errors
            .lock()
            .expect("socket error lock")
            .insert(failed.record.socket_path.to_string_lossy().into_owned());
        *runtime.live_socket.lock().expect("socket lock") = false;
        let listed = sdk.list_microvms().await.expect("listing should work");
        let states: Vec<(&str, MicroVmState)> = listed
            .iter()
            .map(|entry| (entry.name.as_str(), entry.state))
            .collect();
        assert_eq!(
            states,
            [
                ("bulk_failed", MicroVmState::Stopped),
                ("bulk_live", MicroVmState::Running),
                ("bulk_silent", MicroVmState::Stopped),
            ]
        );
    }
    #[tokio::test]
    async fn stop_rejects_invalid_unknown_and_incomplete_names_without_mutation() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let invalid = sdk.stop_microvm("not path friendly").await;
        assert!(matches!(invalid, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));
        let missing = sdk.stop_microvm("missing_vm").await;
        assert!(
            matches!(missing, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "missing_vm")
        );
        start_fixture_vm(&sdk, "creating_vm", false, None);
        delete_child_rows(&sdk, "creating_vm");
        let creating = sdk.stop_microvm("creating_vm").await;
        assert!(
            matches!(creating, Err(SdkError::LifecycleConflict { name, state, .. }) if name == "creating_vm" && state == "creation incomplete")
        );
        assert_eq!(runtime.shutdown_calls.load(Ordering::Relaxed), 0);
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stop_returns_stopped_without_host_changes_for_silent_sockets() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        for name in ["stopped_vm", "never_started_vm", "externally_killed_vm"] {
            let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
            let process = if name == "externally_killed_vm" {
                Some(4242)
            } else {
                None
            };
            start_fixture_vm(&sdk, name, false, process);
            *runtime.live_socket.lock().expect("socket lock") = false;
            *runtime.live_process.lock().expect("liveness lock") = false;
            let stopped = sdk
                .stop_microvm(name)
                .await
                .expect("silent socket should stop successfully");
            assert_eq!(stopped.state, MicroVmState::Stopped);
            assert!(!stopped.forced);
            assert_eq!(runtime.shutdown_calls.load(Ordering::Relaxed), 0);
            assert!(
                runtime
                    .terminate_calls
                    .lock()
                    .expect("terminate lock")
                    .is_empty()
            );
            let stored = sdk
                .run_repository(|repository| {
                    repository
                        .find_microvm(name)?
                        .ok_or_else(|| SdkError::NotFound {
                            kind: "MicroVM".to_owned(),
                            id: name.to_owned(),
                        })
                })
                .await
                .expect("fixture should load");
            let (_, _, persisted) = stored
                .require_full("stop MicroVM")
                .expect("stopped VM is complete");
            assert_eq!(persisted.process_id, None);
            assert_eq!(persisted.process_state, "stopped");
        }
    }

    #[tokio::test]
    async fn stop_returns_stopped_for_a_race_silenced_socket() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "race_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        runtime
            .shutdown_results
            .lock()
            .expect("shutdown lock")
            .insert(
                stored.record.socket_path.to_string_lossy().into_owned(),
                Ok(false),
            );
        *runtime.live_socket.lock().expect("socket lock") = false;
        let stopped = sdk
            .stop_microvm("race_vm")
            .await
            .expect("silenced socket should stop successfully");
        assert_eq!(stopped.state, MicroVmState::Stopped);
        assert!(!stopped.forced);
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stop_shuts_down_a_running_vm_gracefully() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "graceful_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        *runtime.live_process.lock().expect("liveness lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .push(Ok(true));
        let stopped = sdk
            .stop_microvm("graceful_vm")
            .await
            .expect("running VM should stop gracefully");
        assert_eq!(stopped.state, MicroVmState::Stopped);
        assert!(!stopped.forced);
        assert_eq!(runtime.shutdown_calls.load(Ordering::Relaxed), 1);
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("graceful_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "graceful_vm".to_owned(),
                    })
            })
            .await
            .expect("stopped VM should load");
        let (_, _, persisted) = stored
            .require_full("stop MicroVM")
            .expect("stopped VM is complete");
        assert_eq!(persisted.process_id, None);
        assert_eq!(persisted.process_state, "stopped");
        assert!(!stored.record.socket_path.exists());
    }

    #[tokio::test]
    async fn stop_forces_an_unresponsive_vm_and_reports_forced() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "forced_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        *runtime.live_process.lock().expect("liveness lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .extend([Ok(true), Ok(false)]);
        let stopped = sdk
            .stop_microvm("forced_vm")
            .await
            .expect("unresponsive VM should be forced");
        assert_eq!(stopped.state, MicroVmState::Stopped);
        assert!(stopped.forced);
        assert_eq!(
            *runtime.terminate_calls.lock().expect("terminate lock"),
            vec![4242]
        );
    }

    #[tokio::test]
    async fn stop_treats_a_natural_exit_before_sigkill_as_graceful() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "natural_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        *runtime.live_process.lock().expect("liveness lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .extend([Ok(true), Ok(false)]);
        runtime
            .process_answers
            .lock()
            .expect("process map lock")
            .replace([(4242, false)].into_iter().collect());
        *runtime.live_socket.lock().expect("socket lock") = false;
        let stopped = sdk
            .stop_microvm("natural_vm")
            .await
            .expect("natural exit should stop gracefully");
        assert!(!stopped.forced);
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stop_runs_one_shutdown_sequence_for_concurrent_stops() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "shared_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        *runtime.live_process.lock().expect("liveness lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .extend([Ok(true), Ok(true), Ok(true)]);
        let sdk = std::sync::Arc::new(sdk);
        let first = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.stop_microvm("shared_vm").await })
        };
        let second = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.stop_microvm("shared_vm").await })
        };
        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first stop should join")
            .expect("first stop should work");
        let second = second
            .expect("second stop should join")
            .expect("second stop should work");
        assert_eq!(first.state, MicroVmState::Stopped);
        assert_eq!(second.state, MicroVmState::Stopped);
        let shutdowns = runtime.shutdown_calls.load(Ordering::Relaxed);
        assert!(
            shutdowns <= 2,
            "concurrent stops sent {shutdowns} shutdown requests"
        );
    }
    #[tokio::test]
    async fn stop_errors_without_forcing_when_shutdown_is_undeliverable() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "broken_pipe_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        runtime
            .shutdown_results
            .lock()
            .expect("shutdown lock")
            .insert(
                stored.record.socket_path.to_string_lossy().into_owned(),
                Err("injected delivery failure".to_owned()),
            );
        let error = sdk
            .stop_microvm("broken_pipe_vm")
            .await
            .expect_err("undeliverable shutdown should fail");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
        let stored = sdk
            .run_repository(|repository| {
                repository
                    .find_microvm("broken_pipe_vm")?
                    .ok_or_else(|| SdkError::NotFound {
                        kind: "MicroVM".to_owned(),
                        id: "broken_pipe_vm".to_owned(),
                    })
            })
            .await
            .expect("failed VM should load");
        let (_, _, persisted) = stored
            .require_full("stop MicroVM")
            .expect("failed VM is complete");
        assert_eq!(persisted.process_id, Some(4242));
    }

    #[tokio::test]
    async fn stop_errors_without_signaling_when_no_process_identity_exists() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "orphaned_vm", false, None);
        *runtime.live_socket.lock().expect("socket lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .push(Ok(false));
        let error = sdk
            .stop_microvm("orphaned_vm")
            .await
            .expect_err("unforceable machine should fail");
        assert!(matches!(
            error,
            SdkError::TemporaryRuntime { stopped: false, .. }
        ));
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stop_never_signals_a_recycled_process_identity() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "recycled_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        runtime
            .process_answers
            .lock()
            .expect("process map lock")
            .replace([(4242, false)].into_iter().collect());
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .push(Ok(false));
        let error = sdk
            .stop_microvm("recycled_vm")
            .await
            .expect_err("recycled PID should fail");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert!(
            runtime
                .terminate_calls
                .lock()
                .expect("terminate lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn prune_skips_a_candidate_whose_target_lock_is_held() {
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let kernel_id = "linux-test-x86_64".to_owned();
        let target = sdk.home.join("artifacts/kernels/linux-test-x86_64/vmlinux");
        assert!(target.is_file());
        let held = sdk
            .target_lock(&target)
            .expect("prune target lock should resolve");
        let _guard = held.lock().await;
        let summary = sdk
            .prune_unused_artifacts()
            .await
            .expect("prune with a held lock should succeed");
        assert_eq!(
            summary.skipped_artifact_keys,
            vec!["kernel:linux-test-x86_64".to_owned()]
        );
        assert!(summary.removed_kernels.is_empty());
        assert!(target.is_file());
        drop(_guard);
        let summary = sdk
            .prune_unused_artifacts()
            .await
            .expect("prune after releasing the lock should succeed");
        assert!(summary.skipped_artifact_keys.is_empty());
        assert_eq!(summary.removed_kernels, vec![kernel_id]);
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn stop_errors_when_the_machine_survives_sigkill() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "stubborn_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        *runtime.live_process.lock().expect("liveness lock") = true;
        runtime
            .stop_results
            .lock()
            .expect("stop lock")
            .extend([Ok(false), Ok(false)]);
        let error = sdk
            .stop_microvm("stubborn_vm")
            .await
            .expect_err("surviving machine should fail");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert_eq!(
            *runtime.terminate_calls.lock().expect("terminate lock"),
            vec![4242]
        );
    }

    #[tokio::test]
    async fn delete_removes_a_stopped_vm_and_everything_it_owns() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "doomed_vm", false, None);
        let volume_path = stored.record.volume_path.clone();
        assert!(volume_path.join("rootfs.ext4").is_file());
        let deleted = sdk
            .delete_microvm("doomed_vm")
            .await
            .expect("stopped VM should delete");
        assert_eq!(deleted.name, "doomed_vm");
        assert!(!volume_path.exists(), "the whole volume directory is gone");
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            1,
            "owned network is released exactly once"
        );
        let missing = sdk
            .run_repository(|repository| repository.find_microvm("doomed_vm"))
            .await
            .expect("lookup should work");
        assert!(missing.is_none(), "the record no longer resolves");
        let repeat = sdk.delete_microvm("doomed_vm").await;
        assert!(
            matches!(repeat, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "doomed_vm")
        );
        let _ = runtime;
    }

    #[tokio::test]
    async fn delete_converges_when_owned_files_are_already_absent() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "ghost_files_vm", false, None);
        std::fs::remove_dir_all(&stored.record.volume_path)
            .expect("fixture volume should be removable");
        let deleted = sdk
            .delete_microvm("ghost_files_vm")
            .await
            .expect("absent owned files converge");
        assert_eq!(deleted.name, "ghost_files_vm");
        let missing = sdk
            .run_repository(|repository| repository.find_microvm("ghost_files_vm"))
            .await
            .expect("lookup should work");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn delete_removes_an_incomplete_creation_leftover() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "half_gone_vm", false, None);
        delete_child_rows(&sdk, "half_gone_vm");
        let deleted = sdk
            .delete_microvm("half_gone_vm")
            .await
            .expect("incomplete leftover should delete");
        assert_eq!(deleted.name, "half_gone_vm");
        let missing = sdk
            .run_repository(|repository| repository.find_microvm("half_gone_vm"))
            .await
            .expect("lookup should work");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn delete_keeps_shared_artifacts_and_other_vms_intact() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "first_vm", false, None);
        let survivor = start_fixture_vm(&sdk, "second_vm", false, None);
        let survivor_volume = survivor.record.volume_path.clone();
        let kernel_path = sdk.home.join("artifacts/kernels/linux-test-x86_64/vmlinux");
        let deleted = sdk
            .delete_microvm("first_vm")
            .await
            .expect("first VM should delete");
        assert_eq!(deleted.name, "first_vm");
        assert!(kernel_path.is_file(), "shared kernel is preserved");
        assert!(
            survivor_volume.join("rootfs.ext4").is_file(),
            "surviving VM files are intact"
        );
        let survivor_still_there = sdk
            .run_repository(|repository| repository.find_microvm("second_vm"))
            .await
            .expect("lookup should work");
        assert!(survivor_still_there.is_some());
    }

    #[tokio::test]
    async fn delete_refuses_an_escaping_volume_without_removing_anything() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "escaped_vm", false, None);
        let outside = sdk.home.join("outside-escape");
        std::fs::create_dir_all(&outside).expect("outside directory should be created");
        std::fs::write(outside.join("keep.txt"), b"keep").expect("outside file should be written");
        {
            use rusqlite::params;
            let database = sdk.home.join("state").join("inventory.db");
            let connection = rusqlite::Connection::open(&database).expect("inventory should open");
            connection
                .execute(
                    "UPDATE microvms SET volume_path = ?1, rootfs_path = ?2, socket_path = ?3 WHERE name = 'escaped_vm'",
                    params![
                        outside.to_string_lossy().into_owned(),
                        outside.join("rootfs.ext4").to_string_lossy().into_owned(),
                        outside.join("firecracker.sock").to_string_lossy().into_owned(),
                    ],
                )
                .expect("volume should be tampered");
        }
        let error = sdk
            .delete_microvm("escaped_vm")
            .await
            .expect_err("escaping volume should fail");
        assert!(
            matches!(error, SdkError::Credential { .. }),
            "tampered key paths fail the credential shape check, got: {error:?}"
        );
        assert!(
            outside.join("keep.txt").is_file(),
            "nothing outside the volume is removed"
        );
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            1,
            "network release precedes containment per the plan order"
        );
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("escaped_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is kept for repair");
        std::fs::remove_dir_all(&outside).expect("outside directory should be removed");
    }

    #[tokio::test]
    async fn delete_refuses_a_home_escaping_volume_without_removing_anything() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "home_escaped_vm", false, None);
        let outside = std::env::temp_dir().join(format!(
            "taumaru-delete-escape-{}-home-escaped-vm",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).expect("outside directory should be created");
        std::fs::write(outside.join("keep.txt"), b"keep").expect("outside file should be written");
        {
            use rusqlite::params;
            let database = sdk.home.join("state").join("inventory.db");
            let connection = rusqlite::Connection::open(&database).expect("inventory should open");
            connection
                .execute(
                    "UPDATE microvms SET volume_path = ?1, rootfs_path = ?2, socket_path = ?3 WHERE name = 'home_escaped_vm'",
                    params![
                        outside.to_string_lossy().into_owned(),
                        outside.join("rootfs.ext4").to_string_lossy().into_owned(),
                        outside.join("firecracker.sock").to_string_lossy().into_owned(),
                    ],
                )
                .expect("volume should be tampered");
            connection
                .execute(
                    "UPDATE vm_credentials SET private_key_path = ?1, public_key_path = ?2 WHERE microvm_id = (SELECT id FROM microvms WHERE name = 'home_escaped_vm')",
                    params![
                        outside.join("ssh/id_ed25519").to_string_lossy().into_owned(),
                        outside.join("ssh/id_ed25519.pub").to_string_lossy().into_owned(),
                    ],
                )
                .expect("credential paths should be tampered");
        }
        let error = sdk
            .delete_microvm("home_escaped_vm")
            .await
            .expect_err("home-escaping volume should fail");
        assert!(
            matches!(error, SdkError::StorageConflict { .. }),
            "home escape fails the volume containment check, got: {error:?}"
        );
        assert!(
            outside.join("keep.txt").is_file(),
            "nothing outside the home is removed"
        );
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            1,
            "network release precedes containment per the plan order"
        );
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("home_escaped_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is kept for repair");
        std::fs::remove_dir_all(&outside).expect("outside directory should be removed");
    }

    #[tokio::test]
    async fn delete_keeps_the_record_when_the_volume_cannot_be_removed() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, _network, _runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "locked_vm", false, None);
        std::fs::remove_dir_all(&stored.record.volume_path)
            .expect("fixture volume should be removable");
        std::fs::write(&stored.record.volume_path, b"not-a-directory")
            .expect("blocking file should be written");
        let error = sdk
            .delete_microvm("locked_vm")
            .await
            .expect_err("blocked volume should fail");
        assert!(matches!(error, SdkError::Filesystem { .. }));
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("locked_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is kept for retry");
        std::fs::remove_file(&stored.record.volume_path)
            .expect("blocking file should be removable");
        let deleted = sdk
            .delete_microvm("locked_vm")
            .await
            .expect("retry should converge");
        assert_eq!(deleted.name, "locked_vm");
    }

    #[tokio::test]
    async fn delete_refuses_a_running_vm_without_host_changes() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "live_vm", false, Some(4242));
        *runtime.live_socket.lock().expect("socket lock") = true;
        let volume_snapshot = stored.record.volume_path.clone();
        let rootfs_before =
            std::fs::read(volume_snapshot.join("rootfs.ext4")).expect("rootfs should be readable");
        let error = sdk
            .delete_microvm("live_vm")
            .await
            .expect_err("running VM should be refused");
        assert!(
            matches!(&error, SdkError::LifecycleConflict { name, state, operation } if name == "live_vm" && state == "running" && operation.contains("stop")),
            "stop-first refusal, got: {error:?}"
        );
        assert_eq!(
            std::fs::read(volume_snapshot.join("rootfs.ext4")).expect("rootfs should be readable"),
            rootfs_before,
            "owned files are unchanged"
        );
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            0,
            "no network release on refusal"
        );
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("live_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is unchanged");
        *runtime.live_socket.lock().expect("socket lock") = false;
        let deleted = sdk
            .delete_microvm("live_vm")
            .await
            .expect("stop-then-delete succeeds");
        assert_eq!(deleted.name, "live_vm");
    }

    #[tokio::test]
    async fn delete_propagates_an_unprobable_socket_without_deleting() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, runtime) = test_sdk(false);
        let stored = start_fixture_vm(&sdk, "unprobable_vm", false, None);
        runtime
            .socket_errors
            .lock()
            .expect("socket error lock")
            .insert(stored.record.socket_path.to_string_lossy().into_owned());
        let error = sdk
            .delete_microvm("unprobable_vm")
            .await
            .expect_err("unprobable socket should fail");
        assert!(
            matches!(error, SdkError::TemporaryRuntime { .. }),
            "probe error propagates, got: {error:?}"
        );
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            0,
            "no network release on probe failure"
        );
        assert!(
            stored.record.volume_path.exists(),
            "owned files are unchanged"
        );
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("unprobable_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is unchanged");
    }

    #[tokio::test]
    async fn delete_runs_one_deletion_sequence_for_concurrent_deletes() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "shared_delete_vm", false, None);
        let sdk = std::sync::Arc::new(sdk);
        let first = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.delete_microvm("shared_delete_vm").await })
        };
        let second = {
            let sdk = sdk.clone();
            tokio::spawn(async move { sdk.delete_microvm("shared_delete_vm").await })
        };
        let (first, second) = tokio::join!(first, second);
        let first = first.expect("first delete should join");
        let second = second.expect("second delete should join");
        let successes = [&first, &second]
            .iter()
            .filter(|result| result.is_ok())
            .count();
        assert_eq!(
            successes, 1,
            "exactly one delete wins; the loser sees NotFound, got: {first:?} / {second:?}"
        );
        for result in [&first, &second] {
            match result {
                Ok(deleted) => assert_eq!(deleted.name, "shared_delete_vm"),
                Err(SdkError::NotFound { kind, id }) => {
                    assert_eq!(kind, "MicroVM");
                    assert_eq!(id, "shared_delete_vm");
                }
                Err(other) => panic!("unexpected concurrent outcome: {other:?}"),
            }
        }
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            1,
            "one deletion sequence releases the network once"
        );
    }

    #[tokio::test]
    async fn delete_skips_absent_network_items_and_keeps_the_record_on_failure() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "flaky_net_vm", false, None);
        network
            .delete_cleanup_results
            .lock()
            .expect("delete cleanup lock")
            .push(Err("injected release failure".to_owned()));
        let error = sdk
            .delete_microvm("flaky_net_vm")
            .await
            .expect_err("unreleasable network should fail");
        assert!(
            matches!(error, SdkError::Network { .. }),
            "present-but-unreleasable item is typed, got: {error:?}"
        );
        let still_there = sdk
            .run_repository(|repository| repository.find_microvm("flaky_net_vm"))
            .await
            .expect("lookup should work");
        assert!(still_there.is_some(), "the record is kept for retry");
        assert!(
            still_there
                .expect("record should exist")
                .record
                .volume_path
                .exists(),
            "the volume is untouched when network release fails first"
        );
        let deleted = sdk
            .delete_microvm("flaky_net_vm")
            .await
            .expect("retry should converge");
        assert_eq!(deleted.name, "flaky_net_vm");
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            2,
            "retry releases the network again"
        );
    }

    #[tokio::test]
    async fn delete_scopes_network_release_to_exactly_this_vm() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        start_fixture_vm(&sdk, "netted_first_vm", false, None);
        let survivor = start_fixture_vm(&sdk, "netted_second_vm", false, None);
        let deleted = sdk
            .delete_microvm("netted_first_vm")
            .await
            .expect("first VM should delete");
        assert_eq!(deleted.name, "netted_first_vm");
        assert_eq!(
            network.delete_cleanup_calls.load(Ordering::Relaxed),
            1,
            "only the deleted VM releases its network"
        );
        let survivor_still_there = sdk
            .run_repository(|repository| repository.find_microvm("netted_second_vm"))
            .await
            .expect("lookup should work");
        let survivor_still_there = survivor_still_there.expect("surviving VM record is intact");
        assert!(
            survivor_still_there.network.is_some(),
            "surviving VM keeps its network attachment"
        );
        assert!(
            survivor.record.volume_path.join("rootfs.ext4").is_file(),
            "surviving VM keeps its files"
        );
    }
}
