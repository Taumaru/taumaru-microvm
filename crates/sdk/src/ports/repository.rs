use crate::domain::artifact::{DownloadSpec, FileIntegrity, InstalledBinary};
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
}
