use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::domain::artifact::{DownloadSpec, FileIntegrity, InstalledBinary};
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
}

/// A verified artifact relationship loaded from the local inventory.
#[derive(Clone, Debug)]
pub(crate) struct LocalArtifact {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
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

/// Local inventory and lifecycle persistence for MicroVM records.
pub(crate) trait MicroVmRepository: Send + Sync {
    fn find_microvm(&self, name: &str) -> Result<Option<StoredMicroVm>, SdkError>;

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
}

pub(crate) trait LocalRepository: ArtifactRepository + MicroVmRepository {}

impl<T> LocalRepository for T where T: ArtifactRepository + MicroVmRepository {}
