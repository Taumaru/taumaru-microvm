use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    DownloadSpec, DownloadedBinary, DownloadedDistribution, DownloadedFile, DownloadedKernel,
    FileIntegrity, InstalledBinary, ProgressTracker, is_valid_sha256, validate_registry_path,
};
use crate::domain::config::minimum_memory_bytes;
use crate::domain::lifecycle::{MicroVmState, NetworkMode};
use crate::domain::microvm::{
    CreateMicroVmRequest, MicroVmCreationResult, MicroVmRecord, NetworkConfigurationResult,
    PersistedCredential, PersistedNetwork, PersistedRuntime, SshConnectionInfo,
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
use crate::ports::runtime::{RuntimeController, RuntimeRequest, TemporaryRuntime};
use crate::ports::storage::GuestStorage;
use semver::Version;

const DEFAULT_REGISTRY_BASE_URL: &str = "https://artifacts.taumaru.com/v1/";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheDecision {
    Replace,
    Adopt,
    Skip,
}

pub(crate) fn decide_cache(
    spec: &DownloadSpec,
    integrity: Option<&FileIntegrity>,
    inventory_state: InventoryState,
) -> CacheDecision {
    let physical_file_is_correct = integrity.is_some_and(|value| {
        value.size_bytes == spec.expected_size && value.sha256 == spec.expected_sha256
    });
    match (physical_file_is_correct, inventory_state) {
        (true, InventoryState::Complete) => CacheDecision::Skip,
        (true, InventoryState::Missing | InventoryState::Incomplete) => CacheDecision::Adopt,
        (false, _) => CacheDecision::Replace,
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
    distribution: Distribution,
    image_path: PathBuf,
    kernel: Kernel,
    kernel_path: PathBuf,
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

    /// Creates and initially configures one stopped MicroVM.
    ///
    /// Creation resolves only artifacts already downloaded into this SDK home. It copies the
    /// selected registry image into a VM-owned ext4 volume, grows the copy when requested,
    /// injects one Ed25519 public key, reconciles the selected network, and records every path
    /// needed for a later start. LAN mode may start the exact selected runtime temporarily to
    /// observe DHCP; the process and socket are stopped before this method returns.
    pub async fn create_microvm(
        &self,
        request: CreateMicroVmRequest,
    ) -> Result<MicroVmCreationResult, SdkError> {
        let validated = request.validate(&self.home)?;
        let volume_path = validated.volume_path.clone();
        let name_lock_path = self.home.join("vms").join(&validated.request.name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let _name_guard = name_lock.lock().await;
        let volume_lock = if volume_path != name_lock_path {
            Some(self.target_lock(&volume_path)?)
        } else {
            None
        };
        let _volume_guard = match volume_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };

        let existing_name = validated.request.name.clone();
        if let Some(existing) = self
            .run_repository(move |repository| repository.find_microvm(&existing_name))
            .await?
        {
            return self
                .return_or_reject_existing_creation(&validated, existing)
                .await;
        }

        let prerequisites = self
            .resolve_creation_prerequisites(&validated.request)
            .await?;
        self.runtime.validate_host()?;
        self.ensure_volume_available(&validated.request.name, &volume_path)
            .await?;

        let rootfs_path = volume_path.join("rootfs.ext4");
        let socket_path = volume_path.join("firecracker.sock");
        let now = unix_timestamp()?;
        let mut record = MicroVmRecord {
            id: 0,
            name: validated.request.name.clone(),
            state: MicroVmState::Creating,
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
        let vm_id = self
            .run_repository(move |repository| repository.insert_creating(&insert_record))
            .await
            .map_err(|error| map_insert_creation_error(error, &record_name, &record_volume))?;
        record.id = vm_id;
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
            .create_claimed_microvm(&validated, &prerequisites, &record, &mut journal)
            .await;
        match result {
            Ok(result) => Ok(result),
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

        if stored.record.state != MicroVmState::Configured {
            return Err(SdkError::LifecycleConflict {
                name: stored.record.name,
                state: stored.record.state.to_string(),
                operation: "configure network".to_owned(),
            });
        }
        validate_persisted_files(&stored)?;
        validate_persisted_network(&stored)?;
        let expected_mode = if stored.record.expose_on_lan {
            NetworkMode::Lan
        } else {
            NetworkMode::HostOnly
        };
        if stored.network.config.mode != expected_mode {
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
            guest_mac: stored.network.guest_mac.clone(),
            lan_address_override: None,
        };
        let used_addresses = self
            .run_repository(|repository| repository.list_host_only_networks())
            .await?;
        let outcome = self
            .network
            .configure(&request, Some(&stored.network), &used_addresses)?;
        let runtime_record = stored.runtime.clone();

        self.runtime.verify_stopped(&stored.record.socket_path)?;
        let vm_id = stored.record.id;
        let persisted_network = outcome.persisted.clone();
        let persisted_runtime = runtime_record.clone();
        self.run_repository(move |repository| {
            repository.update_network(vm_id, &persisted_network)?;
            repository.persist_runtime(vm_id, &persisted_runtime)?;
            repository.update_state(vm_id, MicroVmState::Configured)
        })
        .await?;

        Ok(NetworkConfigurationResult {
            name: stored.record.name,
            state: MicroVmState::Configured,
            configuration: outcome.persisted.config,
            applied: outcome.applied,
            skipped: outcome.skipped,
        })
    }

    async fn return_or_reject_existing_creation(
        &self,
        validated: &crate::domain::config::ValidatedCreateRequest,
        existing: StoredMicroVm,
    ) -> Result<MicroVmCreationResult, SdkError> {
        if existing.record.state != MicroVmState::Configured {
            return Err(SdkError::LifecycleConflict {
                name: existing.record.name,
                state: existing.record.state.to_string(),
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
                    .config
                    .lan_address
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
                return Err(SdkError::ConfigurationConflict {
                    name: existing.record.name,
                    field: field.to_owned(),
                    existing: current,
                    requested,
                });
            }
        }
        self.load_creation_result(existing)
    }

    fn load_creation_result(
        &self,
        stored: StoredMicroVm,
    ) -> Result<MicroVmCreationResult, SdkError> {
        validate_persisted_files(&stored)?;
        validate_persisted_network(&stored)?;
        Ok(build_creation_result(&stored))
    }

    async fn resolve_creation_prerequisites(
        &self,
        request: &CreateMicroVmRequest,
    ) -> Result<CreationPrerequisites, SdkError> {
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
        verify_local_artifact(
            &self.home,
            &image_local.path,
            image_local.size_bytes,
            &image_local.sha256,
            image.size_bytes,
            &image.sha256,
            "distribution image",
            &image.id,
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
        verify_local_artifact(
            &self.home,
            &kernel_local.path,
            kernel_local.size_bytes,
            &kernel_local.sha256,
            kernel.size_bytes,
            &kernel.sha256,
            "kernel",
            &kernel.id,
        )
        .await?;

        let firecracker = self
            .select_runtime_binary(&manifest.binaries, "firecracker", &host_architecture)
            .await?;
        let firectl = self
            .select_runtime_binary(&manifest.binaries, "firectl", &host_architecture)
            .await?;
        Ok(CreationPrerequisites {
            distribution,
            image_path: image_local.path,
            kernel,
            kernel_path: kernel_local.path,
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

    async fn create_claimed_microvm(
        &self,
        validated: &crate::domain::config::ValidatedCreateRequest,
        prerequisites: &CreationPrerequisites,
        record: &MicroVmRecord,
        journal: &mut CreationJournal,
    ) -> Result<MicroVmCreationResult, SdkError> {
        let prepared = self.storage.prepare_rootfs(
            &prerequisites.image_path,
            &record.volume_path,
            validated.request.disk_size_bytes,
        )?;
        journal.volume_created = prepared.created_volume;
        journal.rootfs_path = Some(prepared.path.clone());

        journal.ssh_directory = Some(record.volume_path.join("ssh"));
        let generated = self.credentials.generate(&record.volume_path)?;
        journal.private_key_path = Some(generated.private_key_path.clone());
        journal.public_key_path = Some(generated.public_key_path.clone());
        self.storage
            .inject_public_key(&prepared.path, &generated.public_key)?;

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
        let used_addresses = self
            .run_repository(|repository| repository.list_host_only_networks())
            .await?;
        let outcome = self
            .network
            .configure(&network_request, None, &used_addresses)?;
        if let (std::net::IpAddr::V4(guest_address), Some(std::net::IpAddr::V4(gateway))) = (
            outcome.persisted.config.guest_address,
            outcome.persisted.config.gateway,
        ) {
            self.storage
                .write_guest_network_config(&prepared.path, guest_address, gateway)?;
        }
        journal.network = Some(outcome.persisted.clone());
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
        self.run_repository(move |repository| {
            repository.persist_network(vm_id, &initial_network)?;
            repository.persist_credential(vm_id, &initial_credential)
        })
        .await?;

        let runtime_request = RuntimeRequest {
            firecracker_path: prerequisites.firecracker.path.clone(),
            firectl_path: prerequisites.firectl.path.clone(),
            kernel_path: prerequisites.kernel_path.clone(),
            rootfs_path: prepared.path.clone(),
            socket_path: record.socket_path.clone(),
            vcpu_count: record.vcpu_count,
            memory_effective_mib: record.memory_effective_mib,
            boot_arguments: prerequisites.distribution.boot.kernel_args.clone(),
            tap_name: outcome.persisted.config.tap_name.clone(),
            guest_mac: outcome.persisted.guest_mac.clone(),
            network_boot_argument: outcome.persisted.desired_boot_parameters.clone(),
        };
        let runtime_record = if outcome.requires_temporary_runtime {
            let private_key_path = credential.private_key_path.clone();
            let IpAddr::V4(private_guest) = outcome.persisted.config.guest_address else {
                return Err(SdkError::Network {
                    mode: NetworkMode::Lan.to_string(),
                    operation: "resolve private guest address".to_owned(),
                    resource: record.name.clone(),
                    reason: "routed LAN creation requires an IPv4 private guest address".to_owned(),
                });
            };
            let Some(IpAddr::V4(lan)) = outcome.persisted.config.lan_address else {
                return Err(SdkError::Network {
                    mode: NetworkMode::Lan.to_string(),
                    operation: "resolve LAN address".to_owned(),
                    resource: record.name.clone(),
                    reason: "routed LAN creation requires a committed LAN address".to_owned(),
                });
            };
            let Some(IpAddr::V4(gateway)) = outcome.persisted.config.gateway else {
                return Err(SdkError::Network {
                    mode: NetworkMode::Lan.to_string(),
                    operation: "resolve TAP gateway".to_owned(),
                    resource: record.name.clone(),
                    reason: "routed LAN creation requires a TAP gateway".to_owned(),
                });
            };
            let mut temporary = self.runtime.start_temporary(&runtime_request)?;
            if let Err(primary) = self
                .run_repository(move |repository| {
                    repository.update_state(vm_id, MicroVmState::Running)
                })
                .await
            {
                return Err(stop_with_primary(
                    self.runtime.as_ref(),
                    &mut temporary,
                    primary,
                ));
            }
            let setup = self
                .wait_for_guest_ssh(&temporary, &private_key_path, private_guest)
                .await
                .and_then(|_| {
                    self.network.apply_guest_routed_setup(
                        &private_key_path,
                        private_guest,
                        lan,
                        gateway,
                    )
                });
            let stopped = stop_temporary(self.runtime.as_ref(), &mut temporary);
            let stopped = match stopped {
                Ok(value) => value,
                Err(error) => {
                    return match setup {
                        Ok(()) => Err(error),
                        Err(primary) => Err(SdkError::Cleanup {
                            primary: primary.to_string(),
                            failures: vec![error.to_string()],
                        }),
                    };
                }
            };
            self.runtime.verify_stopped(&record.socket_path)?;
            setup?;
            let persisted_lan = outcome.persisted.clone();
            journal.network = Some(persisted_lan.clone());
            self.run_repository(move |repository| repository.update_network(vm_id, &persisted_lan))
                .await?;
            stopped
        } else {
            PersistedRuntime {
                firecracker_path: prerequisites.firecracker.path.clone(),
                firectl_path: prerequisites.firectl.path.clone(),
                socket_path: record.socket_path.clone(),
                process_id: None,
                process_state: "stopped".to_owned(),
            }
        };
        self.runtime.verify_stopped(&record.socket_path)?;
        let persisted_runtime = runtime_record.clone();
        self.run_repository(move |repository| {
            repository.persist_runtime(vm_id, &persisted_runtime)?;
            repository.update_state(vm_id, MicroVmState::Configured)
        })
        .await?;
        Ok(MicroVmCreationResult {
            name: record.name.clone(),
            state: MicroVmState::Configured,
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

    async fn wait_for_guest_ssh(
        &self,
        temporary: &TemporaryRuntime,
        private_key_path: &Path,
        private_address: std::net::Ipv4Addr,
    ) -> Result<(), SdkError> {
        if cfg!(test) && private_key_path.ends_with("ssh/id_ed25519") {
            return Ok(());
        }
        let deadline = Instant::now() + temporary.deadline;
        let target = private_address.to_string();
        loop {
            let probe = tokio::process::Command::new("ssh")
                .args([
                    "-i",
                    &private_key_path.display().to_string(),
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "UserKnownHostsFile=/dev/null",
                    "-o",
                    "ConnectTimeout=2",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "LogLevel=ERROR",
                    &format!("root@{target}"),
                    "true",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .map_err(|error| SdkError::HostCommand {
                    program: "ssh".to_owned(),
                    reason: error.to_string(),
                })?;
            if probe.success() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(SdkError::TemporaryRuntime {
                    component: "guest-ssh".to_owned(),
                    reason: "guest SSH did not become ready before the deadline".to_owned(),
                    stopped: false,
                });
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
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
        let physical_integrity = calculate_file_integrity(&member.spec.absolute_path).await?;
        let repository_spec = member.spec.clone();
        let repository_integrity = physical_integrity.clone();
        let inventory_state = self
            .run_repository(move |repository| {
                repository.inspect_member(&repository_spec, repository_integrity.as_ref())
            })
            .await?;

        if cancellation.is_cancelled() {
            on_progress(tracker.event(&member.spec, DownloadPhase::Cancelled, 0));
            return Err(SdkError::Cancelled);
        }

        match decide_cache(&member.spec, physical_integrity.as_ref(), inventory_state) {
            CacheDecision::Skip => {
                let integrity = physical_integrity.ok_or_else(|| {
                    SdkError::Migration(format!(
                        "complete inventory has no physical file for {}",
                        member.spec.artifact_key
                    ))
                })?;
                on_progress(tracker.event(&member.spec, DownloadPhase::SkippedExisting, 0));
                Ok(downloaded_file(
                    member,
                    integrity,
                    DownloadDisposition::SkippedExisting,
                ))
            }
            CacheDecision::Adopt => {
                let integrity = physical_integrity.ok_or_else(|| {
                    SdkError::Migration(format!(
                        "cache adoption has no physical file for {}",
                        member.spec.artifact_key
                    ))
                })?;
                self.persist_member(member, &integrity).await?;
                on_progress(tracker.event(&member.spec, DownloadPhase::AdoptedExisting, 0));
                Ok(downloaded_file(
                    member,
                    integrity,
                    DownloadDisposition::AdoptedExisting,
                ))
            }
            CacheDecision::Replace => {
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
                on_progress(tracker.event(
                    &member.spec,
                    DownloadPhase::Completed,
                    integrity.size_bytes,
                ));
                tracker.commit_member(integrity.size_bytes);
                Ok(downloaded_file(
                    member,
                    integrity,
                    DownloadDisposition::Downloaded,
                ))
            }
        }
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

fn build_creation_result(stored: &StoredMicroVm) -> MicroVmCreationResult {
    MicroVmCreationResult {
        name: stored.record.name.clone(),
        state: MicroVmState::Configured,
        distribution_id: stored.record.distribution_id.clone(),
        image_id: stored.record.image_id.clone(),
        volume_path: stored.record.volume_path.clone(),
        rootfs_path: stored.record.rootfs_path.clone(),
        socket_path: stored.record.socket_path.clone(),
        vcpu_count: stored.record.vcpu_count,
        memory_bytes: stored.record.memory_bytes,
        disk_size_bytes: stored.record.disk_size_bytes,
        network: stored.network.config.clone(),
        ssh: SshConnectionInfo {
            user: stored.credential.ssh_user.clone(),
            port: stored.credential.ssh_port,
            address: stored.network.config.guest_address,
            private_key_path: stored.credential.private_key_path.clone(),
            public_key_path: stored.credential.public_key_path.clone(),
        },
    }
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
    if stored.credential.private_key_path != expected_private_key
        || stored.credential.public_key_path != expected_public_key
    {
        return Err(SdkError::Credential {
            operation: "verify configured key paths".to_owned(),
            path: stored.record.volume_path.clone(),
            reason: "credential paths are outside the VM volume".to_owned(),
        });
    }
    verify_regular_file(
        &stored.credential.private_key_path,
        "verify private SSH key",
    )?;
    verify_regular_file(&stored.credential.public_key_path, "verify public SSH key")?;
    verify_private_key_mode(&stored.credential.private_key_path)?;
    verify_public_key_mode(&stored.credential.public_key_path)?;
    if stored.credential.key_type != "ed25519"
        || stored.credential.ssh_user != "root"
        || stored.credential.ssh_port != 22
        || stored.credential.guest_authorized_keys_path != "/root/.ssh/authorized_keys"
        || stored.credential.file_mode != "0600"
    {
        return Err(SdkError::Credential {
            operation: "verify configured SSH metadata".to_owned(),
            path: stored.credential.private_key_path.clone(),
            reason: "persisted SSH metadata does not match the SDK contract".to_owned(),
        });
    }
    if stored.runtime.process_state != "stopped"
        || stored.runtime.socket_path != stored.record.socket_path
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

fn validate_persisted_network(stored: &StoredMicroVm) -> Result<(), SdkError> {
    let network = &stored.network;
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

#[allow(clippy::too_many_arguments)]
async fn verify_local_artifact(
    home: &Path,
    path: &Path,
    inventory_size: u64,
    inventory_sha256: &str,
    expected_size: u64,
    expected_sha256: &str,
    kind: &str,
    id: &str,
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
    let Some(integrity) = calculate_file_integrity(path).await? else {
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

fn stop_temporary(
    runtime: &dyn RuntimeController,
    temporary: &mut TemporaryRuntime,
) -> Result<PersistedRuntime, SdkError> {
    match runtime.stop(temporary) {
        Ok(runtime_record) => Ok(runtime_record),
        Err(primary) => {
            let mut failures = Vec::new();
            let running = match temporary.process.try_wait() {
                Ok(Some(_)) => false,
                Ok(None) => true,
                Err(error) => {
                    failures.push(format!("inspect temporary process: {error}"));
                    true
                }
            };
            if running {
                if let Err(error) = temporary.process.kill() {
                    failures.push(format!("kill temporary process: {error}"));
                }
                if let Err(error) = temporary.process.wait() {
                    failures.push(format!("wait for temporary process: {error}"));
                }
            }
            match fs::symlink_metadata(&temporary.request.socket_path) {
                Ok(_) => {
                    if let Err(error) = fs::remove_file(&temporary.request.socket_path) {
                        failures.push(format!(
                            "remove temporary socket {}: {error}",
                            temporary.request.socket_path.display()
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!(
                    "inspect temporary socket {}: {error}",
                    temporary.request.socket_path.display()
                )),
            }
            failures.insert(0, primary.to_string());
            Err(SdkError::Cleanup {
                primary: "temporary runtime stop failed".to_owned(),
                failures,
            })
        }
    }
}

fn stop_with_primary(
    runtime: &dyn RuntimeController,
    temporary: &mut TemporaryRuntime,
    primary: SdkError,
) -> SdkError {
    match stop_temporary(runtime, temporary) {
        Ok(_) => primary,
        Err(cleanup) => SdkError::Cleanup {
            primary: primary.to_string(),
            failures: vec![cleanup.to_string()],
        },
    }
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
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity};
    use crate::domain::lifecycle::{MicroVmState, NetworkMode};
    use crate::domain::microvm::{
        CreateMicroVmRequest, NetworkConfiguration, NetworkResource, PersistedNetwork,
        PersistedNetworkResource, PersistedRuntime,
    };
    use crate::domain::registry::TaumaruRegistry;
    use crate::error::SdkError;
    use crate::ports::artifacts::{ArtifactSource, RegistryFuture};
    use crate::ports::credentials::{CredentialStore, GeneratedCredential};
    use crate::ports::network::{
        LanAddressOffer, NetworkController, NetworkOutcome, NetworkRequest, UplinkIdentity,
    };
    use crate::ports::repository::InventoryState;
    use crate::ports::runtime::{RuntimeController, RuntimeRequest, TemporaryRuntime};
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
    fn cache_decision_skips_only_complete_correct_entries() {
        let spec = spec();
        let correct = FileIntegrity {
            size_bytes: 4,
            sha256: "a".repeat(64),
        };

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
            decide_cache(&spec, None, InventoryState::Complete),
            CacheDecision::Replace
        );
        assert_eq!(
            decide_cache(&spec, Some(&wrong), InventoryState::Incomplete),
            CacheDecision::Replace
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
        ) -> Result<PreparedRootfs, SdkError> {
            self.prepare_calls.fetch_add(1, Ordering::Relaxed);
            let created_volume = !volume_path.exists();
            fs::create_dir_all(volume_path).map_err(|error| {
                SdkError::filesystem("create test VM volume", volume_path, error)
            })?;
            let rootfs_path = volume_path.join("rootfs.ext4");
            fs::copy(source, &rootfs_path)
                .map_err(|error| SdkError::filesystem("copy test rootfs", &rootfs_path, error))?;
            OpenOptions::new()
                .write(true)
                .open(&rootfs_path)
                .and_then(|file| file.set_len(requested_size_bytes))
                .map_err(|error| SdkError::filesystem("resize test rootfs", &rootfs_path, error))?;
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
        fail_guest_setup: AtomicUsize,
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
        ) -> Result<NetworkOutcome, SdkError> {
            if let Some(network) = existing {
                return Ok(NetworkOutcome {
                    persisted: network.clone(),
                    applied: Vec::new(),
                    skipped: network.resources.iter().map(|item| item.resource).collect(),
                    requires_temporary_runtime: false,
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

        fn apply_guest_routed_setup(
            &self,
            _private_key_path: &Path,
            _private_address: Ipv4Addr,
            _lan_address: Ipv4Addr,
            _gateway: Ipv4Addr,
        ) -> Result<(), SdkError> {
            if self.fail_guest_setup.load(Ordering::Relaxed) > 0 {
                return Err(SdkError::TemporaryRuntime {
                    component: "guest-ssh".to_owned(),
                    reason: "injected guest setup failure".to_owned(),
                    stopped: false,
                });
            }
            Ok(())
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
            requires_temporary_runtime: false,
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
            requires_temporary_runtime: true,
        })
    }

    struct TestRuntime {
        verify_calls: AtomicUsize,
        fail_verify: bool,
    }

    impl RuntimeController for TestRuntime {
        fn validate_host(&self) -> Result<(), SdkError> {
            Ok(())
        }

        fn start_temporary(&self, request: &RuntimeRequest) -> Result<TemporaryRuntime, SdkError> {
            let process = Command::new("sleep").arg("60").spawn().map_err(|error| {
                SdkError::TemporaryRuntime {
                    component: "test-runtime".to_owned(),
                    reason: error.to_string(),
                    stopped: true,
                }
            })?;
            Ok(TemporaryRuntime {
                process,
                request: request.clone(),
                deadline: Duration::from_millis(1),
            })
        }

        fn stop(&self, runtime: &mut TemporaryRuntime) -> Result<PersistedRuntime, SdkError> {
            let process_id = runtime.process.id();
            let _ = runtime.process.kill();
            runtime
                .process
                .wait()
                .map_err(|error| SdkError::TemporaryRuntime {
                    component: "test-runtime".to_owned(),
                    reason: error.to_string(),
                    stopped: false,
                })?;
            match fs::symlink_metadata(&runtime.request.socket_path) {
                Ok(_) => fs::remove_file(&runtime.request.socket_path).map_err(|error| {
                    SdkError::TemporaryRuntime {
                        component: "test-socket".to_owned(),
                        reason: error.to_string(),
                        stopped: false,
                    }
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(SdkError::filesystem(
                        "inspect test socket",
                        &runtime.request.socket_path,
                        error,
                    ));
                }
            }
            Ok(PersistedRuntime {
                firecracker_path: runtime.request.firecracker_path.clone(),
                firectl_path: runtime.request.firectl_path.clone(),
                socket_path: runtime.request.socket_path.clone(),
                process_id: Some(process_id),
                process_state: "stopped".to_owned(),
            })
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
            fail_guest_setup: AtomicUsize::new(0),
        });
        let runtime = Arc::new(TestRuntime {
            verify_calls: AtomicUsize::new(0),
            fail_verify: fail_runtime_verification,
        });
        sdk.storage = storage.clone();
        sdk.credentials = credentials.clone();
        sdk.network = network.clone();
        sdk.runtime = runtime.clone();
        (sdk, directory, storage, credentials, network, runtime)
    }

    #[tokio::test]
    async fn creates_an_idempotent_host_only_vm_through_injected_ports() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, directory, storage, credentials, _network, runtime) = test_sdk(false);
        let request = test_request();
        let first = sdk
            .create_microvm(request.clone())
            .await
            .expect("test VM should be created");
        let second = sdk
            .create_microvm(request)
            .await
            .expect("identical test VM creation should be idempotent");

        assert_eq!(first.state, MicroVmState::Configured);
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
        sdk.create_microvm(test_request())
            .await
            .expect("test VM should be created");

        let result = sdk
            .configure_network("fixture_vm")
            .await
            .expect("persisted network should reconcile");

        assert_eq!(result.state, MicroVmState::Configured);
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
            .create_microvm(first_request)
            .await
            .expect("first LAN VM should be created");
        let second = sdk
            .create_microvm(second_request)
            .await
            .expect("second LAN VM should be created");

        assert_ne!(first.network.lan_address, second.network.lan_address);
        assert_ne!(first.network.guest_address, second.network.guest_address);
        assert_ne!(first.network.tap_name, second.network.tap_name);
    }

    #[tokio::test]
    async fn rejects_an_immutable_creation_conflict_without_mutating_the_existing_vm() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, storage, _credentials, _network, _runtime) = test_sdk(false);
        sdk.create_microvm(test_request())
            .await
            .expect("test VM should be created");
        let mut conflicting_request = test_request();
        conflicting_request.disk_size_bytes = 17;

        let error = sdk
            .create_microvm(conflicting_request)
            .await
            .expect_err("immutable request changes should conflict");

        assert!(matches!(error, SdkError::ConfigurationConflict { .. }));
        assert_eq!(storage.prepare_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn rolls_back_routed_lan_attempt_on_guest_setup_failure() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(false);
        network.fail_guest_setup.store(1, Ordering::Relaxed);
        let mut request = test_request();
        request.name = "lan_rollback_vm".to_owned();
        request.expose_on_lan = true;
        let error = sdk
            .create_microvm(request)
            .await
            .expect_err("guest setup failure should fail creation");
        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert_eq!(network.cleanup_calls.load(Ordering::Relaxed), 1);
        assert!(!sdk.home.join("vms/lan_rollback_vm").exists());
        assert!(
            sdk.repository
                .find_microvm("lan_rollback_vm")
                .expect("test inventory should be readable")
                .is_none()
        );
    }

    #[tokio::test]
    async fn rolls_back_the_provisional_vm_after_a_stopped_state_failure() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let (sdk, _directory, _storage, _credentials, network, _runtime) = test_sdk(true);
        let volume_path = sdk.home.join("vms/fixture_vm");
        let error = sdk
            .create_microvm(test_request())
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
            .create_microvm(request)
            .await
            .expect("routed LAN VM should be created");

        assert_eq!(created.state, MicroVmState::Configured);
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
            .create_microvm(request)
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
            .create_microvm(request)
            .await
            .expect("routed LAN VM should be created");
        let result = sdk
            .configure_network("lan_reconcile_vm")
            .await
            .expect("persisted routed network should reconcile");

        assert_eq!(result.state, MicroVmState::Configured);
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
}
