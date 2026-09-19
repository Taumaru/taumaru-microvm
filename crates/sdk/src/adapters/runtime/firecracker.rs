use std::fs::{self, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use crate::error::SdkError;
use crate::ports::runtime::RuntimeController;

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
