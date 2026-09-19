use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::SdkError;
use crate::ports::storage::{GuestStorage, PreparedRootfs};

use super::guest_fs;

static ROOTFS_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Linux ext4 adapter for VM-local rootfs preparation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ext4Storage;

impl GuestStorage for Ext4Storage {
    fn prepare_rootfs(
        &self,
        source: &Path,
        volume_path: &Path,
        requested_size_bytes: u64,
    ) -> Result<PreparedRootfs, SdkError> {
        let source_metadata = fs::symlink_metadata(source)
            .map_err(|error| SdkError::filesystem("inspect source rootfs", source, error))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
            return Err(SdkError::GuestFilesystem {
                operation: "verify source rootfs".to_owned(),
                path: source.to_path_buf(),
                reason: "the source is not a regular file".to_owned(),
            });
        }
        let source_size = source_metadata.len();
        if requested_size_bytes < source_size {
            return Err(SdkError::DiskSizeTooSmall {
                image_id: source.display().to_string(),
                requested_size_bytes,
                source_size_bytes: source_size,
            });
        }
        verify_ext4_filesystem(source)?;

        let created_volume = match fs::symlink_metadata(volume_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(SdkError::StorageConflict {
                        vm_name: volume_path.display().to_string(),
                        volume_path: volume_path.to_path_buf(),
                        owner: "non-directory filesystem entry".to_owned(),
                        reason: "the VM volume must be a real directory".to_owned(),
                    });
                }
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(volume_path).map_err(|source| {
                    SdkError::filesystem("create VM volume directory", volume_path, source)
                })?;
                true
            }
            Err(error) => {
                return Err(SdkError::filesystem(
                    "inspect VM volume directory",
                    volume_path,
                    error,
                ));
            }
        };

        let rootfs_path = volume_path.join("rootfs.ext4");
        if fs::symlink_metadata(&rootfs_path).is_ok() {
            if created_volume {
                let _ = fs::remove_dir(volume_path);
            }
            return Err(SdkError::StorageConflict {
                vm_name: volume_path.display().to_string(),
                volume_path: volume_path.to_path_buf(),
                owner: "existing rootfs.ext4".to_owned(),
                reason: "refusing to overwrite a VM volume file".to_owned(),
            });
        }
        let sequence = ROOTFS_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = volume_path.join(format!(".rootfs.{sequence}.part"));
        let result = copy_rootfs(source, &temporary_path, &rootfs_path, requested_size_bytes)
            .and_then(|_| {
                if requested_size_bytes > source_size {
                    resize_filesystem(&rootfs_path)
                } else {
                    Ok(())
                }
            })
            .and_then(|_| verify_ext4_filesystem(&rootfs_path))
            .and_then(|_| verify_rootfs_size(&rootfs_path, requested_size_bytes));
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary_path);
            let _ = fs::remove_file(&rootfs_path);
            if created_volume {
                let _ = fs::remove_dir(volume_path);
            }
            return Err(error);
        }

        Ok(PreparedRootfs {
            path: rootfs_path,
            created_volume,
        })
    }

    fn inject_public_key(&self, rootfs_path: &Path, public_key: &str) -> Result<(), SdkError> {
        guest_fs::inject_public_key(rootfs_path, public_key)
    }

    fn write_guest_network_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        gateway: std::net::Ipv4Addr,
    ) -> Result<(), SdkError> {
        guest_fs::write_guest_network_config(rootfs_path, guest_address, gateway)
    }

    fn write_guest_lan_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        gateway: std::net::Ipv4Addr,
        lan_address: std::net::Ipv4Addr,
    ) -> Result<(), SdkError> {
        guest_fs::write_guest_lan_config(rootfs_path, guest_address, gateway, lan_address)
    }
}
fn verify_ext4_filesystem(path: &Path) -> Result<(), SdkError> {
    let output = Command::new("blkid")
        .args(["-p", "-o", "value", "-s", "TYPE"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "blkid".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "ext4" {
        return Err(SdkError::GuestFilesystem {
            operation: "verify ext4 filesystem".to_owned(),
            path: path.to_path_buf(),
            reason: "the image does not contain an ext4 filesystem".to_owned(),
        });
    }
    Ok(())
}

fn copy_rootfs(
    source: &Path,
    temporary_path: &Path,
    rootfs_path: &Path,
    requested_size_bytes: u64,
) -> Result<(), SdkError> {
    let mut input = fs::File::open(source)
        .map_err(|error| SdkError::filesystem("open source rootfs", source, error))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|error| SdkError::filesystem("create temporary rootfs", temporary_path, error))?;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| SdkError::filesystem("read source rootfs", source, error))?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(|error| {
            SdkError::filesystem("write temporary rootfs", temporary_path, error)
        })?;
    }
    if requested_size_bytes
        > fs::metadata(source)
            .map_err(|error| SdkError::filesystem("inspect source rootfs", source, error))?
            .len()
    {
        output
            .set_len(requested_size_bytes)
            .map_err(|error| SdkError::filesystem("expand rootfs file", temporary_path, error))?;
    }
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|error| SdkError::filesystem("sync temporary rootfs", temporary_path, error))?;
    drop(output);
    fs::rename(temporary_path, rootfs_path)
        .map_err(|error| SdkError::filesystem("publish VM rootfs", rootfs_path, error))
}

fn resize_filesystem(rootfs_path: &Path) -> Result<(), SdkError> {
    let output = Command::new("resize2fs")
        .arg(rootfs_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "resize2fs".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "resize ext4 filesystem".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("resize2fs exited with {}", output.status),
        });
    }
    Ok(())
}

fn verify_rootfs_size(path: &Path, requested_size_bytes: u64) -> Result<(), SdkError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| SdkError::filesystem("verify VM rootfs", path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SdkError::GuestFilesystem {
            operation: "verify VM rootfs".to_owned(),
            path: path.to_path_buf(),
            reason: "published rootfs is not a regular file".to_owned(),
        });
    }
    if metadata.len() != requested_size_bytes {
        return Err(SdkError::GuestFilesystem {
            operation: "verify VM rootfs size".to_owned(),
            path: path.to_path_buf(),
            reason: format!(
                "expected {requested_size_bytes} bytes, found {}",
                metadata.len()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::copy_rootfs;

    #[test]
    fn copies_the_source_without_mutating_it() {
        let directory = tempdir().expect("temporary directory should exist");
        let source = directory.path().join("source.ext4");
        let temporary = directory.path().join("rootfs.part");
        let destination = directory.path().join("rootfs.ext4");
        fs::write(&source, b"verified source").expect("source should be written");

        copy_rootfs(&source, &temporary, &destination, 32)
            .expect("rootfs copy and expansion should succeed");

        assert_eq!(
            fs::read(&source).expect("source should remain readable"),
            b"verified source"
        );
        assert_eq!(
            fs::metadata(destination)
                .expect("destination should exist")
                .len(),
            32
        );
        assert!(!temporary.exists());
    }
}
