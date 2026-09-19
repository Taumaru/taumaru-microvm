use std::path::{Path, PathBuf};

use crate::error::SdkError;

/// Result of preparing a VM-local writable root filesystem.
#[derive(Clone, Debug)]
pub(crate) struct PreparedRootfs {
    pub path: PathBuf,
    pub created_volume: bool,
}

/// Replaceable storage boundary for rootfs copy, resize, and guest mutation.
///
/// `prepare_rootfs` reports copy progress as `(bytes_copied, expected_bytes)` through
/// `on_copy_progress`; the callback is invoked synchronously from the copy loop and
/// must be non-blocking. Implementations that copy without chunking report a single
/// completion tick.
pub(crate) trait GuestStorage: Send + Sync {
    fn prepare_rootfs(
        &self,
        source: &Path,
        volume_path: &Path,
        requested_size_bytes: u64,
        on_copy_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<PreparedRootfs, SdkError>;

    fn inject_public_key(&self, rootfs_path: &Path, public_key: &str) -> Result<(), SdkError>;

    fn write_guest_network_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        gateway: std::net::Ipv4Addr,
    ) -> Result<(), SdkError>;

    fn write_guest_lan_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        gateway: std::net::Ipv4Addr,
        lan_address: std::net::Ipv4Addr,
    ) -> Result<(), SdkError>;
}
