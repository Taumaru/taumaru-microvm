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
    Architecture, ArtifactFile, ArtifactKind, BinaryFile, BinaryPackage, BootConfiguration,
    Distribution, DistributionImage, DistributionRequirements, DownloadCancellation,
    DownloadDisposition, DownloadPhase, DownloadProgress, DownloadedBinary, DownloadedDistribution,
    DownloadedDistributionImage, DownloadedFile, DownloadedKernel, ElfMetadata, Endianness,
    FilesystemMetadata, InstalledBinary, Kernel, Linkage, PruneFailure, PruneSummary,
    PrunedImageId,
};
pub use domain::{
    CreateMicroVmRequest, CreationEventPhase, CreationOutcome, CreationProgress, CreationStage,
    MicroVmCreationResult, MicroVmStartResult, MicroVmState, MicroVmStopResult, MicroVmSummary,
    NetworkConfiguration, NetworkConfigurationResult, NetworkMode, NetworkResource, RunningMicroVm,
    SshConnectionInfo, TOTAL_CREATION_STEPS,
};
pub use error::SdkError;
pub use manager::MicroVmSdk;
