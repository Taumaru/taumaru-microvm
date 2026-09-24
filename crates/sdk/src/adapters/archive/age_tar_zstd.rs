use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use age::secrecy::SecretString;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::domain::registry::{Architecture, Kernel};
use crate::domain::snapshot::{SnapshotAddressPolicy, SnapshotProgress, SnapshotProgressStage};
use crate::error::SdkError;

use super::manifest::{
    BootManifest, CompatibilityManifest, ConsistencyManifest, FORMAT_ID, FORMAT_VERSION,
    KernelManifest, NetworkManifest, PayloadRecord, ROOTFS_MEMBER, SnapshotManifest, SshManifest,
    VmManifest,
};
const COPY_BUFFER_SIZE: usize = 128 * 1024;
const STATUS_CHECK_INTERVAL: u64 = 64 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) struct SnapshotArchiveInput {
    pub vm_name: String,
    pub output_path: PathBuf,
    pub password: String,
    pub metadata: SnapshotPortableMetadata,
    pub payloads: Vec<SnapshotPayload>,
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotPortableMetadata {
    pub guest_architecture: String,
    pub distribution_id: String,
    pub distribution_name: String,
    pub distribution_version: String,
    pub image_id: String,
    pub image_sha256: String,
    pub kernel: Kernel,
    pub disk_size_bytes: u64,
    pub memory_bytes: u64,
    pub memory_effective_mib: u64,
    pub vcpu_count: u32,
    pub root_device: String,
    pub kernel_args: Vec<String>,
    pub address_policy: SnapshotAddressPolicy,
    pub network_mode: String,
    pub expose_on_lan: bool,
    pub guest_ipv4: Option<std::net::Ipv4Addr>,
    pub prefix_length: Option<u8>,
    pub guest_gateway_ipv4: Option<std::net::Ipv4Addr>,
    pub lan_ipv4: Option<std::net::Ipv4Addr>,
    pub guest_mac: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub ssh_key_type: String,
    pub ssh_public_key_fingerprint: String,
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotPayload {
    pub archive_path: &'static str,
    pub source_path: PathBuf,
    pub mode: u32,
    pub expected_size_bytes: Option<u64>,
    pub expected_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotArchiveResult {
    pub output_path: PathBuf,
    pub encrypted_size_bytes: u64,
}

pub(crate) fn publish_staged_archive(
    staged: SnapshotArchiveResult,
    requested_output_path: &Path,
) -> Result<SnapshotArchiveResult, SdkError> {
    let output_path = normalize_output_path(requested_output_path)?;
    if path_entry_exists(&output_path)? {
        let primary = SdkError::SnapshotOutputExists { path: output_path };
        let cleanup = fs::remove_file(&staged.output_path).err().map(|error| {
            SdkError::filesystem(
                "remove unpublished staged snapshot",
                &staged.output_path,
                error,
            )
        });
        return Err(with_cleanup(primary, cleanup));
    }
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let Err(error) = fs::hard_link(&staged.output_path, &output_path) {
        let primary = if error.kind() == io::ErrorKind::AlreadyExists {
            SdkError::SnapshotOutputExists {
                path: output_path.clone(),
            }
        } else {
            SdkError::filesystem("publish encrypted snapshot", &output_path, error)
        };
        let cleanup = fs::remove_file(&staged.output_path).err().map(|source| {
            SdkError::filesystem(
                "remove unpublished staged snapshot",
                &staged.output_path,
                source,
            )
        });
        return Err(with_cleanup(primary, cleanup));
    }
    if let Err(error) = fs::remove_file(&staged.output_path) {
        let primary = SdkError::filesystem(
            "remove staged encrypted snapshot",
            &staged.output_path,
            error,
        );
        let output_cleanup = fs::remove_file(&output_path).err().map(|source| {
            SdkError::filesystem(
                "remove unpublished encrypted snapshot",
                &output_path,
                source,
            )
        });
        return Err(with_cleanup(primary, output_cleanup));
    }
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        let primary = SdkError::filesystem("sync snapshot output directory", parent, error);
        let output_cleanup = fs::remove_file(&output_path).err().map(|source| {
            SdkError::filesystem(
                "remove unpublished encrypted snapshot",
                &output_path,
                source,
            )
        });
        return Err(with_cleanup(primary, output_cleanup));
    }
    Ok(SnapshotArchiveResult {
        output_path,
        encrypted_size_bytes: staged.encrypted_size_bytes,
    })
}

#[cfg(test)]
pub(crate) fn create_archive<F>(
    input: SnapshotArchiveInput,
    cancellation: CancellationToken,
    check_view: F,
) -> Result<SnapshotArchiveResult, SdkError>
where
    F: Fn() -> Result<(), SdkError>,
{
    create_archive_with_progress(input, cancellation, check_view, |_| {})
}

pub(crate) fn create_archive_with_progress<F, P>(
    mut input: SnapshotArchiveInput,
    cancellation: CancellationToken,
    check_view: F,
    mut on_progress: P,
) -> Result<SnapshotArchiveResult, SdkError>
where
    F: Fn() -> Result<(), SdkError>,
    P: FnMut(SnapshotProgress),
{
    if input.password.is_empty() {
        return Err(SdkError::InvalidRequest {
            field: "password".to_owned(),
            reason: "must not be empty".to_owned(),
        });
    }
    let output_path = normalize_output_path(&input.output_path)?;
    if path_entry_exists(&output_path)? {
        return Err(SdkError::SnapshotOutputExists { path: output_path });
    }
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_metadata = fs::metadata(parent).map_err(|error| {
        SdkError::filesystem("inspect snapshot output directory", parent, error)
    })?;
    if !parent_metadata.is_dir() {
        return Err(SdkError::InvalidRequest {
            field: "output_path".to_owned(),
            reason: "the parent path must be a directory".to_owned(),
        });
    }

    let total_bytes = payload_total_bytes(&input.payloads)?;
    let (temp_path, temp_file) = create_private_temp(parent)?;
    let mut temp_guard = TempPathGuard::new(temp_path.clone());
    on_progress(SnapshotProgress {
        stage: SnapshotProgressStage::StreamingPayloads,
        completed_bytes: 0,
        total_bytes,
    });
    let result = write_encrypted_stream(
        temp_file,
        &mut input,
        cancellation.clone(),
        &check_view,
        total_bytes,
        &mut on_progress,
    );
    let temp_file = match result {
        Ok(file) => file,
        Err(error) => return Err(with_cleanup(error, temp_guard.remove().err())),
    };
    if let Err(source) = temp_file.sync_all() {
        let primary = SdkError::filesystem("sync encrypted snapshot", &temp_path, source);
        drop(temp_file);
        return Err(with_cleanup(primary, temp_guard.remove().err()));
    }
    let encrypted_size_bytes = match temp_file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(source) => {
            let primary = SdkError::filesystem("inspect encrypted snapshot", &temp_path, source);
            drop(temp_file);
            return Err(with_cleanup(primary, temp_guard.remove().err()));
        }
    };
    drop(temp_file);
    if cancellation.is_cancelled() {
        return Err(with_cleanup(
            SdkError::SnapshotCancelled,
            temp_guard.remove().err(),
        ));
    }
    if let Err(error) = check_view() {
        return Err(with_cleanup(error, temp_guard.remove().err()));
    }

    match fs::hard_link(&temp_path, &output_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(with_cleanup(
                SdkError::SnapshotOutputExists { path: output_path },
                temp_guard.remove().err(),
            ));
        }
        Err(error) => {
            let primary = SdkError::filesystem("publish encrypted snapshot", &output_path, error);
            return Err(with_cleanup(primary, temp_guard.remove().err()));
        }
    }
    if let Err(primary) = temp_guard.remove() {
        let output_cleanup = fs::remove_file(&output_path).err().map(|source| {
            SdkError::filesystem(
                "remove unpublished encrypted snapshot",
                &output_path,
                source,
            )
        });
        return Err(with_cleanup(primary, output_cleanup));
    }
    if let Err(source) = File::open(parent).and_then(|directory| directory.sync_all()) {
        let primary = SdkError::filesystem("sync snapshot output directory", parent, source);
        let output_cleanup = fs::remove_file(&output_path).err().map(|cleanup| {
            SdkError::filesystem(
                "remove unpublished encrypted snapshot",
                &output_path,
                cleanup,
            )
        });
        return Err(with_cleanup(primary, output_cleanup));
    }
    Ok(SnapshotArchiveResult {
        output_path,
        encrypted_size_bytes,
    })
}

fn payload_total_bytes(payloads: &[SnapshotPayload]) -> Result<u64, SdkError> {
    payloads.iter().try_fold(0_u64, |total, payload| {
        let metadata = fs::symlink_metadata(&payload.source_path).map_err(|error| {
            SdkError::filesystem("inspect snapshot payload", &payload.source_path, error)
        })?;
        let payload_bytes = payload.expected_size_bytes.unwrap_or(metadata.len());
        total
            .checked_add(payload_bytes)
            .ok_or_else(|| SdkError::SnapshotArchive {
                operation: "calculate snapshot progress",
                reason: "total payload size exceeds the supported byte count".to_owned(),
            })
    })
}

fn write_encrypted_stream<F, P>(
    output: File,
    input: &mut SnapshotArchiveInput,
    cancellation: CancellationToken,
    check_view: &F,
    total_payload_bytes: u64,
    on_progress: &mut P,
) -> Result<File, SdkError>
where
    F: Fn() -> Result<(), SdkError>,
    P: FnMut(SnapshotProgress),
{
    let secret = SecretString::from(std::mem::take(&mut input.password));
    let encryptor = age::Encryptor::with_user_passphrase(secret);
    let age_writer = encryptor
        .wrap_output(output)
        .map_err(|error| archive_error("start age encryption", error))?;
    let zstd_writer = zstd::stream::write::Encoder::new(age_writer, 3)
        .map_err(|error| archive_error("start Zstandard compression", error))?;
    let mut tar_writer = tar::Builder::new(zstd_writer);
    let mut records = Vec::with_capacity(input.payloads.len());
    let mut completed_payload_bytes = 0_u64;

    for payload in &input.payloads {
        if cancellation.is_cancelled() {
            return Err(SdkError::SnapshotCancelled);
        }
        check_view()?;
        let metadata = fs::symlink_metadata(&payload.source_path).map_err(|error| {
            SdkError::filesystem("inspect snapshot payload", &payload.source_path, error)
        })?;
        let source_is_block_device = if payload.archive_path == ROOTFS_MEMBER {
            if metadata.file_type().is_symlink() {
                let target_metadata = fs::metadata(&payload.source_path).map_err(|error| {
                    SdkError::filesystem(
                        "inspect snapshot block device target",
                        &payload.source_path,
                        error,
                    )
                })?;
                is_block_device(&target_metadata)
            } else {
                is_block_device(&metadata)
            }
        } else {
            false
        };
        if !metadata.file_type().is_file() && !source_is_block_device {
            return Err(SdkError::InvalidRequest {
                field: "snapshot_payload".to_owned(),
                reason: format!(
                    "{} must be a regular file or the online snapshot block device",
                    payload.archive_path
                ),
            });
        }
        let expected_size = payload.expected_size_bytes.unwrap_or(metadata.len());
        if !source_is_block_device && metadata.len() != expected_size {
            return Err(SdkError::SnapshotArchive {
                operation: "validate snapshot payload",
                reason: format!(
                    "{} changed size before it could be archived",
                    payload.archive_path
                ),
            });
        }
        let source = File::open(&payload.source_path).map_err(|error| {
            SdkError::filesystem("open snapshot payload", &payload.source_path, error)
        })?;
        let mut reader = GuardedDigestReader::new(
            source,
            expected_size,
            cancellation.clone(),
            check_view,
            completed_payload_bytes,
            total_payload_bytes,
            on_progress,
        );
        let mut header = tar::Header::new_gnu();
        header.set_size(expected_size);
        header.set_mode(payload.mode);
        header.set_mtime(0);
        header.set_cksum();
        let append_result = tar_writer.append_data(&mut header, payload.archive_path, &mut reader);
        if let Some(error) = reader.guard_error.take() {
            return Err(error);
        }
        append_result.map_err(|error| archive_error("write snapshot payload", error))?;
        if reader.bytes_read != expected_size {
            return Err(SdkError::SnapshotArchive {
                operation: "validate snapshot payload",
                reason: format!("{} ended before its declared size", payload.archive_path),
            });
        }
        let actual_metadata = fs::symlink_metadata(&payload.source_path).map_err(|error| {
            SdkError::filesystem("reinspect snapshot payload", &payload.source_path, error)
        })?;
        if (!source_is_block_device && actual_metadata.len() != expected_size)
            || actual_metadata.ino() != metadata.ino()
        {
            return Err(SdkError::SnapshotArchive {
                operation: "validate snapshot payload",
                reason: format!(
                    "{} changed while it was being archived",
                    payload.archive_path
                ),
            });
        }
        check_view()?;
        let bytes_read = reader.bytes_read;
        let sha256 = hex_digest(reader.hasher.clone().finalize());
        drop(reader);
        if payload
            .expected_sha256
            .as_ref()
            .is_some_and(|expected| expected != &sha256)
        {
            return Err(SdkError::SnapshotArchive {
                operation: "verify snapshot payload",
                reason: format!(
                    "{} does not match its locally verified digest",
                    payload.archive_path
                ),
            });
        }
        completed_payload_bytes = completed_payload_bytes.saturating_add(bytes_read);
        records.push(PayloadRecord {
            path: payload.archive_path.to_owned(),
            size_bytes: bytes_read,
            sha256,
            mode: payload.mode,
        });
    }

    on_progress(SnapshotProgress {
        stage: SnapshotProgressStage::FinalizingArchive,
        completed_bytes: total_payload_bytes,
        total_bytes: total_payload_bytes,
    });

    let created_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SdkError::SnapshotArchive {
            operation: "create snapshot manifest",
            reason: format!("system clock predates the Unix epoch: {error}"),
        })?
        .as_secs();
    let network = match input.metadata.address_policy {
        SnapshotAddressPolicy::PreserveIpv4 => NetworkManifest::PreserveIpv4 {
            guest_ipv4: input.metadata.guest_ipv4.ok_or_else(|| {
                SdkError::InvalidSnapshotManifest {
                    reason: "preserve_ipv4 policy requires a guest IPv4 address".to_owned(),
                }
            })?,
            prefix_length: input.metadata.prefix_length.ok_or_else(|| {
                SdkError::InvalidSnapshotManifest {
                    reason: "preserve_ipv4 policy requires an IPv4 prefix length".to_owned(),
                }
            })?,
            guest_gateway_ipv4: input.metadata.guest_gateway_ipv4,
            lan_ipv4: input.metadata.lan_ipv4,
            mode: input.metadata.network_mode.clone(),
            expose_on_lan: input.metadata.expose_on_lan,
            guest_mac: input.metadata.guest_mac.clone(),
        },
        SnapshotAddressPolicy::RegenerateIpv4 => NetworkManifest::RegenerateIpv4 {
            expose_on_lan: input.metadata.expose_on_lan,
        },
    };
    let manifest = SnapshotManifest {
        format: FORMAT_ID.to_owned(),
        format_version: FORMAT_VERSION,
        created_at_unix_seconds,
        vm: VmManifest {
            name: input.vm_name.clone(),
            disk_size_bytes: input.metadata.disk_size_bytes,
            memory_bytes: input.metadata.memory_bytes,
            memory_effective_mib: input.metadata.memory_effective_mib,
            vcpu_count: input.metadata.vcpu_count,
        },
        compatibility: CompatibilityManifest {
            host_os: "linux".to_owned(),
            guest_architecture: input.metadata.guest_architecture.clone(),
            requires_kvm: true,
            runtime: "firecracker".to_owned(),
        },
        boot: BootManifest {
            distribution_id: input.metadata.distribution_id.clone(),
            distribution_name: input.metadata.distribution_name.clone(),
            distribution_version: input.metadata.distribution_version.clone(),
            image_id: input.metadata.image_id.clone(),
            image_sha256: input.metadata.image_sha256.clone(),
            kernel: KernelManifest {
                id: input.metadata.kernel.id.clone(),
                name: input.metadata.kernel.name.clone(),
                display_name: input.metadata.kernel.display_name.clone(),
                version: input.metadata.kernel.version.clone(),
                architecture: architecture_name(&input.metadata.kernel.architecture).to_owned(),
                registry_path: input.metadata.kernel.path.clone(),
                registry_url: input.metadata.kernel.url.clone(),
                filename: input.metadata.kernel.filename.clone(),
                format: input.metadata.kernel.format.clone(),
                mime_type: input.metadata.kernel.mime_type.clone(),
                modified_at: input.metadata.kernel.modified_at.clone(),
            },
            root_device: input.metadata.root_device.clone(),
            kernel_args: input.metadata.kernel_args.clone(),
        },
        network,
        ssh: SshManifest {
            user: input.metadata.ssh_user.clone(),
            port: input.metadata.ssh_port,
            key_type: input.metadata.ssh_key_type.clone(),
            public_key_fingerprint: input.metadata.ssh_public_key_fingerprint.clone(),
        },
        consistency: ConsistencyManifest {
            kind: "disk_only_crash_consistent".to_owned(),
            includes_guest_memory: false,
            includes_process_state: false,
        },
        payloads: records,
    };
    manifest.validate()?;
    let manifest_bytes =
        serde_json::to_vec(&manifest).map_err(|error| SdkError::SnapshotArchive {
            operation: "serialize snapshot manifest",
            reason: error.to_string(),
        })?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(created_at_unix_seconds);
    header.set_cksum();
    tar_writer
        .append_data(&mut header, "manifest.json", manifest_bytes.as_slice())
        .map_err(|error| archive_error("append snapshot manifest", error))?;
    tar_writer
        .finish()
        .map_err(|error| archive_error("finish TAR archive", error))?;
    let zstd_writer = tar_writer
        .into_inner()
        .map_err(|error| archive_error("finalize TAR archive", error))?;
    let age_writer = zstd_writer
        .finish()
        .map_err(|error| archive_error("finish Zstandard compression", error))?;
    age_writer
        .finish()
        .map_err(|error| archive_error("finish age encryption", error))
}

fn architecture_name(architecture: &Architecture) -> &'static str {
    match architecture {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::Riscv64 => "riscv64",
        Architecture::X86 => "x86",
    }
}

struct GuardedDigestReader<'a, F, P>
where
    F: Fn() -> Result<(), SdkError>,
    P: FnMut(SnapshotProgress),
{
    source: File,
    expected_size: u64,
    bytes_read: u64,
    hasher: Sha256,
    cancellation: CancellationToken,
    check_view: &'a F,
    completed_payload_bytes: u64,
    total_payload_bytes: u64,
    on_progress: &'a mut P,
    next_status_check: u64,
    guard_error: Option<SdkError>,
}

impl<'a, F, P> GuardedDigestReader<'a, F, P>
where
    F: Fn() -> Result<(), SdkError>,
    P: FnMut(SnapshotProgress),
{
    fn new(
        source: File,
        expected_size: u64,
        cancellation: CancellationToken,
        check_view: &'a F,
        completed_payload_bytes: u64,
        total_payload_bytes: u64,
        on_progress: &'a mut P,
    ) -> Self {
        Self {
            source,
            expected_size,
            bytes_read: 0,
            hasher: Sha256::new(),
            cancellation,
            check_view,
            completed_payload_bytes,
            total_payload_bytes,
            on_progress,
            next_status_check: 0,
            guard_error: None,
        }
    }
}

impl<F, P> Read for GuardedDigestReader<'_, F, P>
where
    F: Fn() -> Result<(), SdkError>,
    P: FnMut(SnapshotProgress),
{
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.bytes_read >= self.expected_size {
            return Ok(0);
        }
        if self.cancellation.is_cancelled() {
            self.guard_error = Some(SdkError::SnapshotCancelled);
            return Err(io::Error::other("snapshot cancelled"));
        }
        if self.bytes_read >= self.next_status_check {
            if let Err(error) = (self.check_view)() {
                self.guard_error = Some(error);
                return Err(io::Error::other("snapshot view validation failed"));
            }
            self.next_status_check = self.bytes_read.saturating_add(STATUS_CHECK_INTERVAL);
        }
        let read_limit = buffer
            .len()
            .min(COPY_BUFFER_SIZE)
            .min((self.expected_size - self.bytes_read) as usize);
        let read =
            match self.source.read(&mut buffer[..read_limit]) {
                Ok(read) => read,
                Err(source_error) => {
                    self.guard_error = Some((self.check_view)().err().unwrap_or_else(|| {
                        SdkError::SnapshotArchive {
                            operation: "read snapshot payload",
                            reason: source_error.to_string(),
                        }
                    }));
                    return Err(source_error);
                }
            };
        if read == 0 && self.bytes_read < self.expected_size {
            self.guard_error =
                Some(
                    (self.check_view)()
                        .err()
                        .unwrap_or_else(|| SdkError::SnapshotArchive {
                            operation: "read snapshot payload",
                            reason: "the source ended before the recorded byte count".to_owned(),
                        }),
                );
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "snapshot payload ended early",
            ));
        }
        self.bytes_read += read as u64;
        self.hasher.update(&buffer[..read]);
        (self.on_progress)(SnapshotProgress {
            stage: SnapshotProgressStage::StreamingPayloads,
            completed_bytes: self.completed_payload_bytes.saturating_add(self.bytes_read),
            total_bytes: self.total_payload_bytes,
        });
        Ok(read)
    }
}

#[cfg(unix)]
fn is_block_device(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    metadata.file_type().is_block_device()
}

#[cfg(not(unix))]
fn is_block_device(_metadata: &fs::Metadata) -> bool {
    false
}

fn normalize_output_path(path: &Path) -> Result<PathBuf, SdkError> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(SdkError::InvalidRequest {
            field: "output_path".to_owned(),
            reason: "must name a file".to_owned(),
        });
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| SdkError::filesystem("resolve snapshot output path", path, error))
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem("inspect snapshot output", path, error)),
    }
}

struct TempPathGuard {
    path: PathBuf,
    armed: bool,
}

impl TempPathGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn remove(&mut self) -> Result<(), SdkError> {
        if !self.armed {
            return Ok(());
        }
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(SdkError::filesystem(
                "remove encrypted snapshot temporary file",
                &self.path,
                error,
            )),
        }
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_private_temp(parent: &Path) -> Result<(PathBuf, File), SdkError> {
    for _ in 0..128 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".tmvm-snapshot-{}-{nonce}-{sequence}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).read(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(SdkError::filesystem(
                    "create encrypted snapshot temporary file",
                    path,
                    error,
                ));
            }
        }
    }
    Err(SdkError::SnapshotArchive {
        operation: "create encrypted snapshot temporary file",
        reason: "could not reserve a unique temporary path".to_owned(),
    })
}

fn archive_error(operation: &'static str, error: io::Error) -> SdkError {
    SdkError::SnapshotArchive {
        operation,
        reason: error.to_string(),
    }
}

fn with_cleanup(primary: SdkError, cleanup: Option<SdkError>) -> SdkError {
    match cleanup {
        Some(cleanup) => SdkError::Cleanup {
            primary: primary.to_string(),
            failures: vec![cleanup.to_string()],
        },
        None => primary,
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
trait MetadataInode {
    fn ino(&self) -> u64;
}

#[cfg(unix)]
impl MetadataInode for fs::Metadata {
    fn ino(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        MetadataExt::ino(self)
    }
}

#[cfg(not(unix))]
trait MetadataInode {
    fn ino(&self) -> u64;
}

#[cfg(not(unix))]
impl MetadataInode for fs::Metadata {
    fn ino(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufReader, Read};
    use std::path::{Path, PathBuf};

    use age::secrecy::SecretString;
    use sha2::{Digest, Sha256};
    use tokio_util::sync::CancellationToken;

    use super::{SnapshotArchiveInput, SnapshotPayload, SnapshotPortableMetadata, create_archive};
    use crate::adapters::archive::manifest::{
        KERNEL_MEMBER, PRIVATE_KEY_MEMBER, PUBLIC_KEY_MEMBER, ROOTFS_MEMBER,
    };
    use crate::domain::registry::Architecture;
    use crate::domain::snapshot::SnapshotAddressPolicy;

    fn metadata() -> SnapshotPortableMetadata {
        SnapshotPortableMetadata {
            guest_architecture: "x86_64".to_owned(),
            distribution_id: "distro-1".to_owned(),
            distribution_name: "Linux Fixture".to_owned(),
            distribution_version: "1.0".to_owned(),
            image_id: "image-1".to_owned(),
            image_sha256: "a".repeat(64),
            kernel: crate::domain::registry::Kernel {
                id: "kernel-1".to_owned(),
                name: "kernel".to_owned(),
                display_name: "Kernel".to_owned(),
                version: "6.0".to_owned(),
                architecture: Architecture::X86_64,
                path: "kernels/kernel/vmlinux".to_owned(),
                url: "https://example.test/kernel".to_owned(),
                filename: "vmlinux".to_owned(),
                size_bytes: 6,
                sha256: hex_sha256(b"kernel"),
                format: "elf".to_owned(),
                mime_type: "application/octet-stream".to_owned(),
                elf: None,
                modified_at: "2026-01-01T00:00:00Z".to_owned(),
            },
            disk_size_bytes: 4,
            memory_bytes: 128 * 1024 * 1024,
            memory_effective_mib: 128,
            vcpu_count: 1,
            root_device: "/dev/vda".to_owned(),
            kernel_args: vec!["console=ttyS0".to_owned()],
            address_policy: SnapshotAddressPolicy::PreserveIpv4,
            network_mode: "host_only".to_owned(),
            expose_on_lan: false,
            guest_ipv4: Some("192.0.2.2".parse().expect("valid IPv4")),
            prefix_length: Some(30),
            guest_gateway_ipv4: Some("192.0.2.1".parse().expect("valid IPv4")),
            lan_ipv4: None,
            guest_mac: "02:00:00:00:00:01".to_owned(),
            ssh_user: "root".to_owned(),
            ssh_port: 22,
            ssh_key_type: "ed25519".to_owned(),
            ssh_public_key_fingerprint: "SHA256:fixture".to_owned(),
        }
    }

    fn create_test_archive(
        directory: &Path,
        output_name: &str,
        policy: SnapshotAddressPolicy,
    ) -> (PathBuf, PathBuf) {
        let disk = directory.join("rootfs.ext4");
        fs::write(&disk, b"disk").expect("fixture disk should be written");
        let kernel = directory.join("vmlinux");
        let private_key = directory.join("id_ed25519");
        let public_key = directory.join("id_ed25519.pub");
        fs::write(&kernel, b"kernel").expect("kernel fixture should be written");
        fs::write(&private_key, b"private").expect("private key fixture should be written");
        fs::write(&public_key, b"public").expect("public key fixture should be written");
        let output = directory.join(output_name);
        let mut portable_metadata = metadata();
        portable_metadata.address_policy = policy;
        let input = SnapshotArchiveInput {
            vm_name: "fixture-vm".to_owned(),
            output_path: output.clone(),
            password: "correct horse battery staple".to_owned(),
            metadata: portable_metadata,
            payloads: vec![
                SnapshotPayload {
                    archive_path: ROOTFS_MEMBER,
                    source_path: disk.clone(),
                    mode: 0o600,
                    expected_size_bytes: Some(4),
                    expected_sha256: None,
                },
                SnapshotPayload {
                    archive_path: KERNEL_MEMBER,
                    source_path: kernel,
                    mode: 0o644,
                    expected_size_bytes: Some(6),
                    expected_sha256: Some(hex_sha256(b"kernel")),
                },
                SnapshotPayload {
                    archive_path: PRIVATE_KEY_MEMBER,
                    source_path: private_key,
                    mode: 0o600,
                    expected_size_bytes: Some(7),
                    expected_sha256: None,
                },
                SnapshotPayload {
                    archive_path: PUBLIC_KEY_MEMBER,
                    source_path: public_key,
                    mode: 0o644,
                    expected_size_bytes: Some(6),
                    expected_sha256: None,
                },
            ],
        };
        let result = create_archive(input, CancellationToken::new(), || Ok(()))
            .expect("encrypted archive should be created");
        assert!(result.encrypted_size_bytes > 0);
        (output, disk)
    }

    fn decrypt(path: &Path, password: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let encrypted = fs::File::open(path)?;
        let decryptor = age::Decryptor::new(BufReader::new(encrypted))?;
        let identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
        let reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
        let mut decoder = zstd::stream::read::Decoder::new(reader)?;
        let mut plaintext = Vec::new();
        decoder.read_to_end(&mut plaintext)?;
        Ok(plaintext)
    }

    #[test]
    fn encrypted_tar_places_integrity_manifest_last_and_keeps_temp_private() {
        let directory = tempfile::tempdir().expect("test directory should be created");
        let (archive_path, _) = create_test_archive(
            directory.path(),
            "snapshot.tmvmsnap",
            SnapshotAddressPolicy::PreserveIpv4,
        );
        let plaintext = decrypt(&archive_path, "correct horse battery staple")
            .expect("correct passphrase should decrypt the archive");
        let mut archive = tar::Archive::new(plaintext.as_slice());
        let mut members = Vec::new();
        for entry in archive.entries().expect("TAR entries should parse") {
            let mut entry = entry.expect("TAR member should parse");
            let path = entry
                .path()
                .expect("member path should parse")
                .to_string_lossy()
                .into_owned();
            let mode = entry.header().mode().expect("member mode should parse");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("member should read");
            members.push((path, mode, bytes));
        }
        assert_eq!(members[0].0, "payload/rootfs.ext4");
        assert_eq!(members[0].1, 0o600);
        assert_eq!(members[0].2, b"disk");
        assert_eq!(
            members.last().map(|member| member.0.as_str()),
            Some("manifest.json")
        );
        assert_eq!(members.last().map(|member| member.1), Some(0o600));
        let manifest: serde_json::Value = serde_json::from_slice(&members.last().unwrap().2)
            .expect("manifest should be valid JSON");
        assert_eq!(manifest["format"], "taumaru.microvm.snapshot");
        assert_eq!(manifest["format_version"], 2);
        assert_eq!(manifest["boot"]["kernel"]["id"], "kernel-1");
        assert_eq!(manifest["network"]["address_policy"], "preserve_ipv4");
        assert_eq!(manifest["network"]["guest_ipv4"], "192.0.2.2");
        assert_eq!(manifest["payloads"][0]["path"], "payload/rootfs.ext4");
        assert_eq!(manifest["payloads"][0]["size_bytes"], 4);
        assert_eq!(manifest["payloads"][0]["sha256"], hex_sha256(b"disk"));
        let files = fs::read_dir(directory.path())
            .expect("output directory should list")
            .map(|entry| entry.expect("directory entry should read").file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            files.len(),
            5,
            "only the four source fixtures and encrypted archive remain"
        );
        assert!(archive_path.exists());
    }

    #[test]
    fn regenerate_manifest_omits_all_source_ipv4_assignments() {
        let directory = tempfile::tempdir().expect("test directory should be created");
        let (archive_path, _) = create_test_archive(
            directory.path(),
            "regenerate.tmvmsnap",
            SnapshotAddressPolicy::RegenerateIpv4,
        );
        let plaintext = decrypt(&archive_path, "correct horse battery staple")
            .expect("correct passphrase should decrypt the archive");
        let mut archive = tar::Archive::new(plaintext.as_slice());
        let mut bytes = Vec::new();
        let mut found_manifest = false;
        for entry in archive.entries().expect("TAR entries should parse") {
            let mut entry = entry.expect("TAR member should parse");
            let path = entry
                .path()
                .expect("member path should parse")
                .to_string_lossy()
                .into_owned();
            if path == "manifest.json" {
                entry.read_to_end(&mut bytes).expect("manifest should read");
                found_manifest = true;
                break;
            }
        }
        assert!(found_manifest, "manifest should be present in the archive");
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).expect("manifest JSON");
        let network = manifest["network"].as_object().expect("network object");
        assert_eq!(network.len(), 2);
        assert_eq!(network["address_policy"], "regenerate_ipv4");
        assert_eq!(network["expose_on_lan"], false);
        for field in [
            "guest_ipv4",
            "prefix_length",
            "guest_gateway_ipv4",
            "lan_ipv4",
            "mode",
            "guest_mac",
        ] {
            assert!(
                !network.contains_key(field),
                "regenerate policy must omit {field}"
            );
        }
    }

    #[test]
    fn wrong_password_tampering_and_truncation_are_rejected() {
        let directory = tempfile::tempdir().expect("test directory should be created");
        let (archive_path, _) = create_test_archive(
            directory.path(),
            "snapshot.tmvmsnap",
            SnapshotAddressPolicy::PreserveIpv4,
        );
        assert!(decrypt(&archive_path, "wrong password").is_err());

        let original = fs::read(&archive_path).expect("archive should read");
        let mut tampered = original.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        fs::write(&archive_path, tampered).expect("tampered archive should write");
        assert!(decrypt(&archive_path, "correct horse battery staple").is_err());

        fs::write(&archive_path, &original[..original.len() - 5])
            .expect("truncated archive should write");
        assert!(decrypt(&archive_path, "correct horse battery staple").is_err());
    }

    #[test]
    fn cancellation_does_not_publish_an_archive() {
        let directory = tempfile::tempdir().expect("test directory should be created");
        let disk = directory.path().join("rootfs.ext4");
        fs::write(&disk, b"disk").expect("fixture disk should be written");
        let output = directory.path().join("cancelled.tmvmsnap");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let input = SnapshotArchiveInput {
            vm_name: "fixture-vm".to_owned(),
            output_path: output.clone(),
            password: "password".to_owned(),
            metadata: metadata(),
            payloads: vec![SnapshotPayload {
                archive_path: "payload/rootfs.ext4",
                source_path: disk,
                mode: 0o600,
                expected_size_bytes: Some(4),
                expected_sha256: None,
            }],
        };
        assert!(matches!(
            create_archive(input, cancellation, || Ok(())),
            Err(crate::error::SdkError::SnapshotCancelled)
        ));
        assert!(!output.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    fn hex_sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
