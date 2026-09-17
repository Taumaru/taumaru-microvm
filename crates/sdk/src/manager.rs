use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::fs as async_fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::adapters::persistence::sqlite::SqliteRepository;
use crate::adapters::registry::taumaru::TaumaruRegistryClient;
use crate::domain::artifact::{
    ArtifactKind, DownloadCancellation, DownloadDisposition, DownloadPhase, DownloadProgress,
    DownloadSpec, DownloadedBinary, DownloadedDistribution, DownloadedFile, DownloadedKernel,
    FileIntegrity, InstalledBinary, ProgressTracker, is_valid_sha256, validate_registry_path,
};
use crate::domain::registry::{
    BinaryFile, BinaryPackage, Distribution, DistributionImage, Kernel, TaumaruRegistry,
};
use crate::error::SdkError;
use crate::ports::artifacts::ArtifactSource;
use crate::ports::repository::{ArtifactRepository, InventoryState};

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
    pub(crate) repository: Arc<SqliteRepository>,
    pub(crate) registry: Arc<TaumaruRegistryClient>,
    target_locks: Arc<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
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
        F: FnOnce(&SqliteRepository) -> Result<T, SdkError> + Send + 'static,
    {
        let repository = Arc::clone(&self.repository);
        tokio::task::spawn_blocking(move || operation(&repository)).await?
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
                normalized.pop();
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
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity};
    use crate::ports::repository::InventoryState;

    use super::{CacheDecision, decide_cache, normalize_home_path, path_is_below_home};

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
}
