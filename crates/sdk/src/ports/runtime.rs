use std::path::Path;

use crate::error::SdkError;

/// Replaceable process boundary for Firecracker host validation, liveness
/// checks, detached launch, and readiness probing.
pub(crate) trait RuntimeController: Send + Sync {
    fn validate_host(&self) -> Result<(), SdkError>;

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError>;

    /// Returns true only when the recorded process has the persisted firectl
    /// executable and arguments for this VM's socket and Firecracker binary.
    /// Never signals the process.
    fn process_references_vm(
        &self,
        process_id: u32,
        socket_path: &Path,
        firectl_path: &Path,
        firecracker_path: &Path,
    ) -> Result<bool, SdkError> {
        let _ = (process_id, socket_path, firectl_path, firecracker_path);
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

    /// Sends one graceful shutdown request through the volume-local control
    /// socket. Returns `Ok(true)` when the request is delivered, `Ok(false)`
    /// when the socket is already silent, and `Err` for any other delivery
    /// failure. Never attempts forced termination.
    fn request_shutdown(&self, socket_path: &Path) -> Result<bool, SdkError> {
        let _ = socket_path;
        Err(SdkError::TemporaryRuntime {
            component: "firecracker.sock".to_owned(),
            reason: "graceful shutdown is not implemented by this runtime".to_owned(),
            stopped: false,
        })
    }

    /// Waits until the machine has exited or the bound expires. The machine
    /// counts as exited when the control socket is silent and the recorded
    /// process, when present, no longer references the VM. A `None` process
    /// identity waits on socket silence alone. Returns `Ok(true)` on exit,
    /// `Ok(false)` on expiry.
    fn wait_for_stop(
        &self,
        socket_path: &Path,
        process_id: Option<u32>,
        firectl_path: &Path,
        firecracker_path: &Path,
        deadline: std::time::Duration,
    ) -> Result<bool, SdkError> {
        let _ = (
            socket_path,
            process_id,
            firectl_path,
            firecracker_path,
            deadline,
        );
        Ok(false)
    }

    /// Terminates a process spawned by the current call, or the re-verified
    /// recorded process on the stop escalation path. Must never be used with
    /// a process identity that was not verified to reference this VM.
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
    /// Verified runtime block path passed internally to firectl.
    pub runtime_disk_path: std::path::PathBuf,
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
