use std::path::{Path, PathBuf};

use crate::error::SdkError;

/// Result of preparing a VM-local writable root filesystem.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRootfs {
    pub path: PathBuf,
    pub created_volume: bool,
}

/// Replaceable storage boundary for rootfs copy, resize, and guest mutation.
pub(crate) trait GuestStorage: Send + Sync {
    fn prepare_rootfs(
        &self,
        source: &Path,
        volume_path: &Path,
        requested_size_bytes: u64,
    ) -> Result<PreparedRootfs, SdkError>;

    fn inject_public_key(&self, rootfs_path: &Path, public_key: &str) -> Result<(), SdkError>;
}
