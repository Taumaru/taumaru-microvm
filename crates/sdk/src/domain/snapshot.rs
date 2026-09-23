use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

/// The stage currently being performed by a snapshot operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotProgressStage {
    /// Preparing the point-in-time view of a running disk.
    PreparingDiskView,
    /// Reserving space for the snapshot copy-on-write store.
    AllocatingCowStore,
    /// Connecting the completed copy-on-write store to Device Mapper.
    InstallingDiskView,
    /// Preparing the encrypted archive before payload streaming begins.
    PreparingArchive,
    /// Reading payload bytes and writing them into the encrypted archive.
    StreamingPayloads,
    /// Writing archive metadata and finalizing compression and encryption.
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

/// Result of creating a password-encrypted MicroVM snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotResult {
    /// Name of the captured MicroVM.
    pub vm_name: String,
    /// Final path of the encrypted snapshot archive.
    pub output_path: PathBuf,
    /// Number of encrypted bytes written to the archive.
    pub encrypted_size_bytes: u64,
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
