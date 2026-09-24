use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

/// Selects how IPv4 identity is stored in a snapshot and reconstructed on restore.
///
/// `PreserveIpv4` embeds the source guest address, prefix, optional gateway and LAN
/// address, network mode, exposure setting, and guest MAC. Restore fails if the
/// destination cannot reproduce those exact values. `RegenerateIpv4` stores only
/// whether the source VM is exposed on the LAN; restore allocates destination-local
/// addresses and writes them into the restored guest disk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotAddressPolicy {
    /// Preserve the source IPv4 addresses and guest MAC across restore.
    PreserveIpv4,
    /// Allocate destination-local IPv4 addresses during restore.
    RegenerateIpv4,
}

/// The stage currently being performed by a snapshot operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotProgressStage {
    /// Preparing the point-in-time view of a running disk.
    PreparingDiskView,
    /// Reserving space for the snapshot copy-on-write store.
    AllocatingCowStore,
    /// Connecting the completed copy-on-write store to Device Mapper.
    InstallingDiskView,
    /// Creating a writable private view for network sanitization.
    PreparingPrivateView,
    /// Removing the managed guest network file from a private view.
    SanitizingNetwork,
    /// Copying a stable disk view when nested snapshots are unavailable.
    CopyingPrivateDisk,
    /// Preparing the archive before payload streaming begins.
    PreparingArchive,
    /// Reading payload bytes and writing them into the archive.
    StreamingPayloads,
    /// Writing archive metadata and finalizing compression.
    FinalizingArchive,
}

/// Progress information emitted during snapshot creation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotProgress {
    /// Current stage of snapshot creation.
    pub stage: SnapshotProgressStage,
    /// Completed bytes for the current stage when it has measurable byte progress.
    ///
    /// This counts reserved COW-store bytes during [`SnapshotProgressStage::AllocatingCowStore`]
    /// and uncompressed payload bytes read during [`SnapshotProgressStage::StreamingPayloads`].
    pub completed_bytes: u64,
    /// Total bytes expected for the current stage when it has measurable byte progress.
    pub total_bytes: u64,
}

/// Result of creating an unencrypted MicroVM snapshot archive.
///
/// The archive contains the VM disk and SSH credentials and is readable by any account that can
/// access the output file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotResult {
    /// Name of the captured MicroVM.
    pub vm_name: String,
    /// Final path of the unencrypted snapshot archive.
    pub output_path: PathBuf,
    /// Number of compressed archive bytes written.
    pub archive_size_bytes: u64,
    /// Whether the VM was running when the disk capture began.
    pub source_was_running: bool,
}

/// Cooperative cancellation handle for snapshot creation.
///
/// Cancellation is observed between bounded archive reads. A snapshot operation returns only
/// after temporary output and owned Device Mapper resources have been cleaned up.
#[derive(Clone, Debug)]
pub struct SnapshotCancellation {
    token: CancellationToken,
}

impl SnapshotCancellation {
    /// Creates a cancellation handle in the non-cancelled state.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Requests cancellation of the associated snapshot operation.
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

impl Default for SnapshotCancellation {
    fn default() -> Self {
        Self::new()
    }
}
