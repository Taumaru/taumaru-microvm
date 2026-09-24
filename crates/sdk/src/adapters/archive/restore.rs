use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::manifest::{
    KERNEL_MEMBER, MAX_MANIFEST_BYTES, PRIVATE_KEY_MEMBER, PUBLIC_KEY_MEMBER, ROOTFS_MEMBER,
    SnapshotManifest, parse_manifest,
};
use crate::adapters::credentials::ed25519::validate_ed25519_key_pair;
use crate::error::SdkError;

const COPY_BUFFER_SIZE: usize = 128 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 3 * 1024 * 1024 * 1024 * 1024;
const MAX_ROOTFS_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_KERNEL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SSH_KEY_BYTES: u64 = 1024 * 1024;
const MANIFEST_MEMBER: &str = "manifest.json";

pub(crate) struct StagedSnapshot {
    pub manifest: SnapshotManifest,
    pub rootfs_path: PathBuf,
    pub kernel_path: PathBuf,
    pub private_key_path: PathBuf,
    pub public_key_path: PathBuf,
    _staging_guard: StagingDirectoryGuard,
}

pub(crate) fn read_archive(
    archive_path: &Path,
    staging_path: &Path,
    cancellation: CancellationToken,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<StagedSnapshot, SdkError> {
    const LEGACY_AGE_HEADER: &[u8] = b"age-encryption.org/v1\n";
    const ZSTANDARD_MAGIC: &[u8; 4] = &[0x28, 0xb5, 0x2f, 0xfd];

    let archive_metadata = fs::symlink_metadata(archive_path)
        .map_err(|error| SdkError::filesystem("inspect snapshot archive", archive_path, error))?;
    if archive_metadata.file_type().is_symlink() || !archive_metadata.is_file() {
        return Err(SdkError::InvalidRequest {
            field: "archive_path".to_owned(),
            reason: "must identify a regular file".to_owned(),
        });
    }
    if archive_metadata.len() == 0 || archive_metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(restore_archive_error(
            "validate snapshot archive",
            "archive size is outside the supported limit",
        ));
    }

    let mut source = File::open(archive_path)
        .map_err(|error| SdkError::filesystem("open snapshot archive", archive_path, error))?;
    let mut prefix = [0_u8; 22];
    let mut prefix_len = 0;
    while prefix_len < prefix.len() {
        let read = source.read(&mut prefix[prefix_len..]).map_err(|error| {
            SdkError::filesystem("read snapshot archive header", archive_path, error)
        })?;
        if read == 0 {
            break;
        }
        prefix_len += read;
    }
    if prefix_len >= LEGACY_AGE_HEADER.len()
        && &prefix[..LEGACY_AGE_HEADER.len()] == LEGACY_AGE_HEADER
    {
        return Err(restore_archive_error(
            "detect legacy snapshot format",
            "age-encrypted snapshots are unsupported; create a new unencrypted snapshot and try again",
        ));
    }
    if prefix_len < ZSTANDARD_MAGIC.len() || &prefix[..ZSTANDARD_MAGIC.len()] != ZSTANDARD_MAGIC {
        return Err(restore_archive_error(
            "validate snapshot format",
            "archive is not a supported unencrypted Zstandard snapshot",
        ));
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| SdkError::filesystem("rewind snapshot archive", archive_path, error))?;

    let staging_guard = StagingDirectoryGuard::create(staging_path)?;
    on_progress(0, 0);
    let decoder = zstd::stream::read::Decoder::new(source)
        .map_err(|error| restore_archive_error("decompress snapshot", &error.to_string()))?;
    let mut archive = tar::Archive::new(decoder);
    let mut seen = HashSet::new();
    let mut extracted = HashMap::<String, ExtractedPayload>::new();
    let mut manifest_bytes = None;

    {
        let entries = archive
            .entries()
            .map_err(|error| restore_archive_error("read TAR entries", &error.to_string()))?;
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err(SdkError::RestoreCancelled);
            }
            let entry = entry
                .map_err(|error| restore_archive_error("read TAR entry", &error.to_string()))?;
            let member = std::str::from_utf8(entry.path_bytes().as_ref())
                .map_err(|_| {
                    restore_archive_error("validate TAR member", "member name is not UTF-8")
                })?
                .to_owned();
            if !is_allowed_member(&member) {
                return Err(restore_archive_error(
                    "validate TAR member",
                    &format!("unexpected archive member {member:?}"),
                ));
            }
            if !seen.insert(member.clone()) {
                return Err(restore_archive_error(
                    "validate TAR member",
                    &format!("duplicate archive member {member:?}"),
                ));
            }
            if !entry.header().entry_type().is_file() {
                return Err(restore_archive_error(
                    "validate TAR member",
                    &format!("archive member {member:?} is not a regular file"),
                ));
            }
            let declared_size = entry.header().size().map_err(|error| {
                restore_archive_error("read TAR member size", &error.to_string())
            })?;
            let member_limit = member_limit(&member);
            if declared_size == 0 || declared_size > member_limit {
                return Err(restore_archive_error(
                    "validate TAR member size",
                    &format!("archive member {member:?} exceeds its supported size"),
                ));
            }
            if member == ROOTFS_MEMBER {
                on_progress(0, declared_size);
            }
            if member == MANIFEST_MEMBER {
                let mut bytes = Vec::with_capacity(declared_size as usize);
                entry
                    .take(declared_size)
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        restore_archive_error("read snapshot manifest", &error.to_string())
                    })?;
                if bytes.len() as u64 != declared_size {
                    return Err(restore_archive_error(
                        "read snapshot manifest",
                        "manifest ended before its declared size",
                    ));
                }
                manifest_bytes = Some(bytes);
                continue;
            }
            let output_path = staging_guard.member_path(&member)?;
            let mut output = create_staged_file(&output_path)?;
            let mut reader = entry.take(declared_size);
            let mut hasher = Sha256::new();
            let mut member_bytes = 0_u64;
            let mut buffer = [0_u8; COPY_BUFFER_SIZE];
            loop {
                if cancellation.is_cancelled() {
                    return Err(SdkError::RestoreCancelled);
                }
                let read = reader.read(&mut buffer).map_err(|error| {
                    restore_archive_error("extract snapshot payload", &error.to_string())
                })?;
                if read == 0 {
                    break;
                }
                output.write_all(&buffer[..read]).map_err(|error| {
                    SdkError::filesystem("write staged snapshot payload", &output_path, error)
                })?;
                hasher.update(&buffer[..read]);
                member_bytes = member_bytes.saturating_add(read as u64);
                if member == ROOTFS_MEMBER {
                    on_progress(
                        member_bytes.min(declared_size.saturating_sub(1)),
                        declared_size,
                    );
                }
            }
            if member_bytes != declared_size {
                return Err(restore_archive_error(
                    "extract snapshot payload",
                    &format!("archive member {member:?} ended before its declared size"),
                ));
            }
            output.sync_all().map_err(|error| {
                SdkError::filesystem("sync staged snapshot payload", &output_path, error)
            })?;
            extracted.insert(
                member,
                ExtractedPayload {
                    path: output_path,
                    size_bytes: member_bytes,
                    sha256: hex_digest(hasher.finalize()),
                },
            );
        }
    }

    if seen.len() != 5 {
        return Err(restore_archive_error(
            "validate TAR members",
            "archive does not contain the exact required member set",
        ));
    }
    let mut decoder = archive.into_inner();
    let mut tail = [0_u8; COPY_BUFFER_SIZE];
    loop {
        let read = decoder.read(&mut tail).map_err(|error| {
            restore_archive_error(
                "verify compressed archive",
                &format!("compressed stream is damaged or incomplete: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        if tail[..read].iter().any(|byte| *byte != 0) {
            return Err(restore_archive_error(
                "validate TAR end marker",
                "unexpected data follows the archive members",
            ));
        }
    }
    if cancellation.is_cancelled() {
        return Err(SdkError::RestoreCancelled);
    }
    let mut archive_reader = decoder.finish();
    let mut discard = [0_u8; COPY_BUFFER_SIZE];
    let trailing_bytes = archive_reader.read(&mut discard).map_err(|error| {
        restore_archive_error(
            "validate archive framing",
            &format!("could not read bytes after the compressed frame: {error}"),
        )
    })?;
    if trailing_bytes != 0 {
        return Err(restore_archive_error(
            "validate archive framing",
            "unexpected bytes follow the compressed TAR stream",
        ));
    }

    let manifest_bytes = manifest_bytes
        .ok_or_else(|| restore_archive_error("validate TAR members", "manifest.json is missing"))?;
    let manifest = parse_manifest(&manifest_bytes)?;
    verify_payloads(&manifest, &extracted)?;
    validate_staged_credentials(&manifest, &extracted)?;
    let path_for = |member: &str| -> Result<PathBuf, SdkError> {
        extracted
            .get(member)
            .map(|payload| payload.path.clone())
            .ok_or_else(|| restore_archive_error("stage snapshot", "required payload is missing"))
    };
    let rootfs_path = path_for(ROOTFS_MEMBER)?;
    let kernel_path = path_for(KERNEL_MEMBER)?;
    let private_key_path = path_for(PRIVATE_KEY_MEMBER)?;
    let public_key_path = path_for(PUBLIC_KEY_MEMBER)?;
    let rootfs_size_bytes = extracted
        .get(ROOTFS_MEMBER)
        .map(|payload| payload.size_bytes)
        .ok_or_else(|| restore_archive_error("stage snapshot", "root disk payload is missing"))?;
    set_mode(&kernel_path, 0o644)?;
    set_mode(&public_key_path, 0o644)?;
    on_progress(rootfs_size_bytes, rootfs_size_bytes);
    Ok(StagedSnapshot {
        manifest,
        rootfs_path,
        kernel_path,
        private_key_path,
        public_key_path,
        _staging_guard: staging_guard,
    })
}

fn verify_payloads(
    manifest: &SnapshotManifest,
    extracted: &HashMap<String, ExtractedPayload>,
) -> Result<(), SdkError> {
    for record in &manifest.payloads {
        let payload = extracted.get(&record.path).ok_or_else(|| {
            restore_archive_error("verify snapshot payload", "a required payload is missing")
        })?;
        if payload.size_bytes != record.size_bytes || payload.sha256 != record.sha256 {
            return Err(restore_archive_error(
                "verify snapshot payload",
                &format!("size or SHA-256 mismatch for {}", record.path),
            ));
        }
    }
    Ok(())
}

fn validate_staged_credentials(
    manifest: &SnapshotManifest,
    extracted: &HashMap<String, ExtractedPayload>,
) -> Result<(), SdkError> {
    let private = extracted
        .get(PRIVATE_KEY_MEMBER)
        .ok_or_else(|| restore_archive_error("verify SSH credentials", "private key is missing"))?;
    let public = extracted
        .get(PUBLIC_KEY_MEMBER)
        .ok_or_else(|| restore_archive_error("verify SSH credentials", "public key is missing"))?;
    let private_bytes = fs::read(&private.path)
        .map_err(|error| SdkError::filesystem("read staged private key", &private.path, error))?;
    let public_bytes = fs::read(&public.path)
        .map_err(|error| SdkError::filesystem("read staged public key", &public.path, error))?;
    validate_ed25519_key_pair(
        &private_bytes,
        &public_bytes,
        &manifest.ssh.public_key_fingerprint,
    )
}

fn create_staged_file(path: &Path) -> Result<File, SdkError> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| SdkError::filesystem("create staged snapshot payload", path, error))
}

fn set_mode(path: &Path, mode: u32) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| SdkError::filesystem("set staged payload permissions", path, error))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn is_allowed_member(member: &str) -> bool {
    matches!(
        member,
        MANIFEST_MEMBER | ROOTFS_MEMBER | KERNEL_MEMBER | PRIVATE_KEY_MEMBER | PUBLIC_KEY_MEMBER
    )
}

fn member_limit(member: &str) -> u64 {
    match member {
        MANIFEST_MEMBER => MAX_MANIFEST_BYTES,
        ROOTFS_MEMBER => MAX_ROOTFS_BYTES,
        KERNEL_MEMBER => MAX_KERNEL_BYTES,
        PRIVATE_KEY_MEMBER | PUBLIC_KEY_MEMBER => MAX_SSH_KEY_BYTES,
        _ => 0,
    }
}

fn restore_archive_error(operation: &'static str, reason: &str) -> SdkError {
    SdkError::RestoreArchive {
        operation,
        reason: reason.to_owned(),
    }
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct ExtractedPayload {
    path: PathBuf,
    size_bytes: u64,
    sha256: String,
}

struct StagingDirectoryGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingDirectoryGuard {
    fn create(path: &Path) -> Result<Self, SdkError> {
        fs::create_dir(path).map_err(|error| {
            SdkError::filesystem("create restore staging directory", path, error)
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
                let _ = fs::remove_dir(path);
                SdkError::filesystem("secure restore staging directory", path, error)
            })?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            armed: true,
        })
    }

    fn member_path(&self, member: &str) -> Result<PathBuf, SdkError> {
        let file_name = match member {
            ROOTFS_MEMBER => "rootfs.ext4",
            KERNEL_MEMBER => "vmlinux",
            PRIVATE_KEY_MEMBER => "id_ed25519",
            PUBLIC_KEY_MEMBER => "id_ed25519.pub",
            _ => {
                return Err(restore_archive_error(
                    "stage snapshot payload",
                    "archive member has no SDK-selected staging destination",
                ));
            }
        };
        Ok(self.path.join(file_name))
    }
}

impl Drop for StagingDirectoryGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::adapters::credentials::ed25519::Ed25519CredentialStore;
    use crate::error::SdkError;
    use crate::ports::credentials::CredentialStore;

    use super::read_archive;

    #[test]
    fn reads_plain_v3_archive_with_exact_fixed_members_and_real_progress() {
        let fixture = ArchiveFixture::new(false, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        let mut ticks = Vec::new();
        let restored = read_archive(
            &fixture.archive_path,
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |completed, total| ticks.push((completed, total)),
        )
        .expect("valid unencrypted archive should restore");
        assert_eq!(restored.manifest.vm.name, "demo");
        assert_eq!(
            fs::read(&restored.rootfs_path).expect("disk bytes"),
            b"root-disk"
        );
        assert_eq!(
            fs::read(&restored.kernel_path).expect("kernel bytes"),
            b"kernel"
        );
        let archive_size = fs::metadata(&fixture.archive_path)
            .expect("snapshot archive metadata")
            .len();
        let disk_size = b"root-disk".len() as u64;
        assert!(archive_size > disk_size);
        assert_eq!(ticks.first().copied(), Some((0, 0)));
        assert!(ticks.contains(&(0, disk_size)));
        assert!(
            ticks
                .windows(2)
                .any(|progress| progress[1].0 > progress[0].0)
        );
        assert_eq!(ticks.last().copied(), Some((disk_size, disk_size)));
    }

    #[test]
    fn reports_uncompressed_rootfs_progress_for_a_highly_compressed_archive() {
        let disk = vec![0_u8; 4 * 1024 * 1024];
        let fixture = ArchiveFixture::with_disk(&disk);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        let mut ticks = Vec::new();

        let restored = read_archive(
            &fixture.archive_path,
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |completed, total| ticks.push((completed, total)),
        )
        .expect("valid unencrypted archive should restore");

        let archive_size = fs::metadata(&fixture.archive_path)
            .expect("snapshot archive metadata")
            .len();
        let disk_size = disk.len() as u64;
        assert!(archive_size < disk_size / 10);
        assert_eq!(ticks.first().copied(), Some((0, 0)));
        assert!(ticks.contains(&(0, disk_size)));
        assert!(ticks.iter().any(|(completed, total)| {
            *total == disk_size && *completed > 0 && *completed < disk_size
        }));
        assert_eq!(ticks.last().copied(), Some((disk_size, disk_size)));
        assert_eq!(
            fs::metadata(&restored.rootfs_path)
                .expect("staged disk metadata")
                .len(),
            disk_size
        );
    }

    #[test]
    fn rejects_duplicate_members_and_cleans_staging_directory() {
        let fixture = ArchiveFixture::new(true, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        assert!(
            read_archive(
                &fixture.archive_path,
                &staging,
                tokio_util::sync::CancellationToken::new(),
                &mut |_, _| {}
            )
            .is_err()
        );
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_payload_digest_mismatch_and_cleans_staging_directory() {
        let fixture = ArchiveFixture::new(false, true);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        assert!(
            read_archive(
                &fixture.archive_path,
                &staging,
                tokio_util::sync::CancellationToken::new(),
                &mut |_, _| {}
            )
            .is_err()
        );
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_legacy_age_header_before_creating_staging_directory() {
        let directory = tempdir().expect("test directory");
        let archive_path = directory.path().join("legacy.tmvmsnap");
        fs::write(
            &archive_path,
            b"age-encryption.org/v1\n-> scrypt\nlegacy fixture",
        )
        .expect("legacy archive fixture should be written");
        let staging = directory.path().join("stage");

        let error = read_archive(
            &archive_path,
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |_, _| {},
        )
        .err()
        .expect("legacy age-encrypted archive must be rejected");

        assert!(matches!(error, SdkError::RestoreArchive { .. }));
        assert!(
            error
                .to_string()
                .contains("create a new unencrypted snapshot")
        );
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_unsupported_manifest_versions_and_removes_staging_directory() {
        let fixture = ArchiveFixture::with_version(99, false, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");

        let error = read_archive(
            &fixture.archive_path,
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |_, _| {},
        )
        .err()
        .expect("an unsupported archive version must be rejected");

        assert!(matches!(
            error,
            SdkError::UnsupportedSnapshotVersion { actual: 99, .. }
        ));
        assert!(!staging.exists());
    }

    struct ArchiveFixture {
        _directory: tempfile::TempDir,
        archive_path: std::path::PathBuf,
    }

    impl ArchiveFixture {
        fn new(duplicate_root: bool, corrupt_digest: bool) -> Self {
            Self::with_version(3, duplicate_root, corrupt_digest)
        }

        fn with_version(version: u32, duplicate_root: bool, corrupt_digest: bool) -> Self {
            Self::with_version_and_disk(version, duplicate_root, corrupt_digest, b"root-disk")
        }

        fn with_disk(disk: &[u8]) -> Self {
            Self::with_version_and_disk(3, false, false, disk)
        }

        fn with_version_and_disk(
            version: u32,
            duplicate_root: bool,
            corrupt_digest: bool,
            disk: &[u8],
        ) -> Self {
            let directory = tempdir().expect("fixture directory");
            let credentials = Ed25519CredentialStore
                .generate(directory.path())
                .expect("test SSH key generation");
            let private_key = fs::read(&credentials.private_key_path).expect("private key");
            let public_key = fs::read(&credentials.public_key_path).expect("public key");
            let kernel = b"kernel";
            let mut root_hash = digest(disk);
            if corrupt_digest {
                root_hash = "0".repeat(64);
            }
            let manifest = manifest_json(
                disk.len() as u64,
                root_hash,
                kernel.len() as u64,
                digest(kernel),
                private_key.len() as u64,
                digest(&private_key),
                public_key.len() as u64,
                digest(&public_key),
                &credentials.fingerprint,
                version,
            );
            let archive_path = directory.path().join("snapshot.tmvmsnap");
            write_plain_archive(
                &archive_path,
                &manifest,
                disk,
                kernel,
                &private_key,
                &public_key,
                duplicate_root,
            );
            Self {
                _directory: directory,
                archive_path,
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn manifest_json(
        disk_size: u64,
        disk_hash: String,
        kernel_size: u64,
        kernel_hash: String,
        private_size: u64,
        private_hash: String,
        public_size: u64,
        public_hash: String,
        fingerprint: &str,
        version: u32,
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "format":"taumaru.microvm.snapshot", "format_version":version, "created_at_unix_seconds":1,
            "vm":{"name":"demo","disk_size_bytes":disk_size,"memory_bytes":1024,"memory_effective_mib":1,"vcpu_count":1},
            "compatibility":{"host_os":"linux","guest_architecture":"x86_64","requires_kvm":true,"runtime":"firecracker"},
            "boot":{
                "distribution_id":"distro","distribution_name":"Linux","distribution_version":"1.0",
                "image_id":"image","image_sha256":"a".repeat(64),
                "kernel":{"id":"kernel","name":"kernel","display_name":"Kernel","version":"6.0","architecture":"x86_64","registry_path":"kernels/kernel/vmlinux","registry_url":"https://example.test/kernel","filename":"vmlinux","format":"elf","mime_type":"application/octet-stream","modified_at":"2026-01-01T00:00:00Z"},
                "root_device":"/dev/vda","kernel_args":["console=ttyS0"]
            },
            "network":{"address_policy":"regenerate_ipv4","expose_on_lan":false},
            "ssh":{"user":"root","port":22,"key_type":"ed25519","public_key_fingerprint":fingerprint},
            "consistency":{"kind":"disk_only_crash_consistent","includes_guest_memory":false,"includes_process_state":false},
            "payloads":[
                {"path":"payload/rootfs.ext4","size_bytes":disk_size,"sha256":disk_hash,"mode":384},
                {"path":"payload/kernel/vmlinux","size_bytes":kernel_size,"sha256":kernel_hash,"mode":420},
                {"path":"payload/ssh/id_ed25519","size_bytes":private_size,"sha256":private_hash,"mode":384},
                {"path":"payload/ssh/id_ed25519.pub","size_bytes":public_size,"sha256":public_hash,"mode":420}
            ]
        })).expect("serialize fixture manifest")
    }

    fn write_plain_archive(
        path: &Path,
        manifest: &[u8],
        disk: &[u8],
        kernel: &[u8],
        private_key: &[u8],
        public_key: &[u8],
        duplicate_root: bool,
    ) {
        let file = fs::File::create(path).expect("archive output");
        let mut zstd_writer = zstd::stream::write::Encoder::new(file, 3).expect("zstd writer");
        zstd_writer
            .include_checksum(true)
            .expect("Zstandard checksum should be enabled");
        let mut tar_writer = tar::Builder::new(zstd_writer);
        append(&mut tar_writer, "payload/rootfs.ext4", disk, 0o600);
        if duplicate_root {
            append(&mut tar_writer, "payload/rootfs.ext4", disk, 0o600);
        }
        append(&mut tar_writer, "payload/kernel/vmlinux", kernel, 0o644);
        append(
            &mut tar_writer,
            "payload/ssh/id_ed25519",
            private_key,
            0o600,
        );
        append(
            &mut tar_writer,
            "payload/ssh/id_ed25519.pub",
            public_key,
            0o644,
        );
        append(&mut tar_writer, "manifest.json", manifest, 0o600);
        tar_writer.finish().expect("finish TAR");
        let zstd_writer = tar_writer.into_inner().expect("finish TAR writer");
        zstd_writer.finish().expect("finish Zstandard");
    }

    fn append<W: Write>(writer: &mut tar::Builder<W>, path: &str, contents: &[u8], mode: u32) {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        writer
            .append_data(&mut header, path, contents)
            .expect("append TAR entry");
    }

    fn digest(contents: &[u8]) -> String {
        Sha256::digest(contents)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
