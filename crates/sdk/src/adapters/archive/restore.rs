use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use age::secrecy::SecretString;
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
const MAX_SCRYPT_WORK_FACTOR: u8 = 20;
const MANIFEST_MEMBER: &str = "manifest.json";

struct ArchiveProgressReader {
    source: File,
    completed_bytes: Arc<AtomicU64>,
}

impl Read for ArchiveProgressReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.source.read(buffer)?;
        if read > 0 {
            self.completed_bytes
                .fetch_add(read as u64, Ordering::Relaxed);
        }
        Ok(read)
    }
}

struct ArchiveProgress {
    completed_bytes: Arc<AtomicU64>,
    total_bytes: u64,
    last_reported_percent: u64,
}

impl ArchiveProgress {
    fn new(total_bytes: u64) -> Self {
        Self {
            completed_bytes: Arc::new(AtomicU64::new(0)),
            total_bytes,
            last_reported_percent: 0,
        }
    }

    fn wrap(&self, source: File) -> ArchiveProgressReader {
        ArchiveProgressReader {
            source,
            completed_bytes: Arc::clone(&self.completed_bytes),
        }
    }

    fn report(&mut self, on_progress: &mut dyn FnMut(u64, u64)) {
        if self.total_bytes == 0 {
            return;
        }
        let completed_bytes = self
            .completed_bytes
            .load(Ordering::Relaxed)
            .min(self.total_bytes);
        let percent = ((u128::from(completed_bytes) * 100) / u128::from(self.total_bytes)) as u64;
        let report_percent = percent.min(99);
        if report_percent > self.last_reported_percent {
            on_progress(completed_bytes.min(self.total_bytes - 1), self.total_bytes);
            self.last_reported_percent = report_percent;
        }
    }

    fn finish(&mut self, on_progress: &mut dyn FnMut(u64, u64)) {
        on_progress(self.total_bytes, self.total_bytes);
        self.last_reported_percent = 100;
    }
}

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
    password: &str,
    staging_path: &Path,
    cancellation: CancellationToken,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<StagedSnapshot, SdkError> {
    if password.is_empty() {
        return Err(SdkError::InvalidRequest {
            field: "password".to_owned(),
            reason: "must not be empty".to_owned(),
        });
    }
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
            "encrypted archive size is outside the supported limit",
        ));
    }
    let staging_guard = StagingDirectoryGuard::create(staging_path)?;
    let source = File::open(archive_path)
        .map_err(|error| SdkError::filesystem("open snapshot archive", archive_path, error))?;
    let mut progress = ArchiveProgress::new(archive_metadata.len());
    on_progress(0, archive_metadata.len());
    let source = progress.wrap(source);
    let decryptor = age::Decryptor::new(BufReader::new(source)).map_err(|error| {
        restore_archive_error(
            "read age header",
            &format!("invalid encrypted archive: {error}"),
        )
    })?;
    if !decryptor.is_scrypt() {
        return Err(restore_archive_error(
            "validate encryption",
            "archive does not use password-based age encryption",
        ));
    }
    let mut identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    identity.set_max_work_factor(MAX_SCRYPT_WORK_FACTOR);
    let decrypted = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|error| {
            restore_archive_error(
                "decrypt snapshot",
                &format!("password is incorrect or encrypted data is damaged: {error}"),
            )
        })?;
    progress.report(on_progress);
    let decoder = zstd::stream::read::Decoder::new(decrypted)
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
                progress.report(on_progress);
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
                progress.report(on_progress);
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
                "authenticate encrypted archive",
                &format!("compressed stream is damaged or incomplete: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        progress.report(on_progress);
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
    let mut encrypted_reader = decoder.finish();
    let mut discard = [0_u8; COPY_BUFFER_SIZE];
    let trailing_bytes = encrypted_reader.read(&mut discard).map_err(|error| {
        restore_archive_error(
            "authenticate encrypted archive",
            &format!("encrypted stream authentication failed: {error}"),
        )
    })?;
    progress.report(on_progress);
    if trailing_bytes != 0 {
        return Err(restore_archive_error(
            "validate encrypted archive framing",
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
    set_mode(&kernel_path, 0o644)?;
    set_mode(&public_key_path, 0o644)?;
    progress.finish(on_progress);
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

    use age::secrecy::SecretString;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::adapters::credentials::ed25519::Ed25519CredentialStore;
    use crate::error::SdkError;
    use crate::ports::credentials::CredentialStore;

    use super::read_archive;

    const PASSWORD: &str = "restore-test-password";

    #[test]
    fn reads_authenticated_v2_archive_with_exact_fixed_members_and_real_progress() {
        let fixture = ArchiveFixture::new(false, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        let mut ticks = Vec::new();
        let restored = read_archive(
            &fixture.archive_path,
            PASSWORD,
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |completed, total| ticks.push((completed, total)),
        )
        .expect("valid encrypted archive should restore");
        assert_eq!(restored.manifest.vm.name, "demo");
        assert_eq!(
            fs::read(&restored.rootfs_path).expect("disk bytes"),
            b"root-disk"
        );
        assert_eq!(
            fs::read(&restored.kernel_path).expect("kernel bytes"),
            b"kernel"
        );
        let total = fs::metadata(&fixture.archive_path)
            .expect("encrypted archive metadata")
            .len();
        assert_eq!(ticks.first().copied(), Some((0, total)));
        assert!(
            ticks
                .windows(2)
                .any(|progress| progress[1].0 > progress[0].0)
        );
        assert_eq!(ticks.last().copied(), Some((total, total)));
    }

    #[test]
    fn rejects_duplicate_members_and_cleans_staging_directory() {
        let fixture = ArchiveFixture::new(true, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");
        assert!(
            read_archive(
                &fixture.archive_path,
                PASSWORD,
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
                PASSWORD,
                &staging,
                tokio_util::sync::CancellationToken::new(),
                &mut |_, _| {}
            )
            .is_err()
        );
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_a_wrong_password_and_removes_staging_directory() {
        let fixture = ArchiveFixture::new(false, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");

        let error = read_archive(
            &fixture.archive_path,
            "incorrect-password",
            &staging,
            tokio_util::sync::CancellationToken::new(),
            &mut |_, _| {},
        )
        .err()
        .expect("a wrong password must be rejected");

        assert!(matches!(error, SdkError::RestoreArchive { .. }));
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_unsupported_manifest_versions_and_removes_staging_directory() {
        let fixture = ArchiveFixture::with_version(99, false, false);
        let directory = tempdir().expect("test directory");
        let staging = directory.path().join("stage");

        let error = read_archive(
            &fixture.archive_path,
            PASSWORD,
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
            Self::with_version(2, duplicate_root, corrupt_digest)
        }

        fn with_version(version: u32, duplicate_root: bool, corrupt_digest: bool) -> Self {
            let directory = tempdir().expect("fixture directory");
            let credentials = Ed25519CredentialStore
                .generate(directory.path())
                .expect("test SSH key generation");
            let private_key = fs::read(&credentials.private_key_path).expect("private key");
            let public_key = fs::read(&credentials.public_key_path).expect("public key");
            let disk = b"root-disk";
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
            write_encrypted_archive(
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

    fn write_encrypted_archive(
        path: &Path,
        manifest: &[u8],
        disk: &[u8],
        kernel: &[u8],
        private_key: &[u8],
        public_key: &[u8],
        duplicate_root: bool,
    ) {
        let file = fs::File::create(path).expect("archive output");
        let encryptor =
            age::Encryptor::with_user_passphrase(SecretString::from(PASSWORD.to_owned()));
        let age_writer = encryptor.wrap_output(file).expect("age writer");
        let zstd_writer = zstd::stream::write::Encoder::new(age_writer, 1).expect("zstd writer");
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
        let age_writer = zstd_writer.finish().expect("finish Zstandard");
        age_writer.finish().expect("finish age");
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
