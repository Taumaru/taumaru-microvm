use std::path::{Path, PathBuf};

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

    /// Releases only resources whose identity still proves ownership by this VM.
    fn release_mapping(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError>;
}
