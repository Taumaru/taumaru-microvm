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

/// Returns a deterministic readiness message for the bootstrap SDK.
///
/// This is a temporary demonstration API. It may be replaced before the first stable release
/// and does not represent the future MicroVM manager API.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     taumaru_microvm::example_message(),
///     "taumaru-microvm SDK is ready"
/// );
/// ```
#[must_use]
pub fn example_message() -> &'static str {
    "taumaru-microvm SDK is ready"
}
