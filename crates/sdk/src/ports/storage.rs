use std::path::{Path, PathBuf};

use crate::error::SdkError;
use tokio_util::sync::CancellationToken;

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

    fn write_guest_ipv4_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        prefix_length: u8,
        gateway: Option<std::net::Ipv4Addr>,
        lan_address: Option<std::net::Ipv4Addr>,
    ) -> Result<(), SdkError> {
        if prefix_length != 30 {
            return Err(SdkError::GuestFilesystem {
                operation: "write restored guest network configuration".to_owned(),
                path: rootfs_path.to_path_buf(),
                reason: "this storage adapter does not support the archived IPv4 prefix".to_owned(),
            });
        }
        match (gateway, lan_address) {
            (Some(gateway), Some(lan_address)) => {
                self.write_guest_lan_config(rootfs_path, guest_address, gateway, lan_address)
            }
            (Some(gateway), None) => {
                self.write_guest_network_config(rootfs_path, guest_address, gateway)
            }
            _ => Err(SdkError::GuestFilesystem {
                operation: "write restored guest network configuration".to_owned(),
                path: rootfs_path.to_path_buf(),
                reason: "this storage adapter requires an IPv4 gateway".to_owned(),
            }),
        }
    }

    fn prepare_private_snapshot_view(&self, rootfs_path: &Path) -> Result<(), SdkError> {
        Err(SdkError::SnapshotCapability {
            capability: "offline ext4 sanitization".to_owned(),
            reason: format!("storage adapter cannot sanitize {}", rootfs_path.display()),
        })
    }

    fn copy_rootfs_exact(
        &self,
        source: &Path,
        destination: &Path,
        size_bytes: u64,
        cancellation: CancellationToken,
        on_copy_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), SdkError> {
        let _ = (
            source,
            destination,
            size_bytes,
            cancellation,
            on_copy_progress,
        );
        Err(SdkError::SnapshotCapability {
            capability: "private root-disk copy".to_owned(),
            reason: "storage adapter cannot copy a stable block view".to_owned(),
        })
    }
}
