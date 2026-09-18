use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use crate::domain::microvm::PersistedRuntime;
use crate::error::SdkError;
use crate::ports::runtime::{RuntimeController, RuntimeRequest, TemporaryRuntime};

const TEMPORARY_RUNTIME_DEADLINE: Duration = Duration::from_secs(30);

/// Internal firectl/Firecracker process adapter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FirecrackerRuntime;

impl RuntimeController for FirecrackerRuntime {
    fn validate_host(&self) -> Result<(), SdkError> {
        let kvm = Path::new("/dev/kvm");
        let metadata =
            fs::symlink_metadata(kvm).map_err(|error| SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: format!("/dev/kvm is unavailable: {error}"),
            })?;
        if !metadata.file_type().is_char_device() {
            return Err(SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: "/dev/kvm is not a character device".to_owned(),
            });
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(kvm)
            .map(|_| ())
            .map_err(|error| SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: format!("/dev/kvm is not readable and writable: {error}"),
            })
    }

    fn start_temporary(&self, request: &RuntimeRequest) -> Result<TemporaryRuntime, SdkError> {
        if path_entry_exists(&request.socket_path)? {
            return Err(SdkError::TemporaryRuntime {
                component: "firectl".to_owned(),
                reason: "the VM socket path is already active".to_owned(),
                stopped: false,
            });
        }
        let arguments = build_arguments(request);
        let mut child = Command::new(&request.firectl_path)
            .args(&arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| SdkError::TemporaryRuntime {
                component: "firectl".to_owned(),
                reason: format!("could not start selected runtime: {error}"),
                stopped: true,
            })?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let cleanup_failures = cleanup_spawned_child(&mut child, &request.socket_path);
                return if cleanup_failures.is_empty() {
                    Err(SdkError::TemporaryRuntime {
                        component: "firectl".to_owned(),
                        reason: format!("selected runtime exited immediately with {status}"),
                        stopped: true,
                    })
                } else {
                    Err(SdkError::Cleanup {
                        primary: format!("selected runtime exited immediately with {status}"),
                        failures: cleanup_failures,
                    })
                };
            }
            Ok(None) => {}
            Err(error) => {
                let cleanup_failures = cleanup_spawned_child(&mut child, &request.socket_path);
                return if cleanup_failures.is_empty() {
                    Err(SdkError::TemporaryRuntime {
                        component: "firectl".to_owned(),
                        reason: format!("could not inspect selected runtime: {error}"),
                        stopped: true,
                    })
                } else {
                    Err(SdkError::Cleanup {
                        primary: format!("could not inspect selected runtime: {error}"),
                        failures: cleanup_failures,
                    })
                };
            }
        }
        Ok(TemporaryRuntime {
            process: child,
            request: request.clone(),
            deadline: TEMPORARY_RUNTIME_DEADLINE,
        })
    }

    fn stop(&self, runtime: &mut TemporaryRuntime) -> Result<PersistedRuntime, SdkError> {
        let process_id = runtime.process.id();
        let already_finished = runtime
            .process
            .try_wait()
            .map_err(|error| SdkError::TemporaryRuntime {
                component: "firectl".to_owned(),
                reason: format!("could not inspect runtime before stopping: {error}"),
                stopped: false,
            })?
            .is_some();
        if !already_finished {
            runtime
                .process
                .kill()
                .map_err(|error| SdkError::TemporaryRuntime {
                    component: "firectl".to_owned(),
                    reason: format!("could not stop the exact temporary process: {error}"),
                    stopped: false,
                })?;
        }
        runtime
            .process
            .wait()
            .map_err(|error| SdkError::TemporaryRuntime {
                component: "firectl".to_owned(),
                reason: format!("could not wait for the temporary process: {error}"),
                stopped: false,
            })?;
        if path_entry_exists(&runtime.request.socket_path)? {
            fs::remove_file(&runtime.request.socket_path).map_err(|error| {
                SdkError::TemporaryRuntime {
                    component: "firecracker.sock".to_owned(),
                    reason: format!("could not remove the temporary socket: {error}"),
                    stopped: false,
                }
            })?;
        }
        Ok(PersistedRuntime {
            firecracker_path: runtime.request.firecracker_path.clone(),
            firectl_path: runtime.request.firectl_path.clone(),
            socket_path: runtime.request.socket_path.clone(),
            process_id: Some(process_id),
            process_state: "stopped".to_owned(),
        })
    }

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError> {
        if path_entry_exists(socket_path)? {
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker.sock".to_owned(),
                reason: "the VM socket is still present after stopping".to_owned(),
                stopped: false,
            });
        }
        Ok(())
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem("inspect runtime socket", path, error)),
    }
}

fn cleanup_spawned_child(child: &mut Child, socket_path: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(error) = child.kill() {
                failures.push(format!("kill selected runtime: {error}"));
            }
            if let Err(error) = child.wait() {
                failures.push(format!("wait for selected runtime: {error}"));
            }
        }
        Err(error) => failures.push(format!("inspect selected runtime: {error}")),
    }
    match fs::symlink_metadata(socket_path) {
        Ok(_) => {
            if let Err(error) = fs::remove_file(socket_path) {
                failures.push(format!("remove temporary socket: {error}"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!("inspect temporary socket: {error}")),
    }
    failures
}

fn build_arguments(request: &RuntimeRequest) -> Vec<String> {
    let boot_arguments = request
        .boot_arguments
        .iter()
        .chain(std::iter::once(&request.network_boot_argument))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    vec![
        "--firecracker-binary".to_owned(),
        request.firecracker_path.display().to_string(),
        "--kernel".to_owned(),
        request.kernel_path.display().to_string(),
        "--root-drive".to_owned(),
        format!("path={},rw=true", request.rootfs_path.display()),
        "--vcpu-count".to_owned(),
        request.vcpu_count.to_string(),
        "--memory".to_owned(),
        request.memory_effective_mib.to_string(),
        "--tap-device".to_owned(),
        format!("{}={}", request.tap_name, request.guest_mac),
        "--kernel-opts".to_owned(),
        boot_arguments,
        "--socket-path".to_owned(),
        request.socket_path.display().to_string(),
    ]
}

#[allow(dead_code)]
fn child_is_running(child: &mut Child) -> Result<bool, SdkError> {
    child
        .try_wait()
        .map(|status| status.is_none())
        .map_err(|error| SdkError::TemporaryRuntime {
            component: "firectl".to_owned(),
            reason: error.to_string(),
            stopped: false,
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::ports::runtime::RuntimeRequest;

    use super::build_arguments;

    #[test]
    fn builds_private_runtime_arguments_from_validated_values() {
        let arguments = build_arguments(&RuntimeRequest {
            firecracker_path: PathBuf::from("/tools/firecracker"),
            firectl_path: PathBuf::from("/tools/firectl"),
            kernel_path: PathBuf::from("/artifacts/vmlinux"),
            rootfs_path: PathBuf::from("/vms/test/rootfs.ext4"),
            socket_path: PathBuf::from("/vms/test/firecracker.sock"),
            vcpu_count: 2,
            memory_effective_mib: 1024,
            boot_arguments: vec!["console=ttyS0".to_owned()],
            tap_name: "tm-test".to_owned(),
            guest_mac: "02:fc:00:00:00:01".to_owned(),
            network_boot_argument: "ip=dhcp".to_owned(),
        });

        assert!(
            arguments
                .windows(2)
                .any(|pair| { pair[0] == "--root-drive" && pair[1].contains("rootfs.ext4") })
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| { pair[0] == "--kernel-opts" && pair[1] == "console=ttyS0 ip=dhcp" })
        );
    }
}
