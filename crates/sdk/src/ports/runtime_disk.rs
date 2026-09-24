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

    /// Creates an independently owned writable Device Mapper snapshot over a stable parent view.
    ///
    /// A capability error with `capability == "nested_classic_snapshot_unsupported"` means a
    /// verified host probe showed that this kernel cannot create the child; callers may then use
    /// the exact-copy fallback. Other failures must not be treated as capability absence.
    fn create_private_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
        stable_parent_path: &Path,
        operation_id: &str,
        on_progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<PathBuf, SdkError> {
        let _ = (
            sdk_home,
            vm_name,
            rootfs_path,
            stable_parent_path,
            operation_id,
            on_progress,
        );
        Err(SdkError::SnapshotCapability {
            capability: "nested_classic_snapshot_unsupported".to_owned(),
            reason: "this runtime disk adapter cannot create nested snapshots".to_owned(),
        })
    }

    /// Verifies the independently owned writable child and its COW status.
    fn check_private_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
        stable_parent_path: &Path,
        operation_id: &str,
    ) -> Result<(), SdkError> {
        let _ = (
            sdk_home,
            vm_name,
            rootfs_path,
            stable_parent_path,
            operation_id,
        );
        Err(SdkError::SnapshotCapability {
            capability: "private_snapshot_view".to_owned(),
            reason: "this runtime disk adapter cannot verify a private snapshot view".to_owned(),
        })
    }

    /// Removes the owned child mapper, its COW loop, and backing file in dependency order.
    fn remove_private_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
        stable_parent_path: &Path,
        operation_id: &str,
    ) -> Result<(), SdkError> {
        let _ = (
            sdk_home,
            vm_name,
            rootfs_path,
            stable_parent_path,
            operation_id,
        );
        Ok(())
    }

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
