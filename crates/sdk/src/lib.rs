//! Reusable host-local MicroVM management capabilities.
//!
//! The SDK is intentionally silent and side-effect free until callers invoke an explicit
//! operation through its public API.

mod adapters;
mod domain;
mod error;
mod manager;
mod ports;

pub use domain::{
    Architecture, ArtifactKind, BinaryFile, BinaryPackage, BootConfiguration, Distribution,
    DistributionImage, DistributionRequirements, DownloadDisposition, DownloadPhase,
    DownloadProgress, DownloadedBinary, DownloadedDistribution, DownloadedFile, DownloadedKernel,
    ElfMetadata, Endianness, FilesystemMetadata, InstalledBinary, Kernel, Linkage,
};
pub use error::SdkError;
pub use manager::MicroVmSdk;
