use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::SdkError;

static MOUNT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Injects one public SSH key into an offline ext4 image.
pub(crate) fn inject_public_key(rootfs_path: &Path, public_key: &str) -> Result<(), SdkError> {
    let sequence = MOUNT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mount_parent = match rootfs_path.parent() {
        Some(parent) => parent,
        None => Path::new("/tmp"),
    };
    let mount_path = mount_parent.join(format!(".guest-mount-{sequence}"));
    fs::create_dir(&mount_path)
        .map_err(|error| SdkError::filesystem("create guest mountpoint", &mount_path, error))?;

    let mount_result = Command::new("mount")
        .args(["-o", "loop"])
        .arg(rootfs_path)
        .arg(&mount_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let mount_output = match mount_result {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let _ = fs::remove_dir(&mount_path);
            return Err(SdkError::GuestFilesystem {
                operation: "mount rootfs for SSH injection".to_owned(),
                path: rootfs_path.to_path_buf(),
                reason: format!("mount exited with {}", output.status),
            });
        }
        Err(error) => {
            let _ = fs::remove_dir(&mount_path);
            return Err(SdkError::HostCommand {
                program: "mount".to_owned(),
                reason: error.to_string(),
            });
        }
    };
    drop(mount_output);

    let operation_result = update_authorized_keys(&mount_path, public_key);
    let unmount_result = Command::new("umount")
        .arg(&mount_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let remove_result = fs::remove_dir(&mount_path);
    let unmount_error = inspect_unmount_result(unmount_result, rootfs_path).err();
    let remove_error = remove_result.err();

    if let Err(error) = operation_result {
        let mut failures = Vec::new();
        if let Some(cleanup) = unmount_error {
            failures.push(cleanup.to_string());
        }
        if let Some(cleanup) = remove_error {
            failures.push(format!("remove guest mountpoint: {cleanup}"));
        }
        return if failures.is_empty() {
            Err(error)
        } else {
            Err(SdkError::Cleanup {
                primary: error.to_string(),
                failures,
            })
        };
    }
    if let Some(error) = unmount_error {
        let mut failures = Vec::new();
        if let Some(cleanup) = remove_error {
            failures.push(format!("remove guest mountpoint: {cleanup}"));
        }
        return if failures.is_empty() {
            Err(error)
        } else {
            Err(SdkError::Cleanup {
                primary: error.to_string(),
                failures,
            })
        };
    }
    if let Some(error) = remove_error {
        return Err(SdkError::filesystem(
            "remove guest mountpoint",
            &mount_path,
            error,
        ));
    }
    Ok(())
}

fn inspect_unmount_result(
    result: std::io::Result<std::process::Output>,
    rootfs_path: &Path,
) -> Result<(), SdkError> {
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(SdkError::GuestFilesystem {
            operation: "unmount rootfs after SSH injection".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("umount exited with {}", output.status),
        }),
        Err(error) => Err(SdkError::HostCommand {
            program: "umount".to_owned(),
            reason: error.to_string(),
        }),
    }
}

fn update_authorized_keys(mount_path: &Path, public_key: &str) -> Result<(), SdkError> {
    let root_directory = mount_path.join("root");
    let root_metadata =
        fs::symlink_metadata(&root_directory).map_err(|error| SdkError::GuestFilesystem {
            operation: "inspect guest root directory".to_owned(),
            path: root_directory.clone(),
            reason: error.to_string(),
        })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(SdkError::GuestFilesystem {
            operation: "verify guest root directory".to_owned(),
            path: root_directory,
            reason: "the expected /root directory is unavailable".to_owned(),
        });
    }
    let ssh_directory = mount_path.join("root/.ssh");
    let metadata =
        fs::symlink_metadata(&ssh_directory).map_err(|error| SdkError::GuestFilesystem {
            operation: "inspect guest SSH directory".to_owned(),
            path: ssh_directory.clone(),
            reason: error.to_string(),
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SdkError::GuestFilesystem {
            operation: "verify guest SSH directory".to_owned(),
            path: ssh_directory,
            reason: "the expected /root/.ssh directory is unavailable".to_owned(),
        });
    }
    set_mode(&ssh_directory, 0o700)?;
    let authorized_keys = mount_path.join("root/.ssh/authorized_keys");
    if let Ok(metadata) = fs::symlink_metadata(&authorized_keys)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(SdkError::GuestFilesystem {
            operation: "verify guest authorized_keys".to_owned(),
            path: authorized_keys,
            reason: "the expected authorized_keys path is not a regular file".to_owned(),
        });
    }

    let mut existing = String::new();
    match fs::File::open(&authorized_keys) {
        Ok(mut file) => {
            file.read_to_string(&mut existing)
                .map_err(|error| SdkError::GuestFilesystem {
                    operation: "read guest authorized_keys".to_owned(),
                    path: authorized_keys.clone(),
                    reason: error.to_string(),
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SdkError::GuestFilesystem {
                operation: "open guest authorized_keys".to_owned(),
                path: authorized_keys.clone(),
                reason: error.to_string(),
            });
        }
    }
    let public_key_line = public_key.trim_end_matches(['\r', '\n']);
    let already_present = existing.lines().any(|line| line.trim() == public_key_line);
    if !already_present {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&authorized_keys)
            .map_err(|error| SdkError::GuestFilesystem {
                operation: "open guest authorized_keys for update".to_owned(),
                path: authorized_keys.clone(),
                reason: error.to_string(),
            })?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            file.write_all(b"\n")
                .map_err(|error| SdkError::GuestFilesystem {
                    operation: "separate guest authorized_keys entries".to_owned(),
                    path: authorized_keys.clone(),
                    reason: error.to_string(),
                })?;
        }
        file.write_all(public_key_line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|error| SdkError::GuestFilesystem {
                operation: "inject public SSH key".to_owned(),
                path: authorized_keys.clone(),
                reason: error.to_string(),
            })?;
    }
    set_mode(&authorized_keys, 0o600)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
            SdkError::GuestFilesystem {
                operation: "set guest SSH permissions".to_owned(),
                path: path.to_path_buf(),
                reason: error.to_string(),
            }
        })?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}
