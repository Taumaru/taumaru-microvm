use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::microvm::PersistedRuntime;
use crate::error::SdkError;

/// Runtime inputs assembled by the manager from verified artifacts and desired VM state.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeRequest {
    pub firecracker_path: PathBuf,
    pub firectl_path: PathBuf,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub socket_path: PathBuf,
    pub vcpu_count: u32,
    pub memory_effective_mib: u64,
    pub boot_arguments: Vec<String>,
    pub tap_name: String,
    pub guest_mac: String,
    pub network_boot_argument: String,
}

/// Replaceable process boundary for temporary Firecracker setup.
pub(crate) trait RuntimeController: Send + Sync {
    fn validate_host(&self) -> Result<(), SdkError>;

    fn start_temporary(&self, request: &RuntimeRequest) -> Result<TemporaryRuntime, SdkError>;

    fn stop(&self, runtime: &mut TemporaryRuntime) -> Result<PersistedRuntime, SdkError>;

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError>;
}

/// Handle for the exact process started by one temporary operation.
pub(crate) struct TemporaryRuntime {
    pub process: std::process::Child,
    pub request: RuntimeRequest,
    pub deadline: Duration,
}
