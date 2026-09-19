use std::path::Path;

use tokio_util::sync::CancellationToken;

use super::registry::{Architecture, BinaryPackage, Distribution, DistributionImage, Kernel};

/// Identifies the kind of physical artifact being transferred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    /// A Linux kernel image.
    Kernel,
    /// A file from a runtime binary package.
    Binary,
    /// A filesystem image from a distribution.
    DistributionImage,
}

/// Identifies the stage represented by a progress callback event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadPhase {
    /// Bytes are being received from the registry.
    Downloading,
    /// The published file is being checked against its registry metadata.
    Verifying,
    /// The file was transferred, verified, and committed to the inventory.
    Completed,
    /// The existing correct file was added to an incomplete inventory.
    AdoptedExisting,
    /// The existing correct file and complete inventory relationship were reused.
    SkippedExisting,
    /// The operation stopped cooperatively before publishing an unverified file.
    Cancelled,
}

/// Cooperative cancellation shared by one or more SDK artifact operations.
///
/// Cancelling a token does not remove verified files or inventory records that were committed
/// before cancellation. An in-flight member is cleaned up by the SDK before its operation
/// returns, and an unverified temporary file is never published as the managed target.
#[derive(Clone, Debug)]
pub struct DownloadCancellation {
    token: CancellationToken,
}

impl DownloadCancellation {
    /// Creates a new cancellation handle in the non-cancelled state.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Requests cooperative cancellation of the associated download operation.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Default for DownloadCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress information emitted while a public download operation runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    /// Registry artifact category.
    pub artifact_kind: ArtifactKind,
    /// Stable registry ID of the parent artifact.
    pub artifact_id: String,
    /// Component or image ID for multi-file artifacts.
    pub member_name: Option<String>,
    /// Bytes received for the current member.
    pub bytes_received: u64,
    /// Expected bytes for the current member.
    pub total_bytes: u64,
    /// Bytes received across the whole package or distribution operation.
    pub aggregate_bytes_received: u64,
    /// Expected bytes across the whole package or distribution operation.
    pub aggregate_total_bytes: u64,
    /// Event stage.
    pub phase: DownloadPhase,
}

/// Describes whether a physical file required a transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadDisposition {
    /// The file was streamed and verified.
    Downloaded,
    /// A correct existing file was adopted into the inventory.
    AdoptedExisting,
    /// A correct existing file with a complete inventory relationship was reused.
    SkippedExisting,
}

/// A verified local file returned by an artifact download operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadedFile {
    /// Physical artifact category.
    pub artifact_kind: ArtifactKind,
    /// Stable registry ID of the parent artifact.
    pub artifact_id: String,
    /// Component or image ID for multi-file artifacts.
    pub member_name: Option<String>,
    /// Normalized absolute path below the SDK home.
    pub absolute_path: std::path::PathBuf,
    /// Path relative to the SDK home.
    pub relative_path: std::path::PathBuf,
    /// Verified byte length.
    pub size_bytes: u64,
    /// Verified lowercase SHA-256 digest.
    pub sha256: String,
    /// Cache disposition.
    pub disposition: DownloadDisposition,
}

/// Result of downloading a kernel.
#[derive(Clone, Debug)]
pub struct DownloadedKernel {
    /// Registry metadata selected for the operation.
    pub kernel: Kernel,
    /// Verified local kernel file.
    pub file: DownloadedFile,
}

/// Result of downloading a binary package.
#[derive(Clone, Debug)]
pub struct DownloadedBinary {
    /// Registry metadata selected for the operation.
    pub binary: BinaryPackage,
    /// Verified local package files in registry order.
    pub files: Vec<DownloadedFile>,
}

/// Result of downloading a distribution image set.
#[derive(Clone, Debug)]
pub struct DownloadedDistribution {
    /// Registry metadata selected for the operation.
    pub distribution: Distribution,
    /// Verified local images in registry order.
    pub images: Vec<DownloadedFile>,
}

/// Result of downloading one distribution image.
#[derive(Clone, Debug)]
pub struct DownloadedDistributionImage {
    /// Registry metadata for the owning distribution as published.
    pub distribution: Distribution,
    /// Exact registry image metadata selected for the operation.
    pub image: DistributionImage,
    /// Verified local image file.
    pub file: DownloadedFile,
}

/// Metadata for a verified runtime binary resolved from local inventory.
#[derive(Clone, Debug)]
pub struct InstalledBinary {
    /// Stable binary package ID.
    pub package_id: String,
    /// Stable package component name.
    pub component_name: String,
    /// Package version.
    pub version: String,
    /// Package architecture.
    pub architecture: Architecture,
    /// Verified executable path.
    pub path: std::path::PathBuf,
    /// Current verified byte length.
    pub size_bytes: u64,
    /// Current verified lowercase SHA-256 digest.
    pub sha256: String,
    /// Registry executable indication.
    pub executable: bool,
}

/// Small internal descriptor shared by file transfer and persistence code.
#[derive(Clone, Debug)]
pub(crate) struct DownloadSpec {
    pub artifact_kind: ArtifactKind,
    pub artifact_id: String,
    pub member_name: Option<String>,
    pub artifact_key: String,
    pub registry_path: String,
    pub registry_url: String,
    pub filename: String,
    pub expected_size: u64,
    pub expected_sha256: String,
    pub executable: bool,
    pub mode: Option<String>,
    pub relative_path: std::path::PathBuf,
    pub absolute_path: std::path::PathBuf,
}

/// Actual size and digest calculated from a local file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileIntegrity {
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub(crate) struct ProgressTracker {
    aggregate_bytes_received: u64,
    aggregate_total_bytes: u64,
}

impl ProgressTracker {
    pub(crate) fn new(aggregate_total_bytes: u64) -> Self {
        Self {
            aggregate_bytes_received: 0,
            aggregate_total_bytes,
        }
    }

    pub(crate) fn event(
        &self,
        spec: &DownloadSpec,
        phase: DownloadPhase,
        member_bytes_received: u64,
    ) -> DownloadProgress {
        DownloadProgress {
            artifact_kind: spec.artifact_kind.clone(),
            artifact_id: spec.artifact_id.clone(),
            member_name: spec.member_name.clone(),
            bytes_received: member_bytes_received,
            total_bytes: spec.expected_size,
            aggregate_bytes_received: self
                .aggregate_bytes_received
                .saturating_add(member_bytes_received),
            aggregate_total_bytes: self.aggregate_total_bytes,
            phase,
        }
    }

    pub(crate) fn commit_member(&mut self, member_bytes_received: u64) {
        self.aggregate_bytes_received = self
            .aggregate_bytes_received
            .saturating_add(member_bytes_received);
    }
}

pub(crate) fn is_valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn validate_registry_path(value: &str) -> bool {
    if value.is_empty()
        || value.contains('\\')
        || value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return false;
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return false;
    }

    path.components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ArtifactKind, DownloadPhase, DownloadSpec, ProgressTracker, is_valid_sha256,
        validate_registry_path,
    };

    #[test]
    fn accepts_lowercase_sha256_and_rejects_malformed_digests() {
        assert!(is_valid_sha256(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_valid_sha256(
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_valid_sha256("not-a-sha256"));
    }

    #[test]
    fn rejects_registry_paths_that_escape_the_registry_root() {
        assert!(validate_registry_path("kernels/linux/vmlinux"));
        assert!(!validate_registry_path("../outside"));
        assert!(!validate_registry_path("/absolute/path"));
        assert!(!validate_registry_path("kernels//vmlinux"));
        assert!(!validate_registry_path("kernels/linux/./vmlinux"));
    }

    #[test]
    fn progress_events_keep_member_and_aggregate_counters_monotonic() {
        let first = DownloadSpec {
            artifact_kind: ArtifactKind::Binary,
            artifact_id: "package".to_owned(),
            member_name: Some("first".to_owned()),
            artifact_key: "binary:package:first".to_owned(),
            registry_path: "binaries/package/first".to_owned(),
            registry_url: "https://example.invalid/first".to_owned(),
            filename: "first".to_owned(),
            expected_size: 4,
            expected_sha256: "0".repeat(64),
            executable: true,
            mode: Some("755".to_owned()),
            relative_path: PathBuf::from("tools/package/first"),
            absolute_path: PathBuf::from("/tmp/sdk/tools/package/first"),
        };
        let second = DownloadSpec {
            expected_size: 6,
            member_name: Some("second".to_owned()),
            artifact_key: "binary:package:second".to_owned(),
            registry_path: "binaries/package/second".to_owned(),
            registry_url: "https://example.invalid/second".to_owned(),
            filename: "second".to_owned(),
            relative_path: PathBuf::from("tools/package/second"),
            absolute_path: PathBuf::from("/tmp/sdk/tools/package/second"),
            ..first.clone()
        };
        let mut tracker = ProgressTracker::new(10);

        let first_downloading = tracker.event(&first, DownloadPhase::Downloading, 2);
        let first_verifying = tracker.event(&first, DownloadPhase::Verifying, 4);
        tracker.commit_member(4);
        let second_downloading = tracker.event(&second, DownloadPhase::Downloading, 3);

        assert_eq!(first_downloading.aggregate_bytes_received, 2);
        assert_eq!(first_verifying.aggregate_bytes_received, 4);
        assert_eq!(second_downloading.aggregate_bytes_received, 7);
        assert!(
            first_downloading.aggregate_bytes_received <= first_verifying.aggregate_bytes_received
        );
        assert_eq!(first_verifying.total_bytes, 4);
        assert_eq!(second_downloading.total_bytes, 6);
    }
}
