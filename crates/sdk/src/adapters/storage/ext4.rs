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

    fn disk_used_bytes(&self, rootfs_path: &Path) -> Result<u64, SdkError> {
        filesystem_minimum_bytes(rootfs_path)
    }

    fn resize_rootfs(&self, rootfs_path: &Path, new_size_bytes: u64) -> Result<(), SdkError> {
        resize_rootfs_file(rootfs_path, new_size_bytes)
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

fn regular_file_len(path: &Path, operation: &'static str) -> Result<u64, SdkError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| SdkError::filesystem(operation, path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: path.to_path_buf(),
            reason: "the root disk is not a regular file".to_owned(),
        });
    }
    Ok(metadata.len())
}

fn filesystem_geometry(rootfs_path: &Path) -> Result<(u64, u64), SdkError> {
    let output = Command::new("dumpe2fs")
        .args(["-h"])
        .arg(rootfs_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "dumpe2fs".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "inspect ext4 geometry".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("dumpe2fs exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut block_count = None;
    let mut block_size = None;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("Block count:")
            && let Ok(count) = value.trim().parse::<u64>()
            && count > 0
        {
            block_count = Some(count);
        }
        if let Some(value) = line.strip_prefix("Block size:")
            && let Ok(size) = value.trim().parse::<u64>()
            && size > 0
        {
            block_size = Some(size);
        }
    }
    match (block_count, block_size) {
        (Some(count), Some(size)) => Ok((count, size)),
        _ => Err(SdkError::GuestFilesystem {
            operation: "inspect ext4 geometry".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "dumpe2fs did not report the block count and size".to_owned(),
        }),
    }
}

fn filesystem_block_size(rootfs_path: &Path) -> Result<u64, SdkError> {
    filesystem_geometry(rootfs_path)
        .map(|(_, size)| size)
        .map_err(|error| match error {
            SdkError::GuestFilesystem {
                operation,
                path,
                reason,
            } if operation == "inspect ext4 geometry" => SdkError::GuestFilesystem {
                operation: "inspect ext4 block size".to_owned(),
                path,
                reason,
            },
            other => other,
        })
}

fn filesystem_minimum_bytes(rootfs_path: &Path) -> Result<u64, SdkError> {
    verify_ext4_filesystem(rootfs_path)?;
    let output = Command::new("resize2fs")
        .args(["-P", &rootfs_path.to_string_lossy()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "resize2fs".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "inspect ext4 minimum size".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("resize2fs exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let minimum_blocks = stdout
        .split_whitespace()
        .next_back()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| SdkError::GuestFilesystem {
            operation: "inspect ext4 minimum size".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "resize2fs did not report a minimum size".to_owned(),
        })?;
    let block_size = filesystem_block_size(rootfs_path)?;
    minimum_blocks
        .checked_mul(block_size)
        .ok_or_else(|| SdkError::GuestFilesystem {
            operation: "inspect ext4 minimum size".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "the filesystem minimum size exceeds the supported range".to_owned(),
        })
}

fn tool_stderr(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        return String::new();
    }
    // Keep the diagnostic short; tool output never carries secrets, only
    // filesystem state. Newlines collapse so the reason stays one line.
    // Truncate at a character boundary: tool output can contain non-UTF8
    // bytes echoed back as multi-byte replacement characters.
    let single_line = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.len() <= 300 {
        return single_line;
    }
    let mut end = 300;
    while !single_line.is_char_boundary(end) {
        end -= 1;
    }
    single_line[..end].to_owned()
}

fn run_e2fsck(rootfs_path: &Path) -> Result<(), SdkError> {
    let output = Command::new("e2fsck")
        .args(["-f", "-y"])
        .arg(rootfs_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "e2fsck".to_owned(),
            reason: error.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }
    // e2fsck exit status 1 means errors were corrected; that is expected
    // before a resize. Other statuses indicate an unusable filesystem.
    if output.status.code() == Some(1) {
        return Ok(());
    }
    let detail = tool_stderr(&output);
    Err(SdkError::GuestFilesystem {
        operation: "check ext4 filesystem before resize".to_owned(),
        path: rootfs_path.to_path_buf(),
        reason: if detail.is_empty() {
            format!("e2fsck exited with {}", output.status)
        } else {
            format!("e2fsck exited with {}: {detail}", output.status)
        },
    })
}

fn run_resize2fs(rootfs_path: &Path, size_argument: Option<String>) -> Result<(), SdkError> {
    let mut command = Command::new("resize2fs");
    command.arg(rootfs_path);
    if let Some(size) = size_argument.as_deref() {
        command.arg(size);
    }
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "resize2fs".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        let detail = tool_stderr(&output);
        return Err(SdkError::GuestFilesystem {
            operation: "resize ext4 filesystem".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: if detail.is_empty() {
                format!("resize2fs exited with {}", output.status)
            } else {
                format!("resize2fs exited with {}: {detail}", output.status)
            },
        });
    }
    Ok(())
}

fn resize_rootfs_file(rootfs_path: &Path, new_size_bytes: u64) -> Result<(), SdkError> {
    if new_size_bytes == 0 {
        return Err(SdkError::InvalidRequest {
            field: "disk_size_bytes".to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    let current_len = regular_file_len(rootfs_path, "inspect VM rootfs")?;
    if current_len == new_size_bytes {
        return Ok(());
    }
    verify_ext4_filesystem(rootfs_path)?;
    // A guest filesystem may carry a dirty journal or repaired-pending
    // errors from its last shutdown; resize2fs refuses those. Checking first
    // keeps grow and shrink on the same safe path.
    run_e2fsck(rootfs_path)?;
    if new_size_bytes > current_len {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(rootfs_path)
            .map_err(|error| SdkError::filesystem("expand VM rootfs", rootfs_path, error))?;
        file.set_len(new_size_bytes)
            .map_err(|error| SdkError::filesystem("expand VM rootfs", rootfs_path, error))?;
        drop(file);
        run_resize2fs(rootfs_path, None)?;
    } else {
        let sectors = new_size_bytes / 512;
        if sectors == 0 {
            return Err(SdkError::GuestFilesystem {
                operation: "resize ext4 filesystem".to_owned(),
                path: rootfs_path.to_path_buf(),
                reason: "the requested disk size is below one sector".to_owned(),
            });
        }
        run_resize2fs(rootfs_path, Some(format!("{sectors}s")))?;
    }
    // resize2fs reports success without touching the file when the
    // filesystem already has the requested size, and it rounds the file
    // down to the block size otherwise. Measure the real filesystem from
    // the superblock instead of trusting the file length, then converge
    // the file to exactly the requested size when the filesystem fits.
    let (block_count, block_size) = filesystem_geometry(rootfs_path)?;
    let filesystem_bytes =
        block_count
            .checked_mul(block_size)
            .ok_or_else(|| SdkError::GuestFilesystem {
                operation: "verify VM rootfs size".to_owned(),
                path: rootfs_path.to_path_buf(),
                reason: "the resized filesystem size exceeds the supported range".to_owned(),
            })?;
    if filesystem_bytes > new_size_bytes {
        return Err(SdkError::GuestFilesystem {
            operation: "verify VM rootfs size".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!(
                "the resized filesystem of {filesystem_bytes} bytes does not fit in {new_size_bytes} bytes"
            ),
        });
    }
    let resized_len = regular_file_len(rootfs_path, "verify VM rootfs")?;
    if resized_len != new_size_bytes {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(rootfs_path)
            .map_err(|error| SdkError::filesystem("verify VM rootfs", rootfs_path, error))?;
        file.set_len(new_size_bytes)
            .map_err(|error| SdkError::filesystem("verify VM rootfs", rootfs_path, error))?;
        drop(file);
    }
    verify_ext4_filesystem(rootfs_path)?;
    verify_rootfs_size(rootfs_path, new_size_bytes)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        copy_rootfs, copy_stable_view_exact, filesystem_minimum_bytes, resize_rootfs_file,
    };
    use crate::error::SdkError;
    use crate::ports::storage::GuestStorage;

    fn test_ext4_image(path: &std::path::Path, size_mb: u64) {
        let output = std::process::Command::new("truncate")
            .arg("-s")
            .arg(format!("{size_mb}M"))
            .arg(path)
            .output()
            .expect("truncate should run");
        assert!(output.status.success());
        let output = std::process::Command::new("mkfs.ext4")
            .args(["-q", "-F"])
            .arg(path)
            .output()
            .expect("mkfs.ext4 should run");
        assert!(output.status.success(), "mkfs.ext4 should succeed");
    }

    #[test]
    fn reports_a_usable_minimum_below_the_current_size() {
        let directory = tempdir().expect("temporary directory should be created");
        let rootfs = directory.path().join("rootfs.ext4");
        test_ext4_image(&rootfs, 64);
        let current = fs::metadata(&rootfs).expect("rootfs metadata").len();

        let used = filesystem_minimum_bytes(&rootfs).expect("minimum should be reported");
        assert!(used > 0);
        assert!(used < current);
    }

    #[test]
    fn grows_and_shrinks_a_real_ext4_image() {
        let directory = tempdir().expect("temporary directory should be created");
        let rootfs = directory.path().join("rootfs.ext4");
        test_ext4_image(&rootfs, 64);
        let storage = super::Ext4Storage;

        storage
            .resize_rootfs(&rootfs, 80 * 1024 * 1024)
            .expect("grow should succeed");
        assert_eq!(
            fs::metadata(&rootfs).expect("rootfs metadata").len(),
            80 * 1024 * 1024
        );

        storage
            .resize_rootfs(&rootfs, 40 * 1024 * 1024)
            .expect("shrink should succeed");
        assert_eq!(
            fs::metadata(&rootfs).expect("rootfs metadata").len(),
            40 * 1024 * 1024
        );
    }

    #[test]
    fn resize_is_a_no_op_for_the_current_size() {
        let directory = tempdir().expect("temporary directory should be created");
        let rootfs = directory.path().join("rootfs.ext4");
        test_ext4_image(&rootfs, 32);
        let before = fs::read(&rootfs).expect("rootfs should be readable");
        resize_rootfs_file(&rootfs, before.len() as u64).expect("no-op resize should succeed");
        assert_eq!(
            fs::read(&rootfs).expect("rootfs should be readable"),
            before
        );
    }

    #[test]
    fn converges_a_file_larger_than_its_filesystem() {
        let directory = tempdir().expect("temporary directory should be created");
        let rootfs = directory.path().join("rootfs.ext4");
        test_ext4_image(&rootfs, 64);
        // A failed grow leaves the file extended while the filesystem keeps
        // its old size; resize2fs then reports success without touching the
        // file. The resize must converge the file instead of failing.
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&rootfs)
            .expect("rootfs should open");
        file.set_len(80 * 1024 * 1024).expect("file should extend");
        drop(file);
        resize_rootfs_file(&rootfs, 64 * 1024 * 1024)
            .expect("converging the file to the filesystem size should succeed");
        assert_eq!(
            fs::metadata(&rootfs).expect("rootfs metadata").len(),
            64 * 1024 * 1024
        );
        super::verify_ext4_filesystem(&rootfs).expect("filesystem should stay valid");
    }

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
