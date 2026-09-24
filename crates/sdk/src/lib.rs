//! Reusable host-local MicroVM management capabilities.
//!
//! The SDK is intentionally silent and side-effect free until callers invoke an explicit
//! operation through its public API.

//!
//! # Snapshot API migration
//!
//! Version 0.2 removes password parameters from snapshot creation and removes the password
//! field from [`RestoreRequest`]. [`SnapshotResult::archive_size_bytes`] replaces the former
//! encrypted-size field. Snapshot archives created before format version 3 use age encryption
//! and cannot be restored by this release. To migrate one, restore it with a compatible prior
//! release, then create a new snapshot using version 0.2. Version 3 archives are not
//! encrypted and include the VM disk and SSH credentials, so access is controlled by the
//! archive file permissions and filesystem policy.

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
    FilesystemMetadata, InstalledBinary, Kernel, Linkage, PruneFailure, PrunePreview, PruneSummary,
    PrunedImageId, SnapshotAddressPolicy, SnapshotCancellation, SnapshotProgress,
    SnapshotProgressStage, SnapshotResult,
};
pub use domain::{
    CreateMicroVmRequest, CreationEventPhase, CreationOutcome, CreationProgress, CreationStage,
    MicroVmCreationResult, MicroVmDeleteResult, MicroVmStartResult, MicroVmState,
    MicroVmStopResult, MicroVmSummary, NetworkConfiguration, NetworkConfigurationResult,
    NetworkMode, NetworkResource, RunningMicroVm, SshConnectionInfo, TOTAL_CREATION_STEPS,
};
pub use error::SdkError;
pub use manager::MicroVmSdk;

pub use domain::{
    RestoreCancellation, RestoreProgress, RestoreProgressStage, RestoreRequest, RestoreResult,
};
