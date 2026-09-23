use std::path::{Path, PathBuf};

use crate::domain::snapshot::SnapshotProgress;
use crate::error::SdkError;

/// Replaceable host boundary for per-VM loop and Device Mapper resources.
pub(crate) trait RuntimeDiskController: Send + Sync {
    /// Verifies or creates the runtime block mapping for one persistent root disk.
    fn ensure_mapping(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<PathBuf, SdkError>;

    /// Creates a temporary read-only point-in-time view of a running root disk.
    fn create_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
        on_progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<PathBuf, SdkError>;

    /// Verifies that the temporary snapshot view remains valid and has not overflowed.
    fn check_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError>;

    /// Removes only the snapshot mapping, its verified COW loop, and its temporary backing file.
    fn remove_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError>;

    /// Releases only resources whose identity still proves ownership by this VM.
    fn release_mapping(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError>;
}
