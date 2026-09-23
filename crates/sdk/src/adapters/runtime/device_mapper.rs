use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::domain::config::validate_vm_name;
use crate::domain::snapshot::{SnapshotProgress, SnapshotProgressStage};
use crate::error::SdkError;
use crate::ports::runtime_disk::RuntimeDiskController;

const DEVICE_MAPPER_NAME_MAX: usize = 127;
const SECTOR_SIZE_BYTES: u64 = 512;
const SNAPSHOT_CHUNK_SECTORS: u64 = 64;
const COW_ALLOCATION_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
const RESOURCE_WAIT: Duration = Duration::from_secs(5);
const RESOURCE_POLL: Duration = Duration::from_millis(50);
const LOCK_POLL: Duration = Duration::from_millis(25);

/// Linux loop and Device Mapper implementation for the SDK runtime disk boundary.
#[derive(Clone)]
pub(crate) struct DeviceMapperRuntime {
    commands: Arc<dyn HostCommandRunner>,
    mapper_directory: PathBuf,
    verify_mapper_node: bool,
}

impl Default for DeviceMapperRuntime {
    fn default() -> Self {
        Self {
            commands: Arc::new(SystemCommandRunner),
            mapper_directory: PathBuf::from("/dev/mapper"),
            verify_mapper_node: true,
        }
    }
}

/// Holds the per-VM inter-process lock until the lifecycle operation completes.
pub(crate) struct VmLifecycleLock {
    _file: File,
}

/// Acquires the persistent per-VM lock file without blocking a Tokio worker.
pub(crate) async fn acquire_lifecycle_lock(
    home: &Path,
    vm_name: &str,
) -> Result<VmLifecycleLock, SdkError> {
    let identity = MappingIdentity::new(home, vm_name)?;
    let lock_directory = identity.canonical_home.join("runtime").join("locks");
    fs::create_dir_all(&lock_directory).map_err(|error| {
        SdkError::filesystem("create runtime lock directory", &lock_directory, error)
    })?;
    let lock_path = lock_directory.join(format!("{}.lock", identity.digest));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| SdkError::filesystem("open VM runtime lock", &lock_path, error))?;

    loop {
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => return Ok(VmLifecycleLock { _file: file }),
            Err(fs4::TryLockError::WouldBlock) => tokio::time::sleep(LOCK_POLL).await,
            Err(fs4::TryLockError::Error(error)) => {
                return Err(SdkError::filesystem("lock VM runtime", &lock_path, error));
            }
        }
    }
}

impl RuntimeDiskController for DeviceMapperRuntime {
    fn ensure_mapping(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<PathBuf, SdkError> {
        let identity = MappingIdentity::new(sdk_home, vm_name)?;
        let rootfs = RootfsIdentity::new(rootfs_path, vm_name)?;
        let mapper = self.find_mapper(&identity)?;
        if let Some(mapper) = mapper {
            self.verify_mapping(&identity, &rootfs, &mapper)?;
            if self.verify_mapper_node
                && let Err(primary) = self.wait_for_mapper_node(&identity)
            {
                let cleanup = self.release_mapping(sdk_home, vm_name, rootfs_path).err();
                return Err(with_cleanup(primary, cleanup));
            }
            return Ok(self.mapper_path(&identity));
        }

        let mut loop_device = self.find_loop(&rootfs)?;
        let created_loop = if loop_device.is_none() {
            let output = self.run_checked(
                "losetup",
                vec![
                    OsString::from("--find"),
                    OsString::from("--show"),
                    OsString::from("--nooverlap"),
                    rootfs.canonical_path.as_os_str().to_owned(),
                ],
            )?;
            let name = output_text(&output.stdout).trim().to_owned();
            if name.is_empty() {
                return Err(host_command_error(
                    "losetup",
                    "loop allocation returned an empty device path",
                ));
            }
            loop_device = self.find_loop(&rootfs)?;
            let Some(device) = loop_device.as_ref() else {
                return Err(host_command_error(
                    "losetup",
                    "the allocated loop device could not be verified against the root disk",
                ));
            };
            if device.name != name {
                return Err(storage_conflict(
                    vm_name,
                    rootfs_path,
                    "losetup selected a different loop association than the verified root disk",
                ));
            }
            true
        } else {
            false
        };
        let loop_device = loop_device.ok_or_else(|| {
            host_command_error("losetup", "no loop device is associated with the root disk")
        })?;
        let sectors = rootfs.size_bytes / SECTOR_SIZE_BYTES;
        if sectors == 0 || rootfs.size_bytes % SECTOR_SIZE_BYTES != 0 {
            if created_loop {
                let _ = self.detach_verified_loop(&rootfs, &loop_device);
            }
            return Err(SdkError::RuntimeIncompatible {
                component: "loop-device-mapping".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: "the root disk size is not a nonzero multiple of 512 bytes".to_owned(),
            });
        }

        let table = format!("0 {sectors} snapshot-origin {}", loop_device.major_minor);
        let create_result = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("create"),
                OsString::from(&identity.mapper_name),
                OsString::from("--uuid"),
                OsString::from(&identity.mapper_uuid),
                OsString::from("--table"),
                OsString::from(&table),
            ],
        );
        if let Err(primary) = create_result {
            let cleanup = if created_loop {
                self.detach_verified_loop(&rootfs, &loop_device).err()
            } else {
                None
            };
            return Err(with_cleanup(primary, cleanup));
        }

        let created_mapper = self.find_mapper(&identity)?;
        let verification = match created_mapper {
            Some(mapper) => self.verify_mapping(&identity, &rootfs, &mapper),
            None => Err(host_command_error(
                "dmsetup",
                "the created mapping did not appear in Device Mapper state",
            )),
        };
        if let Err(primary) = verification {
            let cleanup = self
                .remove_created_mapping(&identity, &rootfs, &loop_device)
                .err()
                .or_else(|| {
                    if created_loop {
                        self.detach_verified_loop(&rootfs, &loop_device).err()
                    } else {
                        None
                    }
                });
            return Err(with_cleanup(primary, cleanup));
        }

        if self.verify_mapper_node
            && let Err(primary) = self.wait_for_mapper_node(&identity)
        {
            let cleanup = self.release_mapping(sdk_home, vm_name, rootfs_path).err();
            return Err(with_cleanup(primary, cleanup));
        }
        Ok(self.mapper_path(&identity))
    }

    fn release_mapping(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError> {
        let identity = MappingIdentity::new(sdk_home, vm_name)?;
        let rootfs = RootfsIdentity::new(rootfs_path, vm_name)?;
        let mapper = self.find_mapper(&identity)?;
        let loop_device = if let Some(mapper) = mapper.as_ref() {
            if mapper.open_count != 0 {
                return Err(temporary_runtime(format!(
                    "the owned mapper still has {} open references",
                    mapper.open_count
                )));
            }
            let table = self.verify_mapping(&identity, &rootfs, mapper)?;
            self.run_checked(
                "dmsetup",
                vec![
                    OsString::from("remove"),
                    OsString::from(&identity.mapper_name),
                ],
            )
            .map_err(|error| map_busy_error(error, "remove the owned mapper"))?;
            self.wait_until_mapper_gone(&identity)?;
            self.loop_for_dependency(&rootfs, &table.dependency)?
        } else {
            match self.find_loop(&rootfs)? {
                Some(device) => device,
                None => return Ok(()),
            }
        };

        self.detach_verified_loop(&rootfs, &loop_device)?;
        Ok(())
    }

    fn create_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
        on_progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<PathBuf, SdkError> {
        let identity = SnapshotIdentity::new(sdk_home, vm_name)?;
        let rootfs = RootfsIdentity::new(rootfs_path, vm_name)?;
        let origin = self.find_mapper(&identity.origin)?.ok_or_else(|| {
            snapshot_view_error(vm_name, "the running VM has no active origin mapping")
        })?;
        self.verify_mapping(&identity.origin, &rootfs, &origin)?;
        self.ensure_origin_resumed(&identity.origin)?;
        self.remove_snapshot_view(sdk_home, vm_name, rootfs_path)?;

        let cow_directory = identity.cow_path.parent().ok_or_else(|| {
            snapshot_view_error(vm_name, "the COW backing path has no parent directory")
        })?;
        fs::create_dir_all(cow_directory).map_err(|error| {
            SdkError::filesystem("create snapshot COW directory", cow_directory, error)
        })?;
        let directory_metadata = fs::symlink_metadata(cow_directory).map_err(|error| {
            SdkError::filesystem("inspect snapshot COW directory", cow_directory, error)
        })?;
        if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
            return Err(snapshot_view_error(
                vm_name,
                "the COW directory is not a real directory",
            ));
        }
        let canonical_cow_directory = fs::canonicalize(cow_directory).map_err(|error| {
            SdkError::filesystem("canonicalize snapshot COW directory", cow_directory, error)
        })?;
        if !canonical_cow_directory.starts_with(&identity.origin.canonical_home) {
            return Err(snapshot_view_error(
                vm_name,
                "the COW directory resolves outside the SDK home",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(cow_directory, fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    SdkError::filesystem("protect snapshot COW directory", cow_directory, error)
                },
            )?;
        }
        let cow_size =
            snapshot_cow_size(rootfs.size_bytes).ok_or_else(|| SdkError::SnapshotCapacity {
                vm_name: vm_name.to_owned(),
                reason: "the required COW size exceeds the host file-size limit".to_owned(),
            })?;
        let cow_file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(&identity.cow_path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    SdkError::SnapshotCapacity {
                        vm_name: vm_name.to_owned(),
                        reason: "a deterministic COW backing file already exists and could not be reconciled safely".to_owned(),
                    }
                } else {
                    SdkError::filesystem("create snapshot COW backing file", &identity.cow_path, error)
                }
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) = cow_file.set_permissions(fs::Permissions::from_mode(0o600)) {
                drop(cow_file);
                let primary = SdkError::filesystem(
                    "protect snapshot COW backing file",
                    &identity.cow_path,
                    error,
                );
                let cleanup = fs::remove_file(&identity.cow_path).err().map(|source| {
                    SdkError::filesystem(
                        "remove unprotected COW backing file",
                        &identity.cow_path,
                        source,
                    )
                });
                return Err(with_cleanup(primary, cleanup));
            }
        }
        drop(cow_file);
        let mut allocated_bytes = 0_u64;
        on_progress(SnapshotProgress {
            stage: SnapshotProgressStage::AllocatingCowStore,
            completed_bytes: allocated_bytes,
            total_bytes: cow_size,
        });
        while allocated_bytes < cow_size {
            let chunk_bytes = (cow_size - allocated_bytes).min(COW_ALLOCATION_CHUNK_BYTES);
            let allocation = self.run_checked(
                "fallocate",
                vec![
                    OsString::from("--offset"),
                    OsString::from(allocated_bytes.to_string()),
                    OsString::from("--length"),
                    OsString::from(chunk_bytes.to_string()),
                    identity.cow_path.as_os_str().to_owned(),
                ],
            );
            if let Err(error) = allocation {
                let primary = SdkError::SnapshotCapacity {
                    vm_name: vm_name.to_owned(),
                    reason: error.to_string(),
                };
                let cleanup = fs::remove_file(&identity.cow_path).err().map(|source| {
                    SdkError::filesystem(
                        "remove incomplete snapshot COW backing file",
                        &identity.cow_path,
                        source,
                    )
                });
                return Err(with_cleanup(primary, cleanup));
            }
            allocated_bytes = allocated_bytes.saturating_add(chunk_bytes);
            on_progress(SnapshotProgress {
                stage: SnapshotProgressStage::AllocatingCowStore,
                completed_bytes: allocated_bytes,
                total_bytes: cow_size,
            });
        }
        on_progress(SnapshotProgress {
            stage: SnapshotProgressStage::InstallingDiskView,
            completed_bytes: 0,
            total_bytes: 0,
        });
        let cow = match RootfsIdentity::new(&identity.cow_path, vm_name) {
            Ok(cow) => cow,
            Err(primary) => {
                let cleanup = fs::remove_file(&identity.cow_path).err().map(|source| {
                    SdkError::filesystem(
                        "remove invalid snapshot COW backing file",
                        &identity.cow_path,
                        source,
                    )
                });
                return Err(with_cleanup(primary, cleanup));
            }
        };
        let loop_result = self.run_checked(
            "losetup",
            vec![
                OsString::from("--find"),
                OsString::from("--show"),
                OsString::from("--nooverlap"),
                cow.canonical_path.as_os_str().to_owned(),
            ],
        );
        let loop_output = match loop_result {
            Ok(output) => output_text(&output.stdout).trim().to_owned(),
            Err(primary) => {
                let cleanup = self.cleanup_snapshot_backing(&identity, &cow).err();
                return Err(with_cleanup(primary, cleanup));
            }
        };
        let cow_loop = match self.find_loop(&cow) {
            Ok(Some(device))
                if !device.read_only && device.offset_bytes == 0 && device.name == loop_output =>
            {
                device
            }
            Ok(_) => {
                let primary = snapshot_view_error(
                    vm_name,
                    "the COW loop device could not be verified against its backing file",
                );
                let cleanup = self.cleanup_snapshot_backing(&identity, &cow).err();
                return Err(with_cleanup(primary, cleanup));
            }
            Err(primary) => {
                let cleanup = self.cleanup_snapshot_backing(&identity, &cow).err();
                return Err(with_cleanup(primary, cleanup));
            }
        };

        let suspended = match self.suspend_origin(&identity.origin) {
            Ok(()) => true,
            Err(primary) => {
                let cleanup = self
                    .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                    .err();
                return Err(with_cleanup(primary, cleanup));
            }
        };
        let origin_path = self.mapper_path(&identity.origin);
        let table = format!(
            "0 {} snapshot {} {} PO {}",
            rootfs.size_bytes / SECTOR_SIZE_BYTES,
            origin_path.display(),
            cow_loop.name,
            SNAPSHOT_CHUNK_SECTORS
        );
        let create = self.create_snapshot_mapping(&identity, &table);
        let resume = if suspended {
            self.resume_origin(&identity.origin).err()
        } else {
            None
        };
        if let Err(primary) = create {
            let cleanup = self
                .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                .err()
                .or(resume);
            return Err(with_cleanup(primary, cleanup));
        }
        if let Some(cleanup) = resume {
            let removal = self
                .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                .err();
            return Err(with_cleanup(cleanup, removal));
        }

        let mapper = match self.find_snapshot_mapper(&identity) {
            Ok(Some(mapper)) => mapper,
            Ok(None) => {
                let primary = snapshot_view_error(
                    vm_name,
                    "the new snapshot mapping did not appear in Device Mapper state",
                );
                let cleanup = self
                    .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                    .err();
                return Err(with_cleanup(primary, cleanup));
            }
            Err(primary) => {
                let cleanup = self
                    .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                    .err();
                return Err(with_cleanup(primary, cleanup));
            }
        };
        if let Err(primary) =
            self.verify_snapshot_mapping(&identity, &rootfs, &cow, &cow_loop, &mapper)
        {
            let cleanup = self
                .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                .err();
            return Err(with_cleanup(primary, cleanup));
        }
        if let Err(primary) = self.check_snapshot_status(&identity) {
            let cleanup = self
                .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                .err();
            return Err(with_cleanup(primary, cleanup));
        }
        if self.verify_mapper_node
            && let Err(primary) = self.wait_for_snapshot_node(&identity)
        {
            let cleanup = self
                .remove_snapshot_view(sdk_home, vm_name, rootfs_path)
                .err();
            return Err(with_cleanup(primary, cleanup));
        }
        Ok(self.snapshot_mapper_path(&identity))
    }

    fn check_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError> {
        let identity = SnapshotIdentity::new(sdk_home, vm_name)?;
        let rootfs = RootfsIdentity::new(rootfs_path, vm_name)?;
        let cow = RootfsIdentity::new(&identity.cow_path, vm_name)?;
        let mapper = self.find_snapshot_mapper(&identity)?.ok_or_else(|| {
            snapshot_view_error(vm_name, "the snapshot mapping disappeared during capture")
        })?;
        let loops = self.find_loops_for_rootfs(&cow)?;
        let [cow_loop] = loops.as_slice() else {
            return Err(snapshot_view_error(
                vm_name,
                "the COW loop association is missing or ambiguous",
            ));
        };
        self.verify_snapshot_mapping(&identity, &rootfs, &cow, cow_loop, &mapper)?;
        self.check_snapshot_status(&identity)
    }

    fn remove_snapshot_view(
        &self,
        sdk_home: &Path,
        vm_name: &str,
        rootfs_path: &Path,
    ) -> Result<(), SdkError> {
        let identity = SnapshotIdentity::new(sdk_home, vm_name)?;
        let rootfs = RootfsIdentity::new(rootfs_path, vm_name)?;
        let cow_exists = path_exists(&identity.cow_path)?;
        let cow = if cow_exists {
            Some(RootfsIdentity::new(&identity.cow_path, vm_name)?)
        } else {
            None
        };
        let mapper = self.find_snapshot_mapper(&identity)?;
        let origin_before_cleanup = self.find_mapper(&identity.origin)?;
        if let Some(origin) = origin_before_cleanup.as_ref() {
            self.verify_mapping(&identity.origin, &rootfs, origin)?;
            self.ensure_origin_resumed(&identity.origin)?;
        }
        if let Some(mapper) = mapper {
            let origin = self.find_mapper(&identity.origin)?.ok_or_else(|| {
                snapshot_view_error(
                    vm_name,
                    "the origin mapping is missing while the snapshot mapping is active",
                )
            })?;
            self.verify_mapping(&identity.origin, &rootfs, &origin)?;
            let cow = cow.as_ref().ok_or_else(|| {
                snapshot_view_error(
                    vm_name,
                    "the active snapshot mapping has no COW backing file",
                )
            })?;
            let loops = self.find_loops_for_rootfs(cow)?;
            let [cow_loop] = loops.as_slice() else {
                return Err(snapshot_view_error(
                    vm_name,
                    "the COW loop association is missing or ambiguous",
                ));
            };
            self.verify_snapshot_mapping(&identity, &rootfs, cow, cow_loop, &mapper)?;
            if mapper.open_count != 0 {
                return Err(temporary_runtime(
                    "the snapshot mapping still has open references",
                ));
            }
            self.suspend_origin(&identity.origin)?;
            let remove = self.run_checked(
                "dmsetup",
                vec![
                    OsString::from("remove"),
                    OsString::from(&identity.mapper_name),
                ],
            );
            let remove_result = remove.and_then(|_| self.wait_until_snapshot_gone(&identity));
            let resume_result = self.resume_origin(&identity.origin);
            if let Err(primary) = remove_result {
                return Err(with_cleanup(primary, resume_result.err()));
            }
            resume_result?;
        }
        if let Some(cow) = cow {
            self.cleanup_snapshot_backing(&identity, &cow)?;
        } else if self
            .find_loop_for_snapshot_path(&identity.cow_path, vm_name)?
            .is_some()
        {
            return Err(snapshot_view_error(
                vm_name,
                "a COW loop exists but its backing file is missing",
            ));
        }
        Ok(())
    }
}

impl DeviceMapperRuntime {
    fn run_checked(&self, program: &str, arguments: Vec<OsString>) -> Result<Output, SdkError> {
        let output = self.commands.run(program, &arguments)?;
        if output.status.success() {
            return Ok(output);
        }
        Err(host_command_error(
            program,
            &format!(
                "command exited with {}; {}",
                output.status,
                output_text(&output.stderr).trim()
            ),
        ))
    }

    fn find_mapper(&self, identity: &MappingIdentity) -> Result<Option<MapperInfo>, SdkError> {
        self.find_named_mapper(
            &identity.mapper_name,
            &identity.mapper_uuid,
            &identity.vm_name,
            &identity.canonical_home,
        )
    }

    fn find_snapshot_mapper(
        &self,
        identity: &SnapshotIdentity,
    ) -> Result<Option<MapperInfo>, SdkError> {
        self.find_named_mapper(
            &identity.mapper_name,
            &identity.mapper_uuid,
            &identity.origin.vm_name,
            &identity.origin.canonical_home,
        )
    }

    fn find_named_mapper(
        &self,
        name: &str,
        uuid: &str,
        vm_name: &str,
        home: &Path,
    ) -> Result<Option<MapperInfo>, SdkError> {
        let output = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("info"),
                OsString::from("--columns"),
                OsString::from("--noheadings"),
                OsString::from("--separator"),
                OsString::from("\t"),
                OsString::from("--options"),
                OsString::from("name,uuid,open"),
            ],
        )?;
        let mappers = parse_mapper_list(&output.stdout)?;
        let named = mappers.iter().find(|mapper| mapper.name == name);
        let uuid_owner = mappers.iter().find(|mapper| mapper.uuid == uuid);
        match (named, uuid_owner) {
            (Some(mapper), Some(owner)) if mapper.name == owner.name && mapper.uuid == uuid => {
                Ok(Some(mapper.clone()))
            }
            (None, None) => Ok(None),
            (Some(_), _) | (_, Some(_)) => Err(storage_conflict(
                vm_name,
                home,
                "the deterministic mapper name or UUID is already used by a different mapping",
            )),
        }
    }

    fn create_snapshot_mapping(
        &self,
        identity: &SnapshotIdentity,
        table: &str,
    ) -> Result<(), SdkError> {
        let run_create = |table: &str| {
            self.run_checked(
                "dmsetup",
                vec![
                    OsString::from("create"),
                    OsString::from(&identity.mapper_name),
                    OsString::from("--uuid"),
                    OsString::from(&identity.mapper_uuid),
                    OsString::from("--table"),
                    OsString::from(table),
                    OsString::from("--readonly"),
                    OsString::from("--noudevrules"),
                    OsString::from("--noudevsync"),
                    OsString::from("--addnodeoncreate"),
                ],
            )
        };
        match run_create(table) {
            Ok(_) => Ok(()),
            Err(primary) if table.contains(" PO ") && supports_po_fallback(&primary) => {
                let fallback = table.replace(" PO ", " P ");
                run_create(&fallback).map(|_| ())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_snapshot_mapping(
        &self,
        identity: &SnapshotIdentity,
        rootfs: &RootfsIdentity,
        cow: &RootfsIdentity,
        cow_loop: &LoopInfo,
        mapper: &MapperInfo,
    ) -> Result<SnapshotDmTable, SdkError> {
        if mapper.name != identity.mapper_name || mapper.uuid != identity.mapper_uuid {
            return Err(snapshot_view_error(
                &identity.origin.vm_name,
                "Device Mapper snapshot ownership does not match the VM identity",
            ));
        }
        let output = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("table"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        let table = parse_snapshot_dm_table(&output.stdout).ok_or_else(|| {
            snapshot_view_error(
                &identity.origin.vm_name,
                "the owned mapper is not one snapshot target",
            )
        })?;
        let expected_sectors = rootfs.size_bytes / SECTOR_SIZE_BYTES;
        let origin_path = self.mapper_path(&identity.origin);
        let expected_origin = block_device_major_minor(&origin_path).map_err(|error| {
            snapshot_view_error(
                &identity.origin.vm_name,
                &format!("could not inspect origin mapper: {error}"),
            )
        })?;
        if table.start_sector != 0
            || table.sector_count != expected_sectors
            || table.target != "snapshot"
            || table.origin_dependency != expected_origin
            || table.cow_dependency != cow_loop.major_minor
            || table.chunk_sectors != SNAPSHOT_CHUNK_SECTORS
            || !matches!(table.mode.as_str(), "P" | "PO")
            || cow.canonical_path != identity.cow_path
            || cow_loop.read_only
            || cow_loop.offset_bytes != 0
            || (cow_loop.size_limit_bytes != 0 && cow_loop.size_limit_bytes < cow.size_bytes)
        {
            return Err(snapshot_view_error(
                &identity.origin.vm_name,
                "the snapshot table or COW loop does not match the verified VM disks",
            ));
        }
        Ok(table)
    }

    fn check_snapshot_status(&self, identity: &SnapshotIdentity) -> Result<(), SdkError> {
        let output = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("status"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        let status = output_text(&output.stdout);
        if status.trim().is_empty() {
            return Err(snapshot_view_error(
                &identity.origin.vm_name,
                "Device Mapper returned an empty snapshot status",
            ));
        }
        let lowered = status.to_ascii_lowercase();
        if lowered.contains("overflow") || lowered.contains("invalid") {
            return Err(snapshot_view_error(
                &identity.origin.vm_name,
                if lowered.contains("overflow") {
                    "the COW store overflowed"
                } else {
                    "the kernel marked the snapshot invalid"
                },
            ));
        }
        Ok(())
    }

    fn suspend_origin(&self, identity: &MappingIdentity) -> Result<(), SdkError> {
        self.run_checked(
            "dmsetup",
            vec![
                OsString::from("suspend"),
                OsString::from(&identity.mapper_name),
            ],
        )
        .map(|_| ())
    }

    fn resume_origin(&self, identity: &MappingIdentity) -> Result<(), SdkError> {
        self.run_checked(
            "dmsetup",
            vec![
                OsString::from("resume"),
                OsString::from(&identity.mapper_name),
            ],
        )
        .map(|_| ())
    }

    fn ensure_origin_resumed(&self, identity: &MappingIdentity) -> Result<(), SdkError> {
        let output = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("info"),
                OsString::from("--columns"),
                OsString::from("--noheadings"),
                OsString::from("--options"),
                OsString::from("suspended"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        let state = output_text(&output.stdout).trim().to_ascii_lowercase();
        match state.as_str() {
            "suspended" | "yes" | "true" | "1" => self.resume_origin(identity),
            "active" | "no" | "false" | "0" => Ok(()),
            _ => Err(snapshot_view_error(
                &identity.vm_name,
                "could not determine whether the origin mapping is suspended",
            )),
        }
    }

    fn cleanup_snapshot_backing(
        &self,
        identity: &SnapshotIdentity,
        cow: &RootfsIdentity,
    ) -> Result<(), SdkError> {
        if let Some(loop_device) = self.find_loop(cow)? {
            self.detach_verified_loop(cow, &loop_device)?;
        }
        if path_exists(&identity.cow_path)? {
            fs::remove_file(&identity.cow_path).map_err(|error| {
                SdkError::filesystem(
                    "remove snapshot COW backing file",
                    &identity.cow_path,
                    error,
                )
            })?;
        }
        Ok(())
    }

    fn find_loop_for_snapshot_path(
        &self,
        path: &Path,
        vm_name: &str,
    ) -> Result<Option<LoopInfo>, SdkError> {
        if path_exists(path)? {
            return self.find_loop(&RootfsIdentity::new(path, "snapshot")?);
        }
        let output = self.run_checked(
            "losetup",
            vec![
                OsString::from("--json"),
                OsString::from("--list"),
                OsString::from("--output"),
                OsString::from("NAME,BACK-FILE,MAJ:MIN,RO,SIZELIMIT,OFFSET"),
            ],
        )?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            host_command_error("losetup", &format!("invalid JSON output: {error}"))
        })?;
        let devices = value
            .get("loopdevices")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                host_command_error("losetup", "loop listing omitted the loopdevices array")
            })?;
        if devices.iter().any(|device| {
            device
                .get("back-file")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|backing| {
                    backing == path.to_string_lossy()
                        || backing == format!("{} (deleted)", path.display())
                })
        }) {
            return Err(snapshot_view_error(
                vm_name,
                "an orphan COW loop refers to a missing backing file; it was preserved",
            ));
        }
        Ok(None)
    }

    fn wait_until_snapshot_gone(&self, identity: &SnapshotIdentity) -> Result<(), SdkError> {
        let deadline = Instant::now() + RESOURCE_WAIT;
        loop {
            if self.find_snapshot_mapper(identity)?.is_none() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(temporary_runtime(
                    "the Device Mapper snapshot did not disappear after removal",
                ));
            }
            std::thread::sleep(RESOURCE_POLL);
        }
    }

    fn wait_for_snapshot_node(&self, identity: &SnapshotIdentity) -> Result<(), SdkError> {
        self.run_checked(
            "dmsetup",
            vec![
                OsString::from("mknodes"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        let path = self.snapshot_mapper_path(identity);
        let deadline = Instant::now() + RESOURCE_WAIT;
        loop {
            if snapshot_mapper_node_ready(&path, &identity.origin.vm_name)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(host_command_error(
                    "dmsetup",
                    "the snapshot block device did not become available",
                ));
            }
            std::thread::sleep(RESOURCE_POLL);
        }
    }

    fn snapshot_mapper_path(&self, identity: &SnapshotIdentity) -> PathBuf {
        self.mapper_directory.join(&identity.mapper_name)
    }

    fn verify_mapping(
        &self,
        identity: &MappingIdentity,
        rootfs: &RootfsIdentity,
        mapper: &MapperInfo,
    ) -> Result<DmTable, SdkError> {
        if mapper.name != identity.mapper_name || mapper.uuid != identity.mapper_uuid {
            return Err(storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "Device Mapper ownership does not match the expected VM identity",
            ));
        }
        let output = self.run_checked(
            "dmsetup",
            vec![
                OsString::from("table"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        let table = parse_dm_table(&output.stdout).ok_or_else(|| {
            storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "the owned mapper table is not one snapshot-origin target",
            )
        })?;
        let expected_sectors = rootfs.size_bytes / SECTOR_SIZE_BYTES;
        if table.start_sector != 0
            || table.sector_count != expected_sectors
            || table.target != "snapshot-origin"
        {
            return Err(storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "the owned mapper target or sector range does not match the root disk",
            ));
        }
        let loops = self.find_loops_for_rootfs(rootfs)?;
        if loops.len() != 1 {
            return Err(storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "the root disk does not have exactly one unambiguous loop association",
            ));
        }
        let loop_device = &loops[0];
        if !dependency_matches(&table.dependency, loop_device)
            || loop_device.read_only
            || loop_device.offset_bytes != 0
            || (loop_device.size_limit_bytes != 0
                && loop_device.size_limit_bytes < rootfs.size_bytes)
        {
            return Err(storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "the mapper dependency is not the writable loop device for this root disk",
            ));
        }
        Ok(table)
    }

    fn find_loop(&self, rootfs: &RootfsIdentity) -> Result<Option<LoopInfo>, SdkError> {
        let mut loops = self.find_loops_for_rootfs(rootfs)?;
        if loops.len() > 1 {
            return Err(storage_conflict(
                &rootfs.vm_name,
                &rootfs.canonical_path,
                "multiple loop devices reference this root disk",
            ));
        }
        Ok(loops.pop())
    }

    fn find_loops_for_rootfs(&self, rootfs: &RootfsIdentity) -> Result<Vec<LoopInfo>, SdkError> {
        let output = self.run_checked(
            "losetup",
            vec![
                OsString::from("--json"),
                OsString::from("--list"),
                OsString::from("--output"),
                OsString::from("NAME,BACK-FILE,MAJ:MIN,RO,SIZELIMIT,OFFSET"),
            ],
        )?;
        parse_loop_list(&output.stdout, rootfs)
    }

    fn loop_for_dependency(
        &self,
        rootfs: &RootfsIdentity,
        dependency: &str,
    ) -> Result<LoopInfo, SdkError> {
        let loops = self.find_loops_for_rootfs(rootfs)?;
        match loops.as_slice() {
            [device]
                if dependency_matches(dependency, device)
                    && !device.read_only
                    && device.offset_bytes == 0
                    && (device.size_limit_bytes == 0
                        || device.size_limit_bytes >= rootfs.size_bytes) =>
            {
                Ok(device.clone())
            }
            _ => Err(storage_conflict(
                &rootfs.vm_name,
                &rootfs.canonical_path,
                "the mapper dependency no longer identifies this VM's writable loop device",
            )),
        }
    }

    fn detach_verified_loop(
        &self,
        rootfs: &RootfsIdentity,
        loop_device: &LoopInfo,
    ) -> Result<(), SdkError> {
        let current = self.loop_for_dependency(rootfs, &loop_device.major_minor)?;
        if current.name != loop_device.name {
            return Err(storage_conflict(
                &rootfs.vm_name,
                &rootfs.canonical_path,
                "the loop device changed before it could be detached",
            ));
        }
        self.run_checked(
            "losetup",
            vec![
                OsString::from("--detach"),
                OsString::from(&loop_device.name),
            ],
        )
        .map_err(|error| map_busy_error(error, "detach the owned loop device"))?;
        self.wait_until_loop_gone(rootfs)
    }

    fn remove_created_mapping(
        &self,
        identity: &MappingIdentity,
        rootfs: &RootfsIdentity,
        loop_device: &LoopInfo,
    ) -> Result<(), SdkError> {
        let Some(mapper) = self.find_mapper(identity)? else {
            return Ok(());
        };
        if mapper.uuid != identity.mapper_uuid || mapper.open_count != 0 {
            return Err(temporary_runtime(
                "the newly created mapper cannot be removed safely",
            ));
        }
        let table = self.verify_mapping(identity, rootfs, &mapper)?;
        if !dependency_matches(&table.dependency, loop_device) {
            return Err(storage_conflict(
                &identity.vm_name,
                &rootfs.canonical_path,
                "the newly created mapper points at an unexpected loop device",
            ));
        }
        self.run_checked(
            "dmsetup",
            vec![
                OsString::from("remove"),
                OsString::from(&identity.mapper_name),
            ],
        )?;
        self.wait_until_mapper_gone(identity)
    }

    fn wait_until_mapper_gone(&self, identity: &MappingIdentity) -> Result<(), SdkError> {
        let deadline = Instant::now() + RESOURCE_WAIT;
        loop {
            if self.find_mapper(identity)?.is_none() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(temporary_runtime(
                    "the Device Mapper resource did not disappear after removal",
                ));
            }
            std::thread::sleep(RESOURCE_POLL);
        }
    }

    fn wait_until_loop_gone(&self, rootfs: &RootfsIdentity) -> Result<(), SdkError> {
        let deadline = Instant::now() + RESOURCE_WAIT;
        loop {
            if self.find_loop(rootfs)?.is_none() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(temporary_runtime(
                    "the loop association did not disappear after detachment",
                ));
            }
            std::thread::sleep(RESOURCE_POLL);
        }
    }

    fn wait_for_mapper_node(&self, identity: &MappingIdentity) -> Result<(), SdkError> {
        let path = self.mapper_path(identity);
        let deadline = Instant::now() + RESOURCE_WAIT;
        loop {
            if mapper_node_ready(&path, identity)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(host_command_error(
                    "dmsetup",
                    "the mapper block device did not become available",
                ));
            }
            std::thread::sleep(RESOURCE_POLL);
        }
    }

    fn mapper_path(&self, identity: &MappingIdentity) -> PathBuf {
        self.mapper_directory.join(&identity.mapper_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MappingIdentity {
    canonical_home: PathBuf,
    vm_name: String,
    digest: String,
    mapper_name: String,
    mapper_uuid: String,
}

impl MappingIdentity {
    fn new(home: &Path, vm_name: &str) -> Result<Self, SdkError> {
        validate_vm_name(vm_name)?;
        let canonical_home = fs::canonicalize(home)
            .map_err(|error| SdkError::filesystem("canonicalize SDK home", home, error))?;
        let home_bytes = path_bytes(&canonical_home);
        let name_bytes = vm_name.as_bytes();
        let mut hash = Sha256::new();
        hash.update((home_bytes.len() as u64).to_be_bytes());
        hash.update(&home_bytes);
        hash.update((name_bytes.len() as u64).to_be_bytes());
        hash.update(name_bytes);
        let digest = hex_digest(hash.finalize().as_slice());
        let readable_name = format!("tmvm-{vm_name}-{}", &digest[..16]);
        if readable_name.len() > DEVICE_MAPPER_NAME_MAX {
            return Err(SdkError::InvalidRequest {
                field: "name".to_owned(),
                reason: "the derived Device Mapper name exceeds the Linux limit".to_owned(),
            });
        }
        Ok(Self {
            canonical_home,
            vm_name: vm_name.to_owned(),
            digest: digest.clone(),
            mapper_name: readable_name,
            mapper_uuid: format!("TAUMARU-MICROVM-{digest}"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotIdentity {
    origin: MappingIdentity,
    mapper_name: String,
    mapper_uuid: String,
    cow_path: PathBuf,
}

impl SnapshotIdentity {
    fn new(home: &Path, vm_name: &str) -> Result<Self, SdkError> {
        let origin = MappingIdentity::new(home, vm_name)?;
        let mapper_name = format!("tmsnap-{}", &origin.digest[..24]);
        let mapper_uuid = format!("TAUMARU-MICROVM-SNAPSHOT-{}", origin.digest);
        let cow_path = origin
            .canonical_home
            .join("tmp")
            .join("snapshots")
            .join(format!("snapshot-{}.cow", origin.digest));
        Ok(Self {
            origin,
            mapper_name,
            mapper_uuid,
            cow_path,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotDmTable {
    start_sector: u64,
    sector_count: u64,
    target: String,
    origin_dependency: String,
    cow_dependency: String,
    mode: String,
    chunk_sectors: u64,
}

#[derive(Clone, Debug)]
struct RootfsIdentity {
    vm_name: String,
    canonical_path: PathBuf,
    device: u64,
    inode: u64,
    size_bytes: u64,
}

impl RootfsIdentity {
    fn new(path: &Path, vm_name: &str) -> Result<Self, SdkError> {
        let canonical_path = fs::canonicalize(path)
            .map_err(|error| SdkError::filesystem("canonicalize root disk", path, error))?;
        let metadata = fs::metadata(&canonical_path)
            .map_err(|error| SdkError::filesystem("inspect root disk", &canonical_path, error))?;
        if !metadata.is_file() {
            return Err(SdkError::InvalidRequest {
                field: "rootfs_path".to_owned(),
                reason: "the VM root disk must be a regular file".to_owned(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                vm_name: vm_name.to_owned(),
                canonical_path,
                device: metadata.dev(),
                inode: metadata.ino(),
                size_bytes: metadata.len(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(SdkError::RuntimeIncompatible {
                component: "loop-device-mapping".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: "runtime disk mapping requires a Unix host".to_owned(),
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MapperInfo {
    name: String,
    uuid: String,
    open_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoopInfo {
    name: String,
    backing_path: PathBuf,
    major_minor: String,
    read_only: bool,
    size_limit_bytes: u64,
    offset_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DmTable {
    start_sector: u64,
    sector_count: u64,
    target: String,
    dependency: String,
}

trait HostCommandRunner: Send + Sync {
    fn run(&self, program: &str, arguments: &[OsString]) -> Result<Output, SdkError>;
}

struct SystemCommandRunner;

impl HostCommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, arguments: &[OsString]) -> Result<Output, SdkError> {
        Command::new(program)
            .args(arguments)
            .output()
            .map_err(|error| {
                host_command_error(program, &format!("could not execute command: {error}"))
            })
    }
}

fn dependency_matches(dependency: &str, loop_device: &LoopInfo) -> bool {
    dependency == loop_device.major_minor || dependency == loop_device.name
}

fn parse_mapper_list(bytes: &[u8]) -> Result<Vec<MapperInfo>, SdkError> {
    let text = output_text(bytes);
    let mut mappers = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let columns: Vec<_> = line.split('\t').map(str::trim).collect();
        if columns.len() != 3 {
            return Err(host_command_error(
                "dmsetup",
                "Device Mapper info output did not contain name, UUID, and open count columns",
            ));
        }
        let open_count = columns[2].parse::<u32>().map_err(|error| {
            host_command_error("dmsetup", &format!("invalid mapper open count: {error}"))
        })?;
        mappers.push(MapperInfo {
            name: columns[0].to_owned(),
            uuid: columns[1].to_owned(),
            open_count,
        });
    }
    Ok(mappers)
}

fn parse_dm_table(bytes: &[u8]) -> Option<DmTable> {
    let text = output_text(bytes);
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next()?.trim();
    if lines.next().is_some() {
        return None;
    }
    let columns: Vec<_> = line.split_whitespace().collect();
    if columns.len() != 4 {
        return None;
    }
    Some(DmTable {
        start_sector: columns[0].parse().ok()?,
        sector_count: columns[1].parse().ok()?,
        target: columns[2].to_owned(),
        dependency: columns[3].to_owned(),
    })
}

fn parse_snapshot_dm_table(bytes: &[u8]) -> Option<SnapshotDmTable> {
    let text = output_text(bytes);
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next()?.trim();
    if lines.next().is_some() {
        return None;
    }
    let columns: Vec<_> = line.split_whitespace().collect();
    if columns.len() != 7 {
        return None;
    }
    Some(SnapshotDmTable {
        start_sector: columns[0].parse().ok()?,
        sector_count: columns[1].parse().ok()?,
        target: columns[2].to_owned(),
        origin_dependency: parse_device_reference(columns[3])?,
        cow_dependency: parse_device_reference(columns[4])?,
        mode: columns[5].to_owned(),
        chunk_sectors: columns[6].parse().ok()?,
    })
}

fn parse_device_reference(value: &str) -> Option<String> {
    if value.contains(':')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b':')
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn snapshot_cow_size(rootfs_size: u64) -> Option<u64> {
    let metadata_reserve = (rootfs_size / 16).max(16 * 1024 * 1024);
    rootfs_size
        .checked_add(metadata_reserve)?
        .checked_add(SECTOR_SIZE_BYTES)
}

fn supports_po_fallback(error: &SdkError) -> bool {
    let reason = error.to_string().to_ascii_lowercase();
    reason.contains("invalid argument")
        || reason.contains("unknown mode")
        || reason.contains("unsupported")
        || reason.contains("not supported")
}

fn snapshot_view_error(vm_name: &str, reason: &str) -> SdkError {
    SdkError::SnapshotViewInvalid {
        vm_name: vm_name.to_owned(),
        reason: reason.to_owned(),
    }
}

fn path_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem(
            "inspect snapshot backing path",
            path,
            error,
        )),
    }
}

#[cfg(target_os = "linux")]
fn block_device_major_minor(path: &Path) -> io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    let device = metadata.rdev();
    let major = ((device >> 8) & 0x0fff) | ((device >> 32) & 0xfffff000);
    let minor = (device & 0x00ff) | ((device >> 12) & 0xffffff00);
    Ok(format!("{major}:{minor}"))
}

#[cfg(not(target_os = "linux"))]
fn block_device_major_minor(_path: &Path) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Device Mapper requires Linux",
    ))
}

#[cfg(unix)]
fn snapshot_mapper_node_ready(path: &Path, vm_name: &str) -> Result<bool, SdkError> {
    use std::os::unix::fs::FileTypeExt;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(SdkError::filesystem(
                "inspect snapshot mapper device",
                path,
                error,
            ));
        }
    };
    if !metadata.file_type().is_block_device() {
        return Err(snapshot_view_error(
            vm_name,
            "the snapshot mapper path is not a block device",
        ));
    }
    OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| SdkError::filesystem("open snapshot mapper device", path, error))?;
    Ok(true)
}

#[cfg(not(unix))]
fn snapshot_mapper_node_ready(_path: &Path, _vm_name: &str) -> Result<bool, SdkError> {
    Ok(false)
}

fn parse_loop_list(
    bytes: &[u8],
    expected_rootfs: &RootfsIdentity,
) -> Result<Vec<LoopInfo>, SdkError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| host_command_error("losetup", &format!("invalid JSON output: {error}")))?;
    let devices = value
        .get("loopdevices")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            host_command_error("losetup", "loop listing omitted the loopdevices array")
        })?;
    let mut matches = Vec::new();
    for device in devices {
        let Some(backing) = device.get("back-file").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let backing_path = PathBuf::from(backing);
        let canonical_backing = fs::canonicalize(&backing_path).unwrap_or(backing_path.clone());
        if canonical_backing != expected_rootfs.canonical_path {
            continue;
        }
        let backing_metadata = fs::metadata(&canonical_backing).map_err(|error| {
            SdkError::filesystem("inspect loop backing file", &canonical_backing, error)
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if backing_metadata.dev() != expected_rootfs.device
                || backing_metadata.ino() != expected_rootfs.inode
            {
                continue;
            }
        }
        let name = required_json_string(device, "name")?;
        let major_minor = required_json_string(device, "maj:min")?;
        let read_only = device
            .get("ro")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                host_command_error("losetup", "loop listing omitted the read-only flag")
            })?;
        let size_limit_bytes = required_json_u64(device, "sizelimit")?;
        let offset_bytes = required_json_u64(device, "offset")?;
        matches.push(LoopInfo {
            name,
            backing_path: canonical_backing,
            major_minor,
            read_only,
            size_limit_bytes,
            offset_bytes,
        });
    }
    Ok(matches)
}

#[cfg(unix)]
fn mapper_node_ready(path: &Path, identity: &MappingIdentity) -> Result<bool, SdkError> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(SdkError::filesystem(
                "inspect mapper block device",
                path,
                error,
            ));
        }
    };
    if !metadata.file_type().is_block_device() {
        return Err(storage_conflict(
            &identity.vm_name,
            path,
            "the mapper path does not resolve to a block device",
        ));
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map(|_| true)
        .map_err(|error| SdkError::RuntimeIncompatible {
            component: "device-mapper".to_owned(),
            package_id: "host".to_owned(),
            version: "unknown".to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            reason: format!("mapper block device is not readable and writable: {error}"),
        })
}

#[cfg(not(unix))]
fn mapper_node_ready(_path: &Path, _identity: &MappingIdentity) -> Result<bool, SdkError> {
    Ok(false)
}

fn required_json_u64(value: &serde_json::Value, key: &str) -> Result<u64, SdkError> {
    let field = value
        .get(key)
        .ok_or_else(|| host_command_error("losetup", &format!("loop listing omitted {key}")))?;
    field
        .as_u64()
        .or_else(|| field.as_str().and_then(|number| number.parse::<u64>().ok()))
        .ok_or_else(|| host_command_error("losetup", &format!("loop listing has an invalid {key}")))
}

fn required_json_string(value: &serde_json::Value, key: &str) -> Result<String, SdkError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| host_command_error("losetup", &format!("loop listing omitted {key}")))
}

fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().as_bytes().to_vec()
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    let mut encoded = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn host_command_error(program: &str, reason: &str) -> SdkError {
    SdkError::HostCommand {
        program: program.to_owned(),
        reason: reason.to_owned(),
    }
}

fn storage_conflict(vm_name: &str, rootfs_path: &Path, reason: &str) -> SdkError {
    SdkError::StorageConflict {
        vm_name: vm_name.to_owned(),
        volume_path: rootfs_path.to_path_buf(),
        owner: "runtime disk mapping".to_owned(),
        reason: reason.to_owned(),
    }
}

fn temporary_runtime(reason: impl Into<String>) -> SdkError {
    SdkError::TemporaryRuntime {
        component: "runtime disk mapping".to_owned(),
        reason: reason.into(),
        stopped: false,
    }
}

fn map_busy_error(error: SdkError, operation: &str) -> SdkError {
    match error {
        SdkError::HostCommand { reason, .. } if reason.to_ascii_lowercase().contains("busy") => {
            temporary_runtime(format!(
                "{operation} is blocked because the resource is busy"
            ))
        }
        other => other,
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::{ExitStatus, Output};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{
        DeviceMapperRuntime, DmTable, HostCommandRunner, LoopInfo, MappingIdentity, RootfsIdentity,
        SnapshotIdentity, acquire_lifecycle_lock, parse_dm_table, parse_loop_list,
        parse_snapshot_dm_table, snapshot_cow_size,
    };
    use crate::error::SdkError;
    use crate::ports::runtime_disk::RuntimeDiskController;
    use tempfile::tempdir;

    #[derive(Default)]
    struct ScriptedCommands {
        responses: Mutex<HashMap<String, VecDeque<Output>>>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl ScriptedCommands {
        fn push(&self, key: &str, output: Output) {
            self.responses
                .lock()
                .expect("script response lock")
                .entry(key.to_owned())
                .or_default()
                .push_back(output);
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().expect("script call lock").clone()
        }
    }

    impl HostCommandRunner for ScriptedCommands {
        fn run(&self, program: &str, arguments: &[OsString]) -> Result<Output, SdkError> {
            let args = arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let operation = if program == "dmsetup" {
                args.first().map(String::as_str).unwrap_or("unknown")
            } else if program == "losetup" && args.iter().any(|argument| argument == "--list") {
                "list"
            } else if program == "losetup" && args.iter().any(|argument| argument == "--detach") {
                "detach"
            } else {
                "other"
            };
            let key = format!("{program}:{operation}");
            self.calls
                .lock()
                .expect("script call lock")
                .push((key.clone(), args));
            let mut responses = self.responses.lock().expect("script response lock");
            let output = responses
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| success_output(""));
            Ok(output)
        }
    }

    fn success_output(stdout: &str) -> Output {
        Output {
            status: success_status(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn failure_output(stderr: &str) -> Output {
        Output {
            status: failure_status(),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn failure_status() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(1 << 8)
        }
        #[cfg(not(unix))]
        {
            panic!("device mapper tests require Unix")
        }
    }

    fn success_status() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
        #[cfg(not(unix))]
        {
            panic!("device mapper tests require Unix")
        }
    }

    fn runtime(commands: Arc<ScriptedCommands>) -> DeviceMapperRuntime {
        DeviceMapperRuntime {
            commands,
            mapper_directory: PathBuf::from("/fake/mapper"),
            verify_mapper_node: false,
        }
    }

    fn identity(home: &Path, name: &str) -> MappingIdentity {
        MappingIdentity::new(home, name).expect("test mapping identity")
    }

    fn root_disk(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(format!("{name}.ext4"));
        std::fs::write(&path, [0x5a; 4096]).expect("root disk fixture");
        path
    }

    fn mapper_listing(identity: &MappingIdentity, open_count: u32) -> String {
        format!(
            "{}\t{}\t{}\n",
            identity.mapper_name, identity.mapper_uuid, open_count
        )
    }

    fn loop_listing(path: &Path, name: &str, major_minor: &str, read_only: bool) -> String {
        serde_json::json!({
            "loopdevices": [{
                "name": name,
                "back-file": path.to_string_lossy(),
                "maj:min": major_minor,
                "ro": read_only,
                "sizelimit": 0,
                "offset": 0
            }]
        })
        .to_string()
    }

    #[test]
    fn mapping_identity_is_stable_and_vm_specific() {
        let home = tempdir().expect("temporary home");
        let first = identity(home.path(), "alpha_vm");
        let second = identity(home.path(), "alpha_vm");
        let other = identity(home.path(), "beta_vm");

        assert_eq!(first, second);
        assert_ne!(first.mapper_name, other.mapper_name);
        assert_ne!(first.mapper_uuid, other.mapper_uuid);
        assert!(first.mapper_name.len() <= 127);
        assert!(first.mapper_uuid.starts_with("TAUMARU-MICROVM-"));
        assert_eq!(first.digest.len(), 64);
    }

    #[test]
    fn snapshot_identity_and_cow_path_are_stable_and_vm_specific() {
        let home = tempdir().expect("temporary home");
        let first = SnapshotIdentity::new(home.path(), "alpha_vm").expect("identity");
        let again = SnapshotIdentity::new(home.path(), "alpha_vm").expect("same identity");
        let other = SnapshotIdentity::new(home.path(), "beta_vm").expect("other identity");

        assert_eq!(first, again);
        assert_ne!(first.mapper_name, other.mapper_name);
        assert_ne!(first.mapper_uuid, other.mapper_uuid);
        assert_ne!(first.cow_path, other.cow_path);
        assert!(first.mapper_name.starts_with("tmsnap-"));
        assert!(first.mapper_uuid.starts_with("TAUMARU-MICROVM-SNAPSHOT-"));
        assert_eq!(snapshot_cow_size(1024 * 1024), Some(17 * 1024 * 1024 + 512));
    }

    #[test]
    fn parses_only_verified_classic_snapshot_tables() {
        assert_eq!(
            parse_snapshot_dm_table(b"0 4096 snapshot 253:3 7:12 PO 64\n"),
            Some(super::SnapshotDmTable {
                start_sector: 0,
                sector_count: 4096,
                target: "snapshot".to_owned(),
                origin_dependency: "253:3".to_owned(),
                cow_dependency: "7:12".to_owned(),
                mode: "PO".to_owned(),
                chunk_sectors: 64,
            })
        );
        assert!(parse_snapshot_dm_table(b"0 4096 linear 253:3 0\n").is_none());
        assert!(
            parse_snapshot_dm_table(b"0 4096 snapshot /dev/mapper/origin 7:12 P 64\n").is_none()
        );
        assert!(
            parse_snapshot_dm_table(b"0 4096 snapshot 253:3 7:12 P 64\n0 1 linear 8:0 0\n")
                .is_none()
        );
    }

    #[test]
    fn snapshot_target_falls_back_from_po_only_when_mode_is_unsupported() {
        let directory = tempdir().expect("test directory");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = SnapshotIdentity::new(&home, "vm_one").expect("snapshot identity");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:create",
            failure_output("Invalid argument: mode PO is unsupported"),
        );
        commands.push("dmsetup:create", success_output(""));
        let adapter = runtime(Arc::clone(&commands));

        adapter
            .create_snapshot_mapping(&identity, "0 8 snapshot 253:1 7:12 PO 64")
            .expect("P mode should be used as compatibility fallback");

        let creates = commands
            .calls()
            .into_iter()
            .filter(|(key, _)| key == "dmsetup:create")
            .collect::<Vec<_>>();
        assert_eq!(creates.len(), 2);
        let first_table = creates[0]
            .1
            .iter()
            .position(|arg| arg == "--table")
            .unwrap();
        let second_table = creates[1]
            .1
            .iter()
            .position(|arg| arg == "--table")
            .unwrap();
        assert!(creates[0].1[first_table + 1].contains(" PO 64"));
        assert!(creates[1].1[second_table + 1].contains(" P 64"));
        assert!(
            creates
                .iter()
                .all(|(_, args)| args.iter().any(|arg| arg == "--readonly"))
        );
    }

    #[test]
    fn detects_snapshot_overflow_and_orders_origin_suspend_resume() {
        let directory = tempdir().expect("test directory");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = SnapshotIdentity::new(&home, "vm_one").expect("snapshot identity");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:status",
            success_output("0 8 snapshot 8/8 Overflow\n"),
        );
        commands.push("dmsetup:suspend", success_output(""));
        commands.push("dmsetup:resume", success_output(""));
        let adapter = runtime(Arc::clone(&commands));

        let error = adapter
            .check_snapshot_status(&identity)
            .expect_err("overflow must invalidate the snapshot");
        assert!(matches!(error, SdkError::SnapshotViewInvalid { .. }));
        adapter
            .suspend_origin(&identity.origin)
            .expect("origin suspends");
        adapter
            .resume_origin(&identity.origin)
            .expect("origin resumes");
        let calls = commands.calls();
        let suspend = calls
            .iter()
            .position(|(key, _)| key == "dmsetup:suspend")
            .unwrap();
        let resume = calls
            .iter()
            .position(|(key, _)| key == "dmsetup:resume")
            .unwrap();
        assert!(suspend < resume);
    }

    #[test]
    fn snapshot_cleanup_detaches_only_its_verified_cow_loop() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = SnapshotIdentity::new(&home, "vm_one").expect("snapshot identity");
        let cow_dir = identity.cow_path.parent().expect("COW parent");
        std::fs::create_dir_all(cow_dir).expect("COW directory");
        std::fs::write(&identity.cow_path, [0xa5; 8192]).expect("COW file");
        let cow = RootfsIdentity::new(&identity.cow_path, "vm_one").expect("COW identity");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "losetup:list",
            success_output(&loop_listing(
                &identity.cow_path,
                "/dev/loop21",
                "7:21",
                false,
            )),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(
                &identity.cow_path,
                "/dev/loop21",
                "7:21",
                false,
            )),
        );
        commands.push("losetup:detach", success_output(""));
        commands.push("losetup:list", success_output("{\"loopdevices\":[]}"));
        let adapter = runtime(Arc::clone(&commands));

        adapter
            .cleanup_snapshot_backing(&identity, &cow)
            .expect("owned COW loop and file should be removed");

        let calls = commands.calls();
        let detach = calls
            .iter()
            .find(|(key, _)| key == "losetup:detach")
            .expect("COW detach call");
        assert_eq!(detach.1.last().map(String::as_str), Some("/dev/loop21"));
        assert!(rootfs.is_file(), "persistent rootfs must remain untouched");
        assert!(!identity.cow_path.exists());
    }

    #[test]
    fn parses_only_one_snapshot_origin_table_with_a_single_dependency() {
        assert_eq!(
            parse_dm_table(b"0 4096 snapshot-origin 7:12\n"),
            Some(DmTable {
                start_sector: 0,
                sector_count: 4096,
                target: "snapshot-origin".to_owned(),
                dependency: "7:12".to_owned(),
            })
        );
        assert_eq!(parse_dm_table(b"0 4096 linear 7:12 0\n"), None);
        assert_eq!(
            parse_dm_table(b"0 4096 snapshot-origin 7:12\n0 4 linear 8:0 0\n"),
            None
        );
    }

    #[test]
    fn reuses_only_a_mapper_with_matching_uuid_table_and_root_disk() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:info",
            success_output(&mapper_listing(&identity, 0)),
        );
        commands.push(
            "dmsetup:table",
            success_output("0 8 snapshot-origin 7:12\n"),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        let adapter = runtime(commands);

        let mapped = adapter
            .ensure_mapping(&home, "vm_one", &rootfs)
            .expect("matching mapper should be reused");

        assert_eq!(
            mapped,
            PathBuf::from("/fake/mapper").join(identity.mapper_name)
        );
    }

    #[test]
    fn creates_writable_loop_device_without_unsupported_rw_option() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push("dmsetup:info", success_output(""));
        commands.push(
            "dmsetup:info",
            success_output(&mapper_listing(&identity, 0)),
        );
        commands.push(
            "dmsetup:table",
            success_output("0 8 snapshot-origin 7:12\n"),
        );
        commands.push("losetup:list", success_output(r#"{"loopdevices":[]}"#));
        commands.push("losetup:other", success_output("/dev/loop12\n"));
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        let adapter = runtime(Arc::clone(&commands));

        let mapped = adapter
            .ensure_mapping(&home, "vm_one", &rootfs)
            .expect("mapping should be created");

        assert_eq!(
            mapped,
            PathBuf::from("/fake/mapper").join(identity.mapper_name)
        );
        let allocation = commands
            .calls()
            .into_iter()
            .find(|(key, _)| key == "losetup:other")
            .expect("loop allocation call");
        assert!(allocation.1.iter().any(|argument| argument == "--find"));
        assert!(allocation.1.iter().any(|argument| argument == "--show"));
        assert!(
            allocation
                .1
                .iter()
                .any(|argument| argument == "--nooverlap")
        );
        assert!(!allocation.1.iter().any(|argument| argument == "--rw"));
        assert!(
            !allocation
                .1
                .iter()
                .any(|argument| argument == "--read-only")
        );
    }

    #[test]
    fn rejects_a_foreign_uuid_under_the_expected_mapper_name_without_mutation() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:info",
            success_output(&format!("{}\tFOREIGN-UUID\t0\n", identity.mapper_name)),
        );
        let adapter = runtime(Arc::clone(&commands));

        let error = adapter
            .ensure_mapping(&home, "vm_one", &rootfs)
            .expect_err("foreign identity must be rejected");

        assert!(matches!(error, SdkError::StorageConflict { .. }));
        assert!(
            commands
                .calls()
                .iter()
                .all(|(key, _)| key != "dmsetup:create" && key != "losetup:detach")
        );
    }

    #[test]
    fn rejects_a_mapper_that_resolves_to_a_different_backing_file() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let foreign = root_disk(directory.path(), "foreign");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:info",
            success_output(&mapper_listing(&identity, 0)),
        );
        commands.push(
            "dmsetup:table",
            success_output("0 8 snapshot-origin 7:12\n"),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&foreign, "/dev/loop12", "7:12", false)),
        );
        let adapter = runtime(Arc::clone(&commands));

        let error = adapter
            .ensure_mapping(&home, "vm_one", &rootfs)
            .expect_err("foreign backing file must be rejected");

        assert!(matches!(error, SdkError::StorageConflict { .. }));
        assert!(
            commands
                .calls()
                .iter()
                .all(|(key, _)| key != "dmsetup:create" && key != "losetup:detach")
        );
    }

    #[test]
    fn releases_the_owned_mapper_before_its_exact_loop_and_keeps_other_mappings() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let foreign = root_disk(directory.path(), "other");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let other = MappingIdentity::new(&home, "vm_two").expect("second mapping identity");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:info",
            success_output(&format!(
                "{}{}\n",
                mapper_listing(&identity, 0),
                mapper_listing(&other, 0)
            )),
        );
        commands.push(
            "dmsetup:table",
            success_output("0 8 snapshot-origin 7:12\n"),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push("dmsetup:remove", success_output(""));
        commands.push("dmsetup:info", success_output(&mapper_listing(&other, 0)));
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push("losetup:detach", success_output(""));
        commands.push("losetup:list", success_output("{\"loopdevices\":[]}"));
        let adapter = runtime(Arc::clone(&commands));

        adapter
            .release_mapping(&home, "vm_one", &rootfs)
            .expect("owned mapping should be released");

        let calls = commands.calls();
        let remove = calls
            .iter()
            .position(|(key, _)| key == "dmsetup:remove")
            .expect("mapper removal should run");
        let detach = calls
            .iter()
            .position(|(key, _)| key == "losetup:detach")
            .expect("loop detachment should run");
        assert!(remove < detach);
        assert_eq!(
            calls[detach].1.last().map(String::as_str),
            Some("/dev/loop12")
        );
        assert!(calls.iter().all(|(key, args)| key != "dmsetup:remove"
            || args.get(1).map(String::as_str) == Some(identity.mapper_name.as_str())));
        assert!(foreign.is_file());
        assert!(rootfs.is_file());
    }

    #[test]
    fn releases_a_verified_orphan_loop_without_deleting_the_root_disk() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push("dmsetup:info", success_output(""));
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push(
            "losetup:list",
            success_output(&loop_listing(&rootfs, "/dev/loop12", "7:12", false)),
        );
        commands.push("losetup:detach", success_output(""));
        commands.push("losetup:list", success_output("{\"loopdevices\":[]}"));
        let adapter = runtime(Arc::clone(&commands));

        adapter
            .release_mapping(&home, "vm_one", &rootfs)
            .expect("verified orphan loop should be detached");

        let calls = commands.calls();
        let detach = calls
            .iter()
            .find(|(key, _)| key == "losetup:detach")
            .expect("exact loop detachment should run");
        assert_eq!(detach.1.last().map(String::as_str), Some("/dev/loop12"));
        assert!(rootfs.is_file());
    }

    #[test]
    fn leaves_busy_mapper_and_loop_resources_untouched() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).expect("test home");
        let identity = identity(&home, "vm_one");
        let commands = Arc::new(ScriptedCommands::default());
        commands.push(
            "dmsetup:info",
            success_output(&mapper_listing(&identity, 1)),
        );
        let adapter = runtime(Arc::clone(&commands));

        let error = adapter
            .release_mapping(&home, "vm_one", &rootfs)
            .expect_err("open mapper must remain busy");

        assert!(matches!(error, SdkError::TemporaryRuntime { .. }));
        assert!(
            commands
                .calls()
                .iter()
                .all(|(key, _)| key != "dmsetup:remove" && key != "losetup:detach")
        );
    }

    #[tokio::test]
    async fn lifecycle_lock_waits_without_unlinking_its_lock_file() {
        let home = tempdir().expect("temporary home");
        let first = acquire_lifecycle_lock(home.path(), "vm_one")
            .await
            .expect("first lifecycle lock");
        let lock_path = std::fs::read_dir(home.path().join("runtime/locks"))
            .expect("lock directory")
            .next()
            .expect("lock file entry")
            .expect("lock file")
            .path();
        assert!(lock_path.is_file());

        let waiting = {
            let home = home.path().to_path_buf();
            tokio::spawn(async move { acquire_lifecycle_lock(&home, "vm_one").await })
        };
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!waiting.is_finished());
        drop(first);
        let second = waiting
            .await
            .expect("lock task should join")
            .expect("second lock should acquire");
        assert!(lock_path.is_file());
        drop(second);
    }

    #[test]
    fn loop_listing_matches_file_identity_not_only_the_backing_path() {
        let directory = tempdir().expect("test directory");
        let rootfs = root_disk(directory.path(), "root");
        let identity = RootfsIdentity::new(&rootfs, "vm_one").expect("rootfs identity");
        let output = loop_listing(&rootfs, "/dev/loop12", "7:12", false);
        let loops = parse_loop_list(output.as_bytes(), &identity).expect("loop listing");
        assert_eq!(
            loops,
            vec![LoopInfo {
                name: "/dev/loop12".to_owned(),
                backing_path: rootfs,
                major_minor: "7:12".to_owned(),
                read_only: false,
                size_limit_bytes: 0,
                offset_bytes: 0,
            }]
        );
    }
}
