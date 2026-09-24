use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

use super::{MicroVmSdk, SnapshotFutureCancellation, host_architecture, unix_timestamp};
use crate::adapters::archive::manifest::{
    KERNEL_MEMBER, NetworkManifest, ROOTFS_MEMBER, SnapshotManifest,
};
use crate::adapters::archive::restore::{self, StagedSnapshot};
use crate::adapters::runtime::device_mapper::VmLifecycleLock;
use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity, InstalledBinary};
use crate::domain::lifecycle::{MicroVmState, NetworkMode};
use crate::domain::microvm::{
    MicroVmRecord, PersistedCredential, PersistedNetwork, PersistedRuntime, SshConnectionInfo,
};
use crate::domain::registry::{Architecture, Kernel};
use crate::domain::restore::{
    RestoreCancellation, RestoreProgress, RestoreProgressStage, RestoreRequest, RestoreResult,
};
use crate::error::SdkError;
use crate::ports::network::{NetworkController, NetworkRequest};
use crate::ports::repository::{
    LocalRepository, RestoreJournal, RestoredMicroVmCommit, RestoredSnapshotMetadata,
};
use crate::ports::runtime::RuntimeController;
use crate::ports::storage::GuestStorage;

static RESTORE_OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);
const RESTORE_COPY_BUFFER_BYTES: usize = 128 * 1024;

type ProgressCallback = Arc<Mutex<Box<dyn FnMut(RestoreProgress) + Send>>>;

impl MicroVmSdk {
    /// Restores a stopped MicroVM from one password-encrypted snapshot archive.
    ///
    /// The VM name and portable settings come from the authenticated archive manifest. The
    /// destination must have compatible runtime binaries and free local network/storage
    /// resources. Restore never launches Firecracker and never overwrites existing paths.
    pub async fn restore_snapshot(
        &self,
        request: RestoreRequest,
    ) -> Result<RestoreResult, SdkError> {
        self.restore_snapshot_with_cancellation_and_progress(
            request,
            RestoreCancellation::new(),
            |_| {},
        )
        .await
    }

    /// Restores a snapshot and emits caller-owned progress events.
    pub async fn restore_snapshot_with_progress<F>(
        &self,
        request: RestoreRequest,
        on_progress: F,
    ) -> Result<RestoreResult, SdkError>
    where
        F: FnMut(RestoreProgress) + Send + 'static,
    {
        self.restore_snapshot_with_cancellation_and_progress(
            request,
            RestoreCancellation::new(),
            on_progress,
        )
        .await
    }

    /// Restores a snapshot with cooperative cancellation and progress reporting.
    ///
    /// Cancellation is checked while decrypting and staging payload bytes and while copying the
    /// root disk. Operation-owned files and network resources are cleaned before an error is
    /// returned. Dropping the future requests cancellation while its blocking worker keeps the
    /// per-VM lifecycle locks until cleanup finishes.
    pub async fn restore_snapshot_with_cancellation_and_progress<F>(
        &self,
        request: RestoreRequest,
        cancellation: RestoreCancellation,
        on_progress: F,
    ) -> Result<RestoreResult, SdkError>
    where
        F: FnMut(RestoreProgress) + Send + 'static,
    {
        if request.password.is_empty() {
            return Err(SdkError::InvalidRequest {
                field: "password".to_owned(),
                reason: "must not be empty".to_owned(),
            });
        }
        let operation_id = new_restore_operation_id()?;
        let staging_parent = self.home.join("tmp").join("restores");
        ensure_directory(&staging_parent, 0o700)?;
        let staging_path = staging_parent.join(&operation_id);
        let archive_path = normalize_archive_path(&request.archive_path)?;
        let callback: ProgressCallback = Arc::new(Mutex::new(Box::new(on_progress)));
        let read_callback = Arc::clone(&callback);
        let token = cancellation.token();
        let future_cancellation = SnapshotFutureCancellation(token.clone());
        let password = request.password;
        let stage_token = token.clone();
        let stage_worker = tokio::task::spawn_blocking(move || {
            let mut progress = |completed_bytes, total_bytes| {
                emit_progress(
                    &read_callback,
                    RestoreProgress {
                        stage: RestoreProgressStage::Staging,
                        completed_bytes,
                        total_bytes,
                    },
                );
            };
            restore::read_archive(
                &archive_path,
                &password,
                &staging_path,
                stage_token,
                &mut progress,
            )
        });
        let staged = stage_worker.await??;
        if cancellation.is_cancelled() {
            return Err(SdkError::RestoreCancelled);
        }
        let vm_name = staged.manifest.vm.name.clone();
        let runtime_lock = super::acquire_lifecycle_lock(&self.home, &vm_name).await?;
        let name_lock_path = self.home.join("vms").join(&vm_name);
        let name_lock = self.target_lock(&name_lock_path)?;
        let name_guard = name_lock.lock_owned().await;
        let home = self.home.clone();
        let repository = Arc::clone(&self.repository);
        let network = Arc::clone(&self.network);
        let storage = Arc::clone(&self.storage);
        let runtime = Arc::clone(&self.runtime);
        let callback = Arc::clone(&callback);
        let operation_id_for_worker = operation_id.clone();
        let worker = tokio::task::spawn_blocking(move || {
            restore_staged_snapshot(
                home,
                repository,
                network,
                storage,
                runtime,
                staged,
                operation_id_for_worker,
                token,
                callback,
                runtime_lock,
                name_guard,
            )
        });
        let result = worker.await?;
        drop(future_cancellation);
        result
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_staged_snapshot(
    home: PathBuf,
    repository: Arc<dyn LocalRepository>,
    network_controller: Arc<dyn NetworkController>,
    storage: Arc<dyn GuestStorage>,
    runtime: Arc<dyn RuntimeController>,
    staged: StagedSnapshot,
    operation_id: String,
    cancellation: CancellationToken,
    callback: ProgressCallback,
    _runtime_lock: VmLifecycleLock,
    _name_guard: OwnedMutexGuard<()>,
) -> Result<RestoreResult, SdkError> {
    let manifest = staged.manifest;
    let vm_name = manifest.vm.name.clone();
    emit_progress(
        &callback,
        RestoreProgress {
            stage: RestoreProgressStage::Verifying,
            completed_bytes: manifest
                .payloads
                .iter()
                .map(|payload| payload.size_bytes)
                .sum(),
            total_bytes: manifest
                .payloads
                .iter()
                .map(|payload| payload.size_bytes)
                .sum(),
        },
    );
    if cancellation.is_cancelled() {
        return Err(SdkError::RestoreCancelled);
    }
    reconcile_restore_journals(
        &home,
        repository.as_ref(),
        network_controller.as_ref(),
        &vm_name,
    )?;
    if repository.find_microvm(&vm_name)?.is_some() {
        return Err(SdkError::RestoreConflict {
            name: vm_name,
            resource: "inventory name".to_owned(),
            value: "a MicroVM with this name already exists".to_owned(),
        });
    }
    runtime.validate_host()?;
    let host_architecture = host_architecture()?;
    let kernel_architecture = parse_architecture(&manifest.boot.kernel.architecture)?;
    if kernel_architecture != host_architecture {
        return Err(SdkError::IncompatibleArtifact {
            artifact: manifest.boot.kernel.id.clone(),
            reason: format!(
                "embedded kernel architecture {} does not match destination host {}",
                architecture_name(&kernel_architecture),
                architecture_name(&host_architecture)
            ),
        });
    }
    let (firecracker, firectl) = resolve_local_runtime(repository.as_ref(), &host_architecture)?;
    let volume_path = home.join("vms").join(&vm_name);
    if repository.find_volume_owner(&volume_path)?.is_some() || path_entry_exists(&volume_path)? {
        return Err(SdkError::RestoreConflict {
            name: vm_name,
            resource: "VM volume".to_owned(),
            value: format!("{} is already occupied", volume_path.display()),
        });
    }
    let kernel = kernel_from_manifest(&manifest)?;
    let kernel_relative_path = PathBuf::from("artifacts")
        .join("kernels")
        .join(&kernel.id)
        .join(&kernel.filename);
    let kernel_path = home.join(&kernel_relative_path);
    let kernel_payload = payload_record(&manifest, KERNEL_MEMBER)?;
    let existing_kernel = repository.resolve_kernel(&kernel.id).ok();
    if let Some(existing) = existing_kernel.as_ref()
        && existing.path != kernel_path
    {
        return Err(SdkError::RestoreConflict {
            name: vm_name,
            resource: "kernel inventory".to_owned(),
            value: format!(
                "kernel {} is already registered at {}",
                kernel.id,
                existing.path.display()
            ),
        });
    }
    let kernel_is_present = match (existing_kernel.as_ref(), path_entry_exists(&kernel_path)?) {
        (Some(_), true) => {
            verify_regular_file(&kernel_path, "inspect existing restored kernel")?;
            verify_file_integrity(
                &kernel_path,
                kernel_payload.size_bytes,
                &kernel_payload.sha256,
            )?;
            true
        }
        (Some(_), false) => {
            return Err(SdkError::RestoreConflict {
                name: vm_name,
                resource: "kernel inventory".to_owned(),
                value: format!(
                    "kernel {} is registered but its file is unavailable",
                    kernel.id
                ),
            });
        }
        (None, true) => {
            return Err(SdkError::RestoreConflict {
                name: vm_name,
                resource: "kernel path".to_owned(),
                value: format!(
                    "{} is occupied by an unregistered file",
                    kernel_path.display()
                ),
            });
        }
        (None, false) => false,
    };
    if let NetworkManifest::PreserveIpv4 { guest_mac, .. } = &manifest.network {
        for existing in repository.list_stored_microvms()? {
            if existing
                .network
                .as_ref()
                .is_some_and(|network| network.guest_mac.eq_ignore_ascii_case(guest_mac))
            {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "guest MAC".to_owned(),
                    value: guest_mac.to_owned(),
                });
            }
        }
    }
    let mode_bits = match &manifest.network {
        NetworkManifest::PreserveIpv4 { mode, .. } => {
            NetworkMode::parse(mode).ok_or_else(|| SdkError::UnsupportedSnapshotPolicy {
                policy: mode.clone(),
            })?
        }
        NetworkManifest::RegenerateIpv4 { expose_on_lan } => {
            if *expose_on_lan {
                NetworkMode::Lan
            } else {
                NetworkMode::HostOnly
            }
        }
    };
    let expose_on_lan = matches!(
        manifest.network,
        NetworkManifest::PreserveIpv4 {
            expose_on_lan: true,
            ..
        } | NetworkManifest::RegenerateIpv4 {
            expose_on_lan: true
        }
    );
    let (guest_mac, guest_override, prefix_override, gateway_override, lan_override, exact) =
        match &manifest.network {
            NetworkManifest::PreserveIpv4 {
                guest_ipv4,
                prefix_length,
                guest_gateway_ipv4,
                lan_ipv4,
                guest_mac,
                ..
            } => (
                guest_mac.clone(),
                Some(*guest_ipv4),
                Some(*prefix_length),
                *guest_gateway_ipv4,
                *lan_ipv4,
                true,
            ),
            NetworkManifest::RegenerateIpv4 { .. } => (
                crate::adapters::network::linux::guest_mac(&vm_name),
                None,
                None,
                None,
                None,
                false,
            ),
        };
    let request = NetworkRequest {
        vm_name: vm_name.clone(),
        mode: mode_bits,
        guest_mac: guest_mac.clone(),
        lan_address_override: lan_override,
        guest_address_override: guest_override,
        prefix_length_override: prefix_override,
        gateway_override,
        exact_network_values: exact,
    };
    let staging_path = staged
        .rootfs_path
        .parent()
        .ok_or_else(|| SdkError::RestoreRecovery {
            operation_id: operation_id.clone(),
            reason: "staging directory path is unavailable".to_owned(),
        })?
        .to_path_buf();
    let mut journal = RestoreJournal {
        operation_id: operation_id.clone(),
        vm_name: vm_name.clone(),
        staging_path,
        volume_path: volume_path.clone(),
        volume_created: false,
        kernel_path: kernel_path.clone(),
        kernel_created: false,
        network: None,
        progress_state: "validated".to_owned(),
    };
    repository.record_restore_journal(&journal)?;
    let mut installed_network: Option<PersistedNetwork> = None;
    let result = (|| {
        if cancellation.is_cancelled() {
            return Err(SdkError::RestoreCancelled);
        }
        emit_progress(
            &callback,
            RestoreProgress {
                stage: RestoreProgressStage::PreparingDestination,
                completed_bytes: 0,
                total_bytes: 0,
            },
        );
        let used_addresses = repository.list_host_only_networks()?;
        let used_lan_addresses = repository.list_lan_addresses()?;
        let network_outcome =
            network_controller.configure(&request, None, &used_addresses, &used_lan_addresses)?;
        verify_network_outcome(&manifest, &network_outcome.persisted)?;
        installed_network = Some(network_outcome.persisted.clone());
        journal.network = Some(network_outcome.persisted.clone());
        journal.progress_state = "network_configured".to_owned();
        repository.record_restore_journal(&journal)?;
        if cancellation.is_cancelled() {
            return Err(SdkError::RestoreCancelled);
        }
        emit_progress(
            &callback,
            RestoreProgress {
                stage: RestoreProgressStage::Installing,
                completed_bytes: 0,
                total_bytes: manifest.vm.disk_size_bytes,
            },
        );
        ensure_directory(&home.join("vms"), 0o700)?;
        journal.progress_state = "volume_creation_started".to_owned();
        repository.record_restore_journal(&journal)?;
        fs::create_dir(&volume_path).map_err(|error| {
            SdkError::filesystem("create restored VM volume", &volume_path, error)
        })?;
        journal.volume_created = true;
        journal.progress_state = "volume_created".to_owned();
        repository.record_restore_journal(&journal)?;
        set_mode(&volume_path, 0o700)?;
        let rootfs_path = volume_path.join("rootfs.ext4");
        let rootfs_payload = payload_record(&manifest, ROOTFS_MEMBER)?;
        let copy_callback = Arc::clone(&callback);
        storage.copy_rootfs_exact(
            &staged.rootfs_path,
            &rootfs_path,
            manifest.vm.disk_size_bytes,
            cancellation.clone(),
            &mut move |completed_bytes, total_bytes| {
                emit_progress(
                    &copy_callback,
                    RestoreProgress {
                        stage: RestoreProgressStage::Installing,
                        completed_bytes,
                        total_bytes,
                    },
                );
            },
        )?;
        set_mode(&rootfs_path, 0o600)?;
        let verification_callback = Arc::clone(&callback);
        verify_file_integrity_with_progress(
            &rootfs_path,
            rootfs_payload.size_bytes,
            &rootfs_payload.sha256,
            Some(&cancellation),
            &mut move |completed_bytes, total_bytes| {
                emit_progress(
                    &verification_callback,
                    RestoreProgress {
                        stage: RestoreProgressStage::Verifying,
                        completed_bytes,
                        total_bytes,
                    },
                );
            },
        )?;
        let ssh_directory = volume_path.join("ssh");
        ensure_directory(&ssh_directory, 0o700)?;
        let private_key_path = ssh_directory.join("id_ed25519");
        let public_key_path = ssh_directory.join("id_ed25519.pub");
        copy_payload(&staged.private_key_path, &private_key_path, 0o600)?;
        copy_payload(&staged.public_key_path, &public_key_path, 0o644)?;
        if !kernel_is_present {
            ensure_directory(&home.join("artifacts"), 0o700)?;
            ensure_directory(&home.join("artifacts").join("kernels"), 0o700)?;
            ensure_directory(kernel_path.parent().unwrap_or(&home), 0o700)?;
            journal.kernel_created = true;
            journal.progress_state = "kernel_installation_started".to_owned();
            repository.record_restore_journal(&journal)?;
            copy_payload(&staged.kernel_path, &kernel_path, 0o644)?;
            verify_file_integrity(
                &kernel_path,
                kernel_payload.size_bytes,
                &kernel_payload.sha256,
            )?;
        }
        let network = network_outcome.persisted;
        let IpAddr::V4(guest_address) = network.config.guest_address else {
            return Err(SdkError::SnapshotNetworkConflict {
                field: "guest IPv4".to_owned(),
                value: network.config.guest_address.to_string(),
            });
        };
        let gateway = match network.config.gateway {
            Some(IpAddr::V4(value)) => Some(value),
            Some(value) => {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "gateway IPv4".to_owned(),
                    value: value.to_string(),
                });
            }
            None => None,
        };
        let lan_address = match network.config.lan_address {
            Some(IpAddr::V4(value)) => Some(value),
            Some(value) => {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "LAN IPv4".to_owned(),
                    value: value.to_string(),
                });
            }
            None => None,
        };
        storage.write_guest_ipv4_config(
            &rootfs_path,
            guest_address,
            network.config.prefix_length,
            gateway,
            lan_address,
        )?;
        let socket_path = volume_path.join("firecracker.sock");
        runtime.verify_stopped(&socket_path)?;
        let created_at = unix_timestamp()?;
        let record = MicroVmRecord {
            id: 0,
            name: vm_name.clone(),
            distribution_id: manifest.boot.distribution_id.clone(),
            image_id: manifest.boot.image_id.clone(),
            kernel_id: kernel.id.clone(),
            firecracker_package_id: firecracker.package_id.clone(),
            firectl_package_id: firectl.package_id.clone(),
            disk_size_bytes: manifest.vm.disk_size_bytes,
            memory_bytes: manifest.vm.memory_bytes,
            memory_effective_mib: manifest.vm.memory_effective_mib,
            vcpu_count: manifest.vm.vcpu_count,
            volume_path: volume_path.clone(),
            rootfs_path: rootfs_path.clone(),
            socket_path: socket_path.clone(),
            expose_on_lan,
            created_at,
        };
        let credential = PersistedCredential {
            private_key_path: private_key_path.clone(),
            public_key_path: public_key_path.clone(),
            guest_authorized_keys_path: "/root/.ssh/authorized_keys".to_owned(),
            key_type: manifest.ssh.key_type.clone(),
            ssh_user: manifest.ssh.user.clone(),
            ssh_port: manifest.ssh.port,
            public_key_fingerprint: manifest.ssh.public_key_fingerprint.clone(),
            file_mode: "0600".to_owned(),
        };
        let runtime_record = PersistedRuntime {
            firecracker_path: firecracker.path.clone(),
            firectl_path: firectl.path.clone(),
            socket_path: socket_path.clone(),
            process_id: None,
            process_state: "stopped".to_owned(),
        };
        let kernel_spec = kernel_download_spec(&home, &kernel, kernel_payload)?;
        let kernel_integrity = FileIntegrity {
            size_bytes: kernel_payload.size_bytes,
            sha256: kernel_payload.sha256.clone(),
        };
        let portable_metadata = RestoredSnapshotMetadata {
            distribution_name: manifest.boot.distribution_name.clone(),
            distribution_version: manifest.boot.distribution_version.clone(),
            root_device: manifest.boot.root_device.clone(),
            kernel_args: manifest.boot.kernel_args.clone(),
            image_sha256: manifest.boot.image_sha256.clone(),
            guest_architecture: manifest.compatibility.guest_architecture.clone(),
        };
        emit_progress(
            &callback,
            RestoreProgress {
                stage: RestoreProgressStage::Committing,
                completed_bytes: manifest.vm.disk_size_bytes,
                total_bytes: manifest.vm.disk_size_bytes,
            },
        );
        journal.progress_state = "committing".to_owned();
        repository.record_restore_journal(&journal)?;
        repository.commit_restored_microvm(RestoredMicroVmCommit {
            record: &record,
            network: &network,
            credential: &credential,
            runtime: &runtime_record,
            kernel: &kernel,
            kernel_spec: &kernel_spec,
            kernel_integrity: &kernel_integrity,
            snapshot_metadata: &portable_metadata,
            operation_id: &operation_id,
        })?;
        let ssh = SshConnectionInfo {
            user: manifest.ssh.user.clone(),
            port: manifest.ssh.port,
            address: network.config.guest_address,
            private_key_path,
            public_key_path,
        };
        let result = RestoreResult {
            vm_name: vm_name.clone(),
            state: MicroVmState::Stopped,
            volume_path: volume_path.clone(),
            rootfs_path,
            disk_size_bytes: manifest.vm.disk_size_bytes,
            memory_bytes: manifest.vm.memory_bytes,
            vcpu_count: manifest.vm.vcpu_count,
            kernel_path: kernel_path.clone(),
            network: network.config,
            ssh,
            address_policy: manifest.address_policy(),
        };
        emit_progress(
            &callback,
            RestoreProgress {
                stage: RestoreProgressStage::Completed,
                completed_bytes: manifest.vm.disk_size_bytes,
                total_bytes: manifest.vm.disk_size_bytes,
            },
        );
        Ok(result)
    })();
    match result {
        Ok(result) => Ok(result),
        Err(primary) => {
            let mut failures = Vec::new();
            if let Some(network) = installed_network.as_ref()
                && let Err(error) = network_controller.cleanup_for_delete(network)
            {
                failures.push(error.to_string());
            }
            if journal.volume_created
                && let Err(error) = remove_operation_directory(&home, &journal.volume_path, "vms")
            {
                failures.push(error.to_string());
            }
            if journal.kernel_created
                && let Err(error) =
                    remove_operation_file(&home, &journal.kernel_path, "artifacts/kernels")
            {
                failures.push(error.to_string());
            }
            if let Err(error) =
                remove_operation_directory(&home, &journal.staging_path, "tmp/restores")
                && !is_not_found(&error)
            {
                failures.push(error.to_string());
            }
            if failures.is_empty()
                && let Err(error) = repository.delete_restore_journal(&operation_id)
            {
                failures.push(error.to_string());
            }
            if failures.is_empty() {
                Err(primary)
            } else {
                Err(SdkError::Cleanup {
                    primary: primary.to_string(),
                    failures,
                })
            }
        }
    }
}

fn reconcile_restore_journals(
    home: &Path,
    repository: &dyn LocalRepository,
    network: &dyn NetworkController,
    vm_name: &str,
) -> Result<(), SdkError> {
    for journal in repository.list_restore_journals(vm_name)? {
        let recovery_error = |reason: String| SdkError::RestoreRecovery {
            operation_id: journal.operation_id.clone(),
            reason,
        };
        if let Some(persisted_network) = journal.network.as_ref() {
            network
                .cleanup_for_delete(persisted_network)
                .map_err(|error| recovery_error(error.to_string()))?;
        }
        if journal.volume_created {
            remove_operation_directory(home, &journal.volume_path, "vms")
                .map_err(|error| recovery_error(error.to_string()))?;
        }
        if journal.kernel_created {
            remove_operation_file(home, &journal.kernel_path, "artifacts/kernels")
                .map_err(|error| recovery_error(error.to_string()))?;
        }
        remove_operation_directory(home, &journal.staging_path, "tmp/restores")
            .map_err(|error| recovery_error(error.to_string()))?;
        repository
            .delete_restore_journal(&journal.operation_id)
            .map_err(|error| recovery_error(error.to_string()))?;
    }
    Ok(())
}

fn resolve_local_runtime(
    repository: &dyn LocalRepository,
    architecture: &Architecture,
) -> Result<(InstalledBinary, InstalledBinary), SdkError> {
    let binaries = repository.list_installed_binaries()?;
    let firecracker = binaries
        .iter()
        .find(|binary| {
            binary.component_name == "firecracker" && &binary.architecture == architecture
        })
        .cloned()
        .ok_or_else(|| SdkError::ArtifactPrerequisite {
            kind: "runtime binary".to_owned(),
            id: "firecracker".to_owned(),
            path: PathBuf::new(),
            reason: "a compatible Firecracker binary is not installed locally".to_owned(),
        })?;
    let firectl = binaries
        .iter()
        .find(|binary| binary.component_name == "firectl" && &binary.architecture == architecture)
        .cloned()
        .ok_or_else(|| SdkError::ArtifactPrerequisite {
            kind: "runtime binary".to_owned(),
            id: "firectl".to_owned(),
            path: PathBuf::new(),
            reason:
                "a compatible local firectl binary for the installed runtime package is unavailable"
                    .to_owned(),
        })?;
    Ok((firecracker, firectl))
}

fn kernel_from_manifest(manifest: &SnapshotManifest) -> Result<Kernel, SdkError> {
    let metadata = &manifest.boot.kernel;
    let payload = payload_record(manifest, KERNEL_MEMBER)?;
    Ok(Kernel {
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        display_name: metadata.display_name.clone(),
        version: metadata.version.clone(),
        architecture: parse_architecture(&metadata.architecture)?,
        path: metadata.registry_path.clone(),
        url: metadata.registry_url.clone(),
        filename: metadata.filename.clone(),
        size_bytes: payload.size_bytes,
        sha256: payload.sha256.clone(),
        format: metadata.format.clone(),
        mime_type: metadata.mime_type.clone(),
        elf: None,
        modified_at: metadata.modified_at.clone(),
    })
}

fn kernel_download_spec(
    home: &Path,
    kernel: &Kernel,
    payload: &crate::adapters::archive::manifest::PayloadRecord,
) -> Result<DownloadSpec, SdkError> {
    let relative_path = PathBuf::from("artifacts")
        .join("kernels")
        .join(&kernel.id)
        .join(&kernel.filename);
    Ok(DownloadSpec {
        artifact_kind: ArtifactKind::Kernel,
        artifact_id: kernel.id.clone(),
        member_name: None,
        artifact_key: format!("kernel:{}", kernel.id),
        registry_path: kernel.path.clone(),
        registry_url: kernel.url.clone(),
        filename: kernel.filename.clone(),
        expected_size: payload.size_bytes,
        expected_sha256: payload.sha256.clone(),
        executable: false,
        mode: None,
        absolute_path: home.join(&relative_path),
        relative_path,
    })
}

fn payload_record<'a>(
    manifest: &'a SnapshotManifest,
    member: &str,
) -> Result<&'a crate::adapters::archive::manifest::PayloadRecord, SdkError> {
    manifest
        .payloads
        .iter()
        .find(|payload| payload.path == member)
        .ok_or_else(|| SdkError::InvalidSnapshotManifest {
            reason: format!("payload record {member} is missing"),
        })
}

fn verify_network_outcome(
    manifest: &SnapshotManifest,
    network: &PersistedNetwork,
) -> Result<(), SdkError> {
    if let NetworkManifest::PreserveIpv4 {
        guest_ipv4,
        prefix_length,
        guest_gateway_ipv4,
        lan_ipv4,
        guest_mac,
        mode,
        ..
    } = &manifest.network
    {
        let actual_guest = match network.config.guest_address {
            IpAddr::V4(value) => value,
            IpAddr::V6(value) => {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "guest IPv4".to_owned(),
                    value: value.to_string(),
                });
            }
        };
        let actual_gateway = match network.config.gateway {
            Some(IpAddr::V4(value)) => Some(value),
            Some(IpAddr::V6(value)) => {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "gateway IPv4".to_owned(),
                    value: value.to_string(),
                });
            }
            None => None,
        };
        let actual_lan = match network.config.lan_address {
            Some(IpAddr::V4(value)) => Some(value),
            Some(IpAddr::V6(value)) => {
                return Err(SdkError::SnapshotNetworkConflict {
                    field: "LAN IPv4".to_owned(),
                    value: value.to_string(),
                });
            }
            None => None,
        };
        if actual_guest != *guest_ipv4
            || network.config.prefix_length != *prefix_length
            || actual_gateway != *guest_gateway_ipv4
            || actual_lan != *lan_ipv4
            || network.guest_mac != *guest_mac
            || network.config.mode.to_string() != *mode
        {
            return Err(SdkError::SnapshotNetworkConflict {
                field: "preserved IPv4 network identity".to_owned(),
                value: "the destination adapter did not reproduce every archived value".to_owned(),
            });
        }
    }
    Ok(())
}

fn parse_architecture(value: &str) -> Result<Architecture, SdkError> {
    match value {
        "x86_64" => Ok(Architecture::X86_64),
        "aarch64" => Ok(Architecture::Aarch64),
        "arm" => Ok(Architecture::Arm),
        "riscv64" => Ok(Architecture::Riscv64),
        "x86" => Ok(Architecture::X86),
        _ => Err(SdkError::IncompatibleArtifact {
            artifact: "kernel".to_owned(),
            reason: format!("unsupported guest architecture {value}"),
        }),
    }
}

fn architecture_name(value: &Architecture) -> &'static str {
    match value {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::Riscv64 => "riscv64",
        Architecture::X86 => "x86",
    }
}

fn normalize_archive_path(path: &Path) -> Result<PathBuf, SdkError> {
    if path.as_os_str().is_empty() {
        return Err(SdkError::InvalidRequest {
            field: "archive_path".to_owned(),
            reason: "must name a snapshot archive".to_owned(),
        });
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| SdkError::filesystem("resolve snapshot archive path", path, error))
    }
}

fn new_restore_operation_id() -> Result<String, SdkError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            SdkError::Migration(format!("system clock is before Unix epoch: {error}"))
        })?
        .as_nanos();
    let counter = RESTORE_OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "restore-{}-{timestamp}-{counter}",
        std::process::id()
    ))
}

fn emit_progress(callback: &ProgressCallback, progress: RestoreProgress) {
    if let Ok(mut callback) = callback.lock() {
        callback(progress);
    }
}

fn ensure_directory(path: &Path, mode: u32) -> Result<(), SdkError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(SdkError::RestoreConflict {
                name: "restore".to_owned(),
                resource: "destination directory".to_owned(),
                value: format!("{} is not a real directory", path.display()),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .map_err(|source| SdkError::filesystem("create restore directory", path, source))?;
            set_mode(path, mode)
        }
        Err(error) => Err(SdkError::filesystem(
            "inspect restore directory",
            path,
            error,
        )),
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| SdkError::filesystem("set restore file permissions", path, error))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn path_entry_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem(
            "inspect restore destination",
            path,
            error,
        )),
    }
}

fn copy_payload(source: &Path, destination: &Path, mode: u32) -> Result<(), SdkError> {
    verify_regular_file(source, "verify staged restore payload")?;
    let mut input = File::open(source)
        .map_err(|error| SdkError::filesystem("open staged restore payload", source, error))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut output = options
        .open(destination)
        .map_err(|error| SdkError::filesystem("create restored payload", destination, error))?;
    let mut buffer = [0_u8; RESTORE_COPY_BUFFER_BYTES];
    let result = (|| {
        loop {
            let read = input.read(&mut buffer).map_err(|error| {
                SdkError::filesystem("read staged restore payload", source, error)
            })?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                SdkError::filesystem("write restored payload", destination, error)
            })?;
        }
        output
            .sync_all()
            .map_err(|error| SdkError::filesystem("sync restored payload", destination, error))
    })();
    if let Err(error) = result {
        drop(output);
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    set_mode(destination, mode)
}

fn verify_file_integrity(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), SdkError> {
    verify_file_integrity_with_progress(path, expected_size, expected_sha256, None, &mut |_, _| {})
}

fn verify_file_integrity_with_progress(
    path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    cancellation: Option<&CancellationToken>,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), SdkError> {
    let mut file = File::open(path).map_err(|error| {
        SdkError::filesystem("open restored payload for verification", path, error)
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| SdkError::filesystem("inspect restored payload", path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; RESTORE_COPY_BUFFER_BYTES];
    let mut size_bytes = 0_u64;
    let mut last_reported_percent = 0_u64;
    on_progress(0, expected_size);
    loop {
        if cancellation.is_some_and(|token| token.is_cancelled()) {
            return Err(SdkError::RestoreCancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| SdkError::filesystem("verify restored payload", path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size_bytes = size_bytes.saturating_add(read as u64);
        if expected_size > 0 {
            let percent = ((u128::from(size_bytes) * 100) / u128::from(expected_size)) as u64;
            let report_percent = percent.min(99);
            if report_percent > last_reported_percent {
                on_progress(size_bytes.min(expected_size - 1), expected_size);
                last_reported_percent = report_percent;
            }
        }
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !metadata.is_file() || size_bytes != expected_size || sha256 != expected_sha256 {
        return Err(SdkError::IntegrityMismatch {
            artifact: path.display().to_string(),
            expected_size,
            actual_size: size_bytes,
            expected_sha256: expected_sha256.to_owned(),
            actual_sha256: sha256,
        });
    }
    on_progress(expected_size, expected_size);
    Ok(())
}

fn verify_regular_file(path: &Path, operation: &'static str) -> Result<(), SdkError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| SdkError::filesystem(operation, path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SdkError::RestoreConflict {
            name: "restore".to_owned(),
            resource: "managed file".to_owned(),
            value: format!("{} is not a regular file", path.display()),
        });
    }
    Ok(())
}

fn remove_operation_directory(
    home: &Path,
    path: &Path,
    relative_root: &str,
) -> Result<(), SdkError> {
    let expected_root = home.join(relative_root);
    if !path.starts_with(&expected_root) || path == expected_root {
        return Err(SdkError::RestoreRecovery {
            operation_id: "unknown".to_owned(),
            reason: format!(
                "refusing to remove path outside {}",
                expected_root.display()
            ),
        });
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(SdkError::RestoreRecovery {
                operation_id: "unknown".to_owned(),
                reason: format!(
                    "refusing to remove non-directory restore path {}",
                    path.display()
                ),
            })
        }
        Ok(_) => fs::remove_dir_all(path).map_err(|error| {
            SdkError::filesystem("remove operation-owned restore directory", path, error)
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SdkError::filesystem(
            "inspect restore cleanup directory",
            path,
            error,
        )),
    }
}

fn remove_operation_file(home: &Path, path: &Path, relative_root: &str) -> Result<(), SdkError> {
    let expected_root = home.join(relative_root);
    if !path.starts_with(&expected_root) || path == expected_root {
        return Err(SdkError::RestoreRecovery {
            operation_id: "unknown".to_owned(),
            reason: format!(
                "refusing to remove path outside {}",
                expected_root.display()
            ),
        });
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(SdkError::RestoreRecovery {
                operation_id: "unknown".to_owned(),
                reason: format!(
                    "refusing to remove non-regular restore path {}",
                    path.display()
                ),
            })
        }
        Ok(_) => fs::remove_file(path).map_err(|error| {
            SdkError::filesystem("remove operation-owned restore file", path, error)
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SdkError::filesystem(
            "inspect restore cleanup file",
            path,
            error,
        )),
    }
}

fn is_not_found(error: &SdkError) -> bool {
    matches!(error, SdkError::Filesystem { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
}
