use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use super::lifecycle::MicroVmState;
use super::microvm::{NetworkConfiguration, SshConnectionInfo};
use super::snapshot::SnapshotAddressPolicy;

/// Caller input for restoring one supported snapshot archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreRequest {
    /// Path to the unencrypted snapshot archive.
    pub archive_path: PathBuf,
}

/// A named phase of snapshot restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreProgressStage {
    /// Validating the archive path and request.
    ValidatingInput,
    /// Reading and staging fixed archive members.
    Staging,
    /// Verifying archive payloads and installed file integrity.
    Verifying,
    /// Checking destination runtime, storage, and network prerequisites.
    PreparingDestination,
    /// Installing verified files and writing destination guest settings.
    Installing,
    /// Publishing the complete stopped VM inventory.
    Committing,
    /// Restore completed and the VM remains stopped.
    Completed,
}

/// Progress information emitted by an SDK restore operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreProgress {
    /// Current restore phase.
    pub stage: RestoreProgressStage,
    /// Bytes consumed or processed in the current phase. During staging this counts uncompressed
    /// root disk bytes written to temporary storage; later phases report the bytes relevant to
    /// that phase.
    pub completed_bytes: u64,
    /// Total bytes for the current phase when known, or zero while progress is indeterminate.
    pub total_bytes: u64,
}

/// Cooperative cancellation handle for snapshot restoration.
#[derive(Clone, Debug)]
pub struct RestoreCancellation {
    token: CancellationToken,
}

impl RestoreCancellation {
    /// Creates a cancellation handle in the non-cancelled state.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Requests cancellation of the associated restore operation.
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

impl Default for RestoreCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of restoring a MicroVM from a supported unencrypted snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreResult {
    /// Restored VM identity taken from the archive manifest.
    pub vm_name: String,
    /// Always [`MicroVmState::Stopped`] when restore succeeds.
    pub state: MicroVmState,
    /// Destination-local VM volume directory.
    pub volume_path: PathBuf,
    /// Destination-local writable root disk.
    pub rootfs_path: PathBuf,
    /// Configured root disk size in bytes.
    pub disk_size_bytes: u64,
    /// Configured memory size in bytes.
    pub memory_bytes: u64,
    /// Configured virtual CPU count.
    pub vcpu_count: u32,
    /// Destination-local embedded guest kernel.
    pub kernel_path: PathBuf,
    /// Network settings installed for the destination host.
    pub network: NetworkConfiguration,
    /// Restored SSH key paths and connection information.
    pub ssh: SshConnectionInfo,
    /// Address policy recorded in the archive.
    pub address_policy: SnapshotAddressPolicy,
}
