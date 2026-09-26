use std::collections::HashSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::domain::artifact::{DownloadSpec, FileIntegrity, InstalledBinary};
use crate::domain::autostart::{AutostartPolicy, AutostartSettings};
use crate::domain::microvm::{
    MicroVmRecord, NetworkConfiguration, PersistedCredential, PersistedNetwork, PersistedRuntime,
};
use crate::domain::registry::{BinaryFile, BinaryPackage, Distribution, DistributionImage, Kernel};
use crate::error::SdkError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InventoryState {
    Missing,
    Incomplete,
    Complete,
}

pub(crate) trait ArtifactRepository: Send + Sync {
    fn inspect_member(
        &self,
        spec: &DownloadSpec,
        integrity: Option<&FileIntegrity>,
    ) -> Result<InventoryState, SdkError>;

    fn remove_member(&self, spec: &DownloadSpec) -> Result<(), SdkError>;

    fn persist_kernel(
        &self,
        kernel: &Kernel,
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError>;

    fn persist_binary_file(
        &self,
        package: &BinaryPackage,
        file: &BinaryFile,
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError>;

    fn persist_distribution_image(
        &self,
        distribution: &Distribution,
        image: &DistributionImage,
        kernels: &[Kernel],
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError>;

    fn resolve_binary(
        &self,
        package_id: &str,
        component_name: &str,
    ) -> Result<InstalledBinary, SdkError>;

    fn resolve_kernel(&self, kernel_id: &str) -> Result<LocalArtifact, SdkError>;

    fn resolve_distribution_image(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<LocalArtifact, SdkError>;

    fn list_ready_distribution_images(&self) -> Result<Vec<(String, String)>, SdkError>;

    #[allow(dead_code)]
    fn list_installed_binaries(&self) -> Result<Vec<InstalledBinary>, SdkError>;

    fn list_prune_references(&self) -> Result<PruneReferences, SdkError>;

    fn list_prunable_kernels(&self) -> Result<Vec<PrunableKernel>, SdkError>;

    fn list_prunable_images(&self) -> Result<Vec<PrunableImage>, SdkError>;

    fn list_orphan_artifact_downloads(&self) -> Result<Vec<OrphanArtifactDownload>, SdkError>;
    fn delete_kernel_if_unreferenced(&self, kernel_id: &str) -> Result<bool, SdkError>;

    fn delete_image_if_unreferenced(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<bool, SdkError>;

    fn delete_orphan_download_if_unreferenced(&self, artifact_key: &str) -> Result<bool, SdkError>;
}

/// A verified artifact relationship loaded from the local inventory.
#[derive(Clone, Debug)]
pub(crate) struct LocalArtifact {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

/// The artifact references held by every existing MicroVM record.
///
/// Built from one light inventory read. Existence of the MicroVM row alone
/// protects an artifact: lifecycle state, process liveness, socket
/// responsiveness, and creation completeness never influence membership.
#[derive(Clone, Debug, Default)]
pub(crate) struct PruneReferences {
    /// Referenced kernel registry IDs.
    pub kernels: HashSet<String>,
    /// Referenced `(distribution_id, image_id)` pairs.
    pub images: HashSet<(String, String)>,
}

impl PruneReferences {
    /// Returns whether any existing MicroVM references this kernel.
    pub(crate) fn kernel_referenced(&self, kernel_id: &str) -> bool {
        self.kernels.contains(kernel_id)
    }

    /// Returns whether any existing MicroVM references this distribution image.
    pub(crate) fn image_referenced(&self, distribution_id: &str, image_id: &str) -> bool {
        self.images
            .contains(&(distribution_id.to_owned(), image_id.to_owned()))
    }
}

/// One recorded kernel download that may be pruned.
#[derive(Clone, Debug)]
pub(crate) struct PrunableKernel {
    /// Kernel registry ID.
    pub registry_id: String,
    /// Recorded absolute file path.
    pub absolute_path: PathBuf,
    /// Recorded byte size for preview estimates.
    pub size_bytes: u64,
}

/// One recorded distribution image download that may be pruned.
#[derive(Clone, Debug)]
pub(crate) struct PrunableImage {
    /// Owning distribution registry ID.
    pub distribution_id: String,
    /// Image registry ID within its distribution.
    pub image_id: String,
    /// Recorded absolute file path.
    pub absolute_path: PathBuf,
    /// Recorded byte size for preview estimates.
    pub size_bytes: u64,
}

/// One orphan `downloads` row of kernel or image type with no member row.
///
/// Orphans are the only sense in which incomplete or failed downloads exist in
/// the schema: leftovers from a crash between the download insert and the
/// member insert, or from a cancelled transfer that committed the download row.
#[derive(Clone, Debug)]
pub(crate) struct OrphanArtifactDownload {
    /// Stable artifact key (`kernel:{id}` or `distribution_image:{dist}:{image}`).
    pub artifact_key: String,
    /// Inventory artifact type (`kernel` or `distribution_image`).
    pub artifact_type: String,
    /// Recorded absolute file path.
    pub absolute_path: PathBuf,
    /// Recorded byte size for preview estimates.
    pub size_bytes: u64,
}

/// The prune identity recovered from an orphan artifact key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OrphanIdentity {
    /// A kernel orphan with its registry ID.
    Kernel {
        /// Kernel registry ID.
        kernel_id: String,
    },
    /// A distribution image orphan with its owning distribution and image IDs.
    DistributionImage {
        /// Owning distribution registry ID.
        distribution_id: String,
        /// Image registry ID within its distribution.
        image_id: String,
    },
}

impl OrphanArtifactDownload {
    /// Recovers the prune identity from the stable artifact key.
    ///
    /// Returns `None` when the key does not match the recorded artifact type.
    /// Unparseable orphans are never deleted; they become failure entries.
    pub(crate) fn parse_identity(&self) -> Option<OrphanIdentity> {
        if self.artifact_type == "kernel" {
            let kernel_id = self.artifact_key.strip_prefix("kernel:")?;
            if kernel_id.is_empty() {
                return None;
            }
            Some(OrphanIdentity::Kernel {
                kernel_id: kernel_id.to_owned(),
            })
        } else if self.artifact_type == "distribution_image" {
            let rest = self.artifact_key.strip_prefix("distribution_image:")?;
            let (distribution_id, image_id) = rest.split_once(':')?;
            if distribution_id.is_empty() || image_id.is_empty() {
                return None;
            }
            Some(OrphanIdentity::DistributionImage {
                distribution_id: distribution_id.to_owned(),
                image_id: image_id.to_owned(),
            })
        } else {
            None
        }
    }
}

/// Durable VM state and its child records loaded from SQLite.
#[derive(Clone, Debug)]
pub(crate) struct StoredMicroVm {
    pub record: MicroVmRecord,
    pub network: Option<PersistedNetwork>,
    pub credential: Option<PersistedCredential>,
    pub runtime: Option<PersistedRuntime>,
}

impl StoredMicroVm {
    pub(crate) fn is_complete(&self) -> bool {
        self.network.is_some() && self.credential.is_some() && self.runtime.is_some()
    }

    pub(crate) fn require_complete(&self, operation: &str) -> Result<(), SdkError> {
        if self.is_complete() {
            Ok(())
        } else {
            Err(SdkError::LifecycleConflict {
                name: self.record.name.clone(),
                state: "creation incomplete".to_owned(),
                operation: operation.to_owned(),
            })
        }
    }

    pub(crate) fn require_full(
        &self,
        operation: &str,
    ) -> Result<(&PersistedNetwork, &PersistedCredential, &PersistedRuntime), SdkError> {
        self.require_complete(operation)?;
        match (&self.network, &self.credential, &self.runtime) {
            (Some(network), Some(credential), Some(runtime)) => Ok((network, credential, runtime)),
            _ => Err(SdkError::LifecycleConflict {
                name: self.record.name.clone(),
                state: "creation incomplete".to_owned(),
                operation: operation.to_owned(),
            }),
        }
    }
}

/// Portable distribution and kernel boot metadata required to recreate a VM.
#[derive(Clone, Debug)]
pub(crate) struct SnapshotStoredMetadata {
    pub distribution_name: String,
    pub distribution_version: String,
    pub root_device: String,
    pub kernel_args: Vec<String>,
    pub image_sha256: String,
    pub guest_architecture: String,
    pub kernel: Kernel,
}

#[derive(Clone, Debug)]
pub(crate) struct RestoreJournal {
    pub operation_id: String,
    pub vm_name: String,
    pub staging_path: PathBuf,
    pub volume_path: PathBuf,
    pub volume_created: bool,
    pub kernel_path: PathBuf,
    pub kernel_created: bool,
    pub network: Option<PersistedNetwork>,
    pub progress_state: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RestoredSnapshotMetadata {
    pub distribution_name: String,
    pub distribution_version: String,
    pub root_device: String,
    pub kernel_args: Vec<String>,
    pub image_sha256: String,
    pub guest_architecture: String,
}

pub(crate) struct RestoredMicroVmCommit<'a> {
    pub record: &'a MicroVmRecord,
    pub network: &'a PersistedNetwork,
    pub credential: &'a PersistedCredential,
    pub runtime: &'a PersistedRuntime,
    pub kernel: &'a Kernel,
    pub kernel_spec: &'a DownloadSpec,
    pub kernel_integrity: &'a FileIntegrity,
    pub snapshot_metadata: &'a RestoredSnapshotMetadata,
    pub operation_id: &'a str,
}

/// Local inventory and lifecycle persistence for MicroVM records.
pub(crate) trait MicroVmRepository: Send + Sync {
    fn find_microvm(&self, name: &str) -> Result<Option<StoredMicroVm>, SdkError>;

    fn snapshot_metadata(&self, name: &str) -> Result<SnapshotStoredMetadata, SdkError>;

    fn record_restore_journal(&self, journal: &RestoreJournal) -> Result<(), SdkError>;

    fn list_restore_journals(&self, vm_name: &str) -> Result<Vec<RestoreJournal>, SdkError>;

    fn delete_restore_journal(&self, operation_id: &str) -> Result<(), SdkError>;

    fn commit_restored_microvm(&self, commit: RestoredMicroVmCommit<'_>) -> Result<i64, SdkError>;

    fn list_stored_microvms(&self) -> Result<Vec<StoredMicroVm>, SdkError>;

    fn find_volume_owner(&self, volume_path: &Path) -> Result<Option<String>, SdkError>;

    fn list_host_only_networks(&self) -> Result<Vec<(String, IpAddr, String)>, SdkError>;

    fn list_lan_addresses(&self) -> Result<Vec<(String, IpAddr, String)>, SdkError>;

    fn insert_microvm(&self, record: &MicroVmRecord) -> Result<i64, SdkError>;

    fn persist_network(&self, vm_id: i64, network: &PersistedNetwork) -> Result<(), SdkError>;

    fn persist_credential(
        &self,
        vm_id: i64,
        credential: &PersistedCredential,
    ) -> Result<(), SdkError>;

    fn persist_runtime(&self, vm_id: i64, runtime: &PersistedRuntime) -> Result<(), SdkError>;

    fn delete_microvm(&self, vm_id: i64) -> Result<(), SdkError>;

    fn update_network(&self, vm_id: i64, network: &PersistedNetwork) -> Result<(), SdkError>;

    fn bridge_has_other_references(
        &self,
        vm_id: i64,
        bridge_name: &str,
        uplink_name: &str,
    ) -> Result<bool, SdkError>;

    #[allow(dead_code)]
    fn bridge_is_managed(&self, bridge_name: &str, uplink_name: &str) -> Result<bool, SdkError>;

    #[allow(dead_code)]
    fn load_network(&self, vm_id: i64) -> Result<NetworkConfiguration, SdkError>;

    fn find_autostart_policy(&self, name: &str) -> Result<Option<AutostartPolicy>, SdkError>;

    /// Returns every stored policy ordered by MicroVM name.
    fn list_autostart_policies(&self) -> Result<Vec<AutostartPolicy>, SdkError>;

    fn insert_autostart_policy(
        &self,
        vm_id: i64,
        settings: &AutostartSettings,
    ) -> Result<(), SdkError>;

    fn update_autostart_policy(
        &self,
        vm_id: i64,
        settings: &AutostartSettings,
    ) -> Result<(), SdkError>;

    /// Returns `true` when a policy row was removed.
    fn delete_autostart_policy(&self, vm_id: i64) -> Result<bool, SdkError>;
}

pub(crate) trait LocalRepository: ArtifactRepository + MicroVmRepository {}

impl<T> LocalRepository for T where T: ArtifactRepository + MicroVmRepository {}
