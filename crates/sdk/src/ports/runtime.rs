use std::path::Path;

use crate::error::SdkError;

/// Replaceable process boundary for Firecracker host validation, liveness
/// checks, detached launch, and readiness probing.
pub(crate) trait RuntimeController: Send + Sync {
    fn validate_host(&self) -> Result<(), SdkError>;

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError>;

    /// Returns `true` only when the recorded process is alive and its command
    /// line still references this VM. Never signals the process.
    fn process_references_vm(
        &self,
        process_id: u32,
        socket_path: &Path,
        firecracker_path: &Path,
    ) -> Result<bool, SdkError> {
        let _ = (process_id, socket_path, firecracker_path);
        Ok(false)
    }

    /// Returns `true` only when a control connection to the socket answers
    /// with a successful machine-configuration read. File existence alone
    /// never counts.
    fn socket_answers(&self, socket_path: &Path) -> Result<bool, SdkError> {
        let _ = socket_path;
        Ok(false)
    }

    /// Launches the full persisted VM configuration as a detached background
    /// process and returns the spawned process identifier.
    fn launch_detached(&self, request: &StartRequest) -> Result<u32, SdkError> {
        let _ = request;
        Err(SdkError::TemporaryRuntime {
            component: "firecracker".to_owned(),
            reason: "detached launch is not implemented by this runtime".to_owned(),
            stopped: true,
        })
    }

    /// Waits until the volume-local control socket answers or the bound
    /// expires. Returns `Ok(true)` on readiness, `Ok(false)` on expiry.
    fn wait_for_socket(&self, socket_path: &Path) -> Result<bool, SdkError> {
        let _ = socket_path;
        Ok(false)
    }

    /// Terminates a process spawned by the current call. Must only be used
    /// with a PID returned by [`RuntimeController::launch_detached`].
    fn terminate_spawned(&self, process_id: u32) -> Result<(), SdkError> {
        let _ = process_id;
        Ok(())
    }
}

/// Validated inputs for one detached VM launch.
#[derive(Clone, Debug)]
pub(crate) struct StartRequest {
    /// VM name used for diagnostics only.
    pub vm_name: String,
    /// Verified `firectl` executable path.
    pub firectl_path: std::path::PathBuf,
    /// Verified Firecracker executable path.
    pub firecracker_path: std::path::PathBuf,
    /// Verified kernel image path.
    pub kernel_path: std::path::PathBuf,
    /// VM-local writable root disk.
    pub rootfs_path: std::path::PathBuf,
    /// Requested vCPU count.
    pub vcpu_count: u32,
    /// Checked effective memory in MiB.
    pub memory_effective_mib: u64,
    /// Kernel command line including root device and network parameters.
    pub kernel_options: String,
    /// VM-specific TAP interface.
    pub tap_name: String,
    /// Guest MAC address.
    pub guest_mac: String,
    /// Volume-local control socket path.
    pub socket_path: std::path::PathBuf,
    /// VM-local log file for captured launch output.
    pub log_path: std::path::PathBuf,
}
