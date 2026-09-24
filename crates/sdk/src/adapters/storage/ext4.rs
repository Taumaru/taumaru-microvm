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
        on_copy_progress: &mut dyn FnMut(u64, u64),
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
        let result = copy_rootfs(
            source,
            &temporary_path,
            &rootfs_path,
            requested_size_bytes,
            on_copy_progress,
        )
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

    fn write_guest_ipv4_config(
        &self,
        rootfs_path: &Path,
        guest_address: std::net::Ipv4Addr,
        prefix_length: u8,
        gateway: Option<std::net::Ipv4Addr>,
        lan_address: Option<std::net::Ipv4Addr>,
    ) -> Result<(), SdkError> {
        guest_fs::write_guest_ipv4_config(
            rootfs_path,
            guest_address,
            prefix_length,
            gateway,
            lan_address,
        )
    }

    fn prepare_private_snapshot_view(&self, rootfs_path: &Path) -> Result<(), SdkError> {
        verify_ext4_filesystem(rootfs_path)?;
        guest_fs::sanitize_private_snapshot_network_file(rootfs_path)
    }

    fn copy_rootfs_exact(
        &self,
        source: &Path,
        destination: &Path,
        size_bytes: u64,
        cancellation: tokio_util::sync::CancellationToken,
        on_copy_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), SdkError> {
        copy_stable_view_exact(
            source,
            destination,
            size_bytes,
            cancellation,
            on_copy_progress,
        )
    }
}
fn copy_stable_view_exact(
    source: &Path,
    destination: &Path,
    size_bytes: u64,
    cancellation: tokio_util::sync::CancellationToken,
    on_copy_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), SdkError> {
    if size_bytes == 0 {
        return Err(SdkError::InvalidRequest {
            field: "snapshot_disk_size".to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    let parent = destination.parent().unwrap_or(Path::new("."));
    let available = available_bytes(parent)?;
    if available < size_bytes {
        return Err(SdkError::SnapshotCapability {
            capability: "private full-copy fallback".to_owned(),
            reason: format!(
                "requires {size_bytes} bytes, but only {available} bytes are available"
            ),
        });
    }
    let mut input = fs::File::open(source)
        .map_err(|error| SdkError::filesystem("open stable snapshot view", source, error))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination).map_err(|error| {
        SdkError::filesystem("create private snapshot disk copy", destination, error)
    })?;
    let result = (|| {
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 128 * 1024];
        on_copy_progress(0, size_bytes);
        while copied < size_bytes {
            if cancellation.is_cancelled() {
                return Err(SdkError::SnapshotCancelled);
            }
            let limit = buffer.len().min((size_bytes - copied) as usize);
            let read = input.read(&mut buffer[..limit]).map_err(|error| {
                SdkError::filesystem("read stable snapshot view", source, error)
            })?;
            if read == 0 {
                return Err(SdkError::SnapshotArchive {
                    operation: "copy stable snapshot view",
                    reason: "source ended before the recorded disk size".to_owned(),
                });
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                SdkError::filesystem("write private snapshot disk copy", destination, error)
            })?;
            copied = copied.saturating_add(read as u64);
            on_copy_progress(copied, size_bytes);
        }
        output.sync_all().map_err(|error| {
            SdkError::filesystem("sync private snapshot disk copy", destination, error)
        })
    })();
    if let Err(error) = result {
        drop(output);
        let cleanup = fs::remove_file(destination).err().map(|cleanup| {
            SdkError::filesystem("remove failed private snapshot copy", destination, cleanup)
        });
        return Err(match cleanup {
            Some(cleanup) => SdkError::Cleanup {
                primary: error.to_string(),
                failures: vec![cleanup.to_string()],
            },
            None => error,
        });
    }
    Ok(())
}

fn available_bytes(path: &Path) -> Result<u64, SdkError> {
    let output = Command::new("df")
        .args(["-Pk"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "df".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SdkError::HostCommand {
            program: "df".to_owned(),
            reason: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let available_kib = stdout
        .lines()
        .last()
        .and_then(|line| line.split_ascii_whitespace().nth(3))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| SdkError::HostCommand {
            program: "df".to_owned(),
            reason: "could not read available storage from df output".to_owned(),
        })?;
    available_kib
        .checked_mul(1024)
        .ok_or_else(|| SdkError::HostCommand {
            program: "df".to_owned(),
            reason: "available storage exceeds the supported size".to_owned(),
        })
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
    on_copy_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), SdkError> {
    let mut input = fs::File::open(source)
        .map_err(|error| SdkError::filesystem("open source rootfs", source, error))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|error| SdkError::filesystem("create temporary rootfs", temporary_path, error))?;
    let source_len = fs::metadata(source)
        .map_err(|error| SdkError::filesystem("inspect source rootfs", source, error))?
        .len();
    let expected = requested_size_bytes.max(source_len).max(1);
    let mut copied = 0_u64;
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
        copied += read as u64;
        on_copy_progress(copied.min(expected), expected);
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
    on_copy_progress(expected, expected);
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

    use super::{copy_rootfs, copy_stable_view_exact};
    use crate::error::SdkError;

    #[test]
    fn refuses_a_private_copy_larger_than_available_storage_before_creating_output() {
        let directory = tempdir().expect("temporary directory should be created");
        let source = directory.path().join("stable-view.ext4");
        let destination = directory.path().join("private-copy.ext4");
        fs::write(&source, b"stable-view").expect("source view should be written");

        let error = copy_stable_view_exact(
            &source,
            &destination,
            u64::MAX,
            tokio_util::sync::CancellationToken::new(),
            &mut |_, _| {},
        )
        .expect_err("an impossible copy size should be rejected");

        assert!(matches!(error, SdkError::SnapshotCapability { .. }));
        assert!(!destination.exists());
        assert_eq!(
            fs::read(&source).expect("source view should remain"),
            b"stable-view"
        );
    }

    #[test]
    fn copies_the_source_without_mutating_it() {
        let directory = tempdir().expect("temporary directory should exist");
        let source = directory.path().join("source.ext4");
        let temporary = directory.path().join("rootfs.part");
        let destination = directory.path().join("rootfs.ext4");
        fs::write(&source, b"verified source").expect("source should be written");

        let mut ticks = Vec::new();
        copy_rootfs(&source, &temporary, &destination, 32, &mut |done, total| {
            ticks.push((done, total));
        })
        .expect("rootfs copy and expansion should succeed");

        assert_eq!(
            fs::read(&source).expect("source should remain readable"),
            b"verified source"
        );
        assert_eq!(
            fs::metadata(&destination)
                .expect("destination should exist")
                .len(),
            32
        );
        assert!(!temporary.exists());
        assert!(!ticks.is_empty());
        assert_eq!(ticks.last(), Some(&(32, 32)));
        assert!(ticks.windows(2).all(|pair| pair[1].0 >= pair[0].0));
    }
}
