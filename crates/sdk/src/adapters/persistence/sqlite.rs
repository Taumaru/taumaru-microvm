use std::convert::TryFrom;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity, InstalledBinary};
use crate::domain::autostart::{AutostartPolicy, AutostartSettings};
use crate::domain::lifecycle::NetworkMode;
use crate::domain::microvm::{
    MicroVmRecord, NetworkConfiguration, NetworkResource, PersistedCredential, PersistedNetwork,
    PersistedNetworkResource, PersistedRuntime,
};
use crate::domain::registry::{
    Architecture, BinaryFile, BinaryPackage, Distribution, DistributionImage, ElfMetadata,
    Endianness, Kernel, Linkage,
};
use crate::error::SdkError;
use crate::ports::repository::{
    ArtifactRepository, InventoryState, LocalArtifact, MicroVmRepository, OrphanArtifactDownload,
    OrphanIdentity, PrunableImage, PrunableKernel, PruneReferences, RestoreJournal,
    RestoredMicroVmCommit, SnapshotStoredMetadata, StoredMicroVm,
};

use super::migrations;

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// SQLite-backed host-local artifact inventory.
#[derive(Clone, Debug)]
pub(crate) struct SqliteRepository {
    database_path: PathBuf,
}

impl SqliteRepository {
    pub(crate) fn initialize(database_path: PathBuf) -> Result<Self, SdkError> {
        let mut connection = open_connection(&database_path)?;
        migrations::apply_pending(&mut connection)?;
        Ok(Self { database_path })
    }

    fn connection(&self) -> Result<Connection, SdkError> {
        open_connection(&self.database_path)
    }

    fn list_network_addresses(
        &self,
        query: &str,
        kind: &str,
    ) -> Result<Vec<(String, IpAddr, String)>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (name, address, tap) = row?;
            let address = address.parse::<IpAddr>().map_err(|error| {
                SdkError::Migration(format!("invalid persisted {kind} for {name}: {error}"))
            })?;
            Ok((name, address, tap))
        })
        .collect()
    }
}

impl ArtifactRepository for SqliteRepository {
    fn inspect_member(
        &self,
        spec: &DownloadSpec,
        integrity: Option<&FileIntegrity>,
    ) -> Result<InventoryState, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT id, artifact_type, registry_path, registry_url, filename,
                        relative_path, absolute_path, expected_size_bytes, expected_sha256,
                        actual_size_bytes, actual_sha256, verification_status
                 FROM downloads WHERE artifact_key = ?1",
                params![spec.artifact_key],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            download_id,
            artifact_type,
            registry_path,
            registry_url,
            filename,
            relative_path,
            absolute_path,
            expected_size,
            expected_sha256,
            actual_size,
            actual_sha256,
            status,
        )) = row
        else {
            return Ok(InventoryState::Missing);
        };

        let expected_size = u64::try_from(expected_size).map_err(|_| {
            SdkError::Migration(format!(
                "negative expected size stored for {}",
                spec.artifact_key
            ))
        })?;
        let actual_size = u64::try_from(actual_size).map_err(|_| {
            SdkError::Migration(format!(
                "negative actual size stored for {}",
                spec.artifact_key
            ))
        })?;
        let expected_relative_path = spec.relative_path.to_string_lossy();
        let expected_absolute_path = spec.absolute_path.to_string_lossy();
        let inventory_match = artifact_type == artifact_type_name(&spec.artifact_kind)
            && registry_path == spec.registry_path
            && registry_url == spec.registry_url
            && filename == spec.filename
            && relative_path == expected_relative_path
            && absolute_path == expected_absolute_path
            && expected_size == spec.expected_size
            && expected_sha256 == spec.expected_sha256
            && actual_size == expected_size
            && actual_sha256 == expected_sha256
            && status == "verified";
        if integrity.is_none() {
            if !inventory_match {
                return Ok(InventoryState::Missing);
            }
            return if member_relation_exists(&connection, spec, download_id)? {
                Ok(InventoryState::Complete)
            } else {
                Ok(InventoryState::Incomplete)
            };
        }
        let physical_match = inventory_match
            && integrity.is_some_and(|value| {
                value.size_bytes == spec.expected_size
                    && value.sha256 == spec.expected_sha256
                    && value.size_bytes == expected_size
                    && value.sha256 == expected_sha256
                    && actual_size == value.size_bytes
                    && actual_sha256 == value.sha256
            });

        if !physical_match {
            return Ok(InventoryState::Missing);
        }

        if member_relation_exists(&connection, spec, download_id)? {
            Ok(InventoryState::Complete)
        } else {
            Ok(InventoryState::Incomplete)
        }
    }

    fn remove_member(&self, spec: &DownloadSpec) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;

        match &spec.artifact_kind {
            ArtifactKind::Kernel => {
                let kernel = transaction
                    .query_row(
                        "SELECT id, download_id FROM kernels WHERE registry_id = ?1",
                        params![spec.artifact_id],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
                    )
                    .optional()?;

                if let Some((kernel_id, Some(download_id))) = kernel {
                    transaction.execute(
                        "DELETE FROM distribution_kernels WHERE kernel_id = ?1",
                        params![kernel_id],
                    )?;
                    transaction.execute("DELETE FROM kernels WHERE id = ?1", params![kernel_id])?;
                    transaction
                        .execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
                } else {
                    transaction.execute(
                        "DELETE FROM downloads WHERE artifact_key = ?1",
                        params![spec.artifact_key],
                    )?;
                }
            }
            ArtifactKind::Binary => {
                let component = spec.member_name.as_deref().ok_or_else(|| {
                    SdkError::invalid_metadata(&spec.artifact_key, "binary component is missing")
                })?;
                let binary_file = transaction
                    .query_row(
                        "SELECT bf.id, bf.download_id
                         FROM binary_files bf
                         JOIN binary_packages bp ON bp.id = bf.binary_package_id
                         WHERE bp.registry_id = ?1 AND bf.component_name = ?2",
                        params![spec.artifact_id, component],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                if let Some((binary_file_id, download_id)) = binary_file {
                    transaction.execute(
                        "DELETE FROM binary_files WHERE id = ?1",
                        params![binary_file_id],
                    )?;
                    transaction
                        .execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
                } else {
                    transaction.execute(
                        "DELETE FROM downloads WHERE artifact_key = ?1",
                        params![spec.artifact_key],
                    )?;
                }
            }
            ArtifactKind::DistributionImage => {
                let image = spec.member_name.as_deref().ok_or_else(|| {
                    SdkError::invalid_metadata(&spec.artifact_key, "distribution image is missing")
                })?;
                let distribution_image = transaction
                    .query_row(
                        "SELECT di.id, di.download_id
                         FROM distribution_images di
                         JOIN distributions d ON d.id = di.distribution_id
                         WHERE d.registry_id = ?1 AND di.registry_id = ?2",
                        params![spec.artifact_id, image],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                if let Some((image_id, download_id)) = distribution_image {
                    transaction.execute(
                        "DELETE FROM distribution_images WHERE id = ?1",
                        params![image_id],
                    )?;
                    transaction
                        .execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
                } else {
                    transaction.execute(
                        "DELETE FROM downloads WHERE artifact_key = ?1",
                        params![spec.artifact_key],
                    )?;
                }
            }
        }

        transaction.commit()?;
        Ok(())
    }

    fn persist_kernel(
        &self,
        kernel: &Kernel,
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let download_id = persist_download(&transaction, spec, integrity)?;
        let now = unix_timestamp()?;
        transaction.execute(
            "INSERT INTO kernels (
                registry_id, download_id, name, display_name, version, architecture,
                registry_path, registry_url, filename, size_bytes, sha256, format,
                mime_type, modified_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
            ON CONFLICT(registry_id) DO UPDATE SET
                download_id = excluded.download_id,
                name = excluded.name,
                display_name = excluded.display_name,
                version = excluded.version,
                architecture = excluded.architecture,
                registry_path = excluded.registry_path,
                registry_url = excluded.registry_url,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                sha256 = excluded.sha256,
                format = excluded.format,
                mime_type = excluded.mime_type,
                modified_at = excluded.modified_at,
                updated_at = excluded.updated_at",
            params![
                kernel.id,
                download_id,
                kernel.name,
                kernel.display_name,
                kernel.version,
                architecture_name(&kernel.architecture),
                kernel.path,
                kernel.url,
                kernel.filename,
                to_sqlite_integer(kernel.size_bytes, &spec.artifact_key)?,
                kernel.sha256,
                kernel.format,
                kernel.mime_type,
                kernel.modified_at,
                now,
            ],
        )?;
        persist_elf(&transaction, download_id, kernel.elf.as_ref())?;
        transaction.commit()?;
        Ok(())
    }

    fn persist_binary_file(
        &self,
        package: &BinaryPackage,
        file: &BinaryFile,
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = unix_timestamp()?;
        transaction.execute(
            "INSERT INTO binary_packages (
                registry_id, name, display_name, description, version, architecture,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(registry_id) DO UPDATE SET
                name = excluded.name,
                display_name = excluded.display_name,
                description = excluded.description,
                version = excluded.version,
                architecture = excluded.architecture,
                updated_at = excluded.updated_at",
            params![
                package.id,
                package.name,
                package.display_name,
                package.description,
                package.version,
                architecture_name(&package.architecture),
                now,
            ],
        )?;
        let package_id: i64 = transaction.query_row(
            "SELECT id FROM binary_packages WHERE registry_id = ?1",
            params![package.id],
            |row| row.get(0),
        )?;
        let download_id = persist_download(&transaction, spec, integrity)?;
        transaction.execute(
            "INSERT INTO binary_files (
                binary_package_id, download_id, component_name, registry_path, registry_url,
                filename, size_bytes, sha256, mime_type, executable, mode, permissions,
                format, modified_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
            ON CONFLICT(binary_package_id, component_name) DO UPDATE SET
                download_id = excluded.download_id,
                registry_path = excluded.registry_path,
                registry_url = excluded.registry_url,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                sha256 = excluded.sha256,
                mime_type = excluded.mime_type,
                executable = excluded.executable,
                mode = excluded.mode,
                permissions = excluded.permissions,
                format = excluded.format,
                modified_at = excluded.modified_at,
                updated_at = excluded.updated_at",
            params![
                package_id,
                download_id,
                file.name,
                file.path,
                file.url,
                file.filename,
                to_sqlite_integer(file.size_bytes, &spec.artifact_key)?,
                file.sha256,
                file.mime_type,
                bool_to_sqlite(file.executable),
                file.mode,
                file.permissions,
                file.format,
                file.modified_at,
                now,
            ],
        )?;
        persist_elf(&transaction, download_id, file.elf.as_ref())?;
        transaction.commit()?;
        Ok(())
    }

    fn persist_distribution_image(
        &self,
        distribution: &Distribution,
        image: &DistributionImage,
        kernels: &[Kernel],
        spec: &DownloadSpec,
        integrity: &FileIntegrity,
    ) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = unix_timestamp()?;
        transaction.execute(
            "INSERT INTO distributions (
                registry_id, name, display_name, description, distribution, version, codename,
                architecture, vendor, homepage, default_kernel_registry_id, root_device,
                min_memory_mb, min_vcpus, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
            ON CONFLICT(registry_id) DO UPDATE SET
                name = excluded.name,
                display_name = excluded.display_name,
                description = excluded.description,
                distribution = excluded.distribution,
                version = excluded.version,
                codename = excluded.codename,
                architecture = excluded.architecture,
                vendor = excluded.vendor,
                homepage = excluded.homepage,
                default_kernel_registry_id = excluded.default_kernel_registry_id,
                root_device = excluded.root_device,
                min_memory_mb = excluded.min_memory_mb,
                min_vcpus = excluded.min_vcpus,
                updated_at = excluded.updated_at",
            params![
                distribution.id,
                distribution.name,
                distribution.display_name,
                distribution.description,
                distribution.distribution,
                distribution.version,
                distribution.codename,
                architecture_name(&distribution.architecture),
                distribution.vendor,
                distribution.homepage,
                distribution.default_kernel,
                distribution.boot.root_device,
                to_sqlite_integer(distribution.requirements.min_memory_mb, &distribution.id)?,
                i64::from(distribution.requirements.min_vcpus),
                now,
            ],
        )?;
        let distribution_id: i64 = transaction.query_row(
            "SELECT id FROM distributions WHERE registry_id = ?1",
            params![distribution.id],
            |row| row.get(0),
        )?;
        replace_boot_args(
            &transaction,
            distribution_id,
            &distribution.boot.kernel_args,
        )?;
        persist_kernel_references(&transaction, distribution_id, distribution, kernels, now)?;

        let download_id = persist_download(&transaction, spec, integrity)?;
        transaction.execute(
            "INSERT INTO distribution_images (
                distribution_id, download_id, registry_id, name, display_name, description,
                variant, format, registry_path, registry_url, filename, size_bytes, sha256,
                mime_type, modified_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
            ON CONFLICT(distribution_id, registry_id) DO UPDATE SET
                download_id = excluded.download_id,
                name = excluded.name,
                display_name = excluded.display_name,
                description = excluded.description,
                variant = excluded.variant,
                format = excluded.format,
                registry_path = excluded.registry_path,
                registry_url = excluded.registry_url,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                sha256 = excluded.sha256,
                mime_type = excluded.mime_type,
                modified_at = excluded.modified_at,
                updated_at = excluded.updated_at",
            params![
                distribution_id,
                download_id,
                image.id,
                image.name,
                image.display_name,
                image.description,
                image.variant,
                image.format,
                image.path,
                image.url,
                image.filename,
                to_sqlite_integer(image.size_bytes, &spec.artifact_key)?,
                image.sha256,
                image.mime_type,
                image.modified_at,
                now,
            ],
        )?;
        let image_id: i64 = transaction.query_row(
            "SELECT id FROM distribution_images WHERE distribution_id = ?1 AND registry_id = ?2",
            params![distribution_id, image.id],
            |row| row.get(0),
        )?;
        replace_filesystem(&transaction, image_id, &image.filesystem)?;
        replace_capabilities(&transaction, image_id, &image.capabilities)?;
        transaction.commit()?;
        Ok(())
    }

    fn resolve_binary(
        &self,
        package_id: &str,
        component_name: &str,
    ) -> Result<InstalledBinary, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT bp.registry_id, bf.component_name, bp.version, bp.architecture,
                        d.absolute_path, d.actual_size_bytes, d.actual_sha256, bf.executable,
                        d.expected_size_bytes, d.expected_sha256, d.verification_status
                 FROM binary_packages bp
                 JOIN binary_files bf ON bf.binary_package_id = bp.id
                 JOIN downloads d ON d.id = bf.download_id
                 WHERE bp.registry_id = ?1 AND bf.component_name = ?2",
                params![package_id, component_name],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        PathBuf::from(row.get::<_, String>(4)?),
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            package_id,
            component_name,
            version,
            architecture,
            path,
            actual_size,
            actual_sha256,
            executable,
            expected_size,
            expected_sha256,
            status,
        )) = row
        else {
            return Err(SdkError::NotFound {
                kind: "binary".to_owned(),
                id: format!("{package_id}/{component_name}"),
            });
        };

        let actual_size = u64::try_from(actual_size)
            .map_err(|_| SdkError::Migration("negative binary size in inventory".to_owned()))?;
        let expected_size = u64::try_from(expected_size).map_err(|_| {
            SdkError::Migration("negative expected binary size in inventory".to_owned())
        })?;
        if status != "verified" || actual_size != expected_size || actual_sha256 != expected_sha256
        {
            return Err(SdkError::StaleBinary {
                package_id,
                component_name,
                path,
            });
        }

        Ok(InstalledBinary {
            package_id,
            component_name,
            version,
            architecture: parse_architecture(&architecture)?,
            path,
            size_bytes: actual_size,
            sha256: actual_sha256,
            executable: executable != 0,
        })
    }

    fn resolve_kernel(&self, kernel_id: &str) -> Result<LocalArtifact, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT d.absolute_path, d.actual_size_bytes, d.actual_sha256,
                        d.expected_size_bytes, d.expected_sha256, d.verification_status
                 FROM kernels k
                 JOIN downloads d ON d.id = k.download_id
                 WHERE k.registry_id = ?1 AND k.download_id IS NOT NULL",
                params![kernel_id],
                |row| {
                    Ok((
                        PathBuf::from(row.get::<_, String>(0)?),
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        local_artifact_from_row(row, "kernel", kernel_id)
    }

    fn resolve_distribution_image(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<LocalArtifact, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT d.absolute_path, d.actual_size_bytes, d.actual_sha256,
                        d.expected_size_bytes, d.expected_sha256, d.verification_status
                 FROM distribution_images di
                 JOIN distributions dist ON dist.id = di.distribution_id
                 JOIN downloads d ON d.id = di.download_id
                 WHERE dist.registry_id = ?1 AND di.registry_id = ?2
                   AND di.download_id IS NOT NULL",
                params![distribution_id, image_id],
                |row| {
                    Ok((
                        PathBuf::from(row.get::<_, String>(0)?),
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        local_artifact_from_row(row, "distribution image", image_id)
    }

    fn list_ready_distribution_images(&self) -> Result<Vec<(String, String)>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT dist.registry_id, di.registry_id
             FROM distribution_images di
             JOIN distributions dist ON dist.id = di.distribution_id
             JOIN downloads d ON d.id = di.download_id
             WHERE d.verification_status = 'verified'
               AND d.actual_size_bytes = d.expected_size_bytes
               AND d.actual_sha256 = d.expected_sha256",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(SdkError::from)
    }

    fn list_installed_binaries(&self) -> Result<Vec<InstalledBinary>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT bp.registry_id, bf.component_name, bp.version, bp.architecture,
                    d.absolute_path, d.actual_size_bytes, d.actual_sha256, bf.executable,
                    d.expected_size_bytes, d.expected_sha256, d.verification_status
             FROM binary_packages bp
             JOIN binary_files bf ON bf.binary_package_id = bp.id
             JOIN downloads d ON d.id = bf.download_id
             ORDER BY bp.registry_id, bf.component_name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                PathBuf::from(row.get::<_, String>(4)?),
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
            ))
        })?;
        rows.map(|row| {
            let (
                package_id,
                component_name,
                version,
                architecture,
                path,
                actual_size,
                actual_sha256,
                executable,
                expected_size,
                expected_sha256,
                status,
            ) = row?;
            let actual_size = u64::try_from(actual_size)
                .map_err(|_| SdkError::Migration("negative binary size in inventory".to_owned()))?;
            let expected_size = u64::try_from(expected_size).map_err(|_| {
                SdkError::Migration("negative expected binary size in inventory".to_owned())
            })?;
            if status != "verified"
                || actual_size != expected_size
                || actual_sha256 != expected_sha256
            {
                return Err(SdkError::StaleBinary {
                    package_id,
                    component_name,
                    path,
                });
            }
            Ok(InstalledBinary {
                package_id,
                component_name,
                version,
                architecture: parse_architecture(&architecture)?,
                path,
                size_bytes: actual_size,
                sha256: actual_sha256,
                executable: executable != 0,
            })
        })
        .collect()
    }

    fn list_prune_references(&self) -> Result<PruneReferences, SdkError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT distribution_id, image_id, kernel_id FROM microvms")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut references = PruneReferences::default();
        for row in rows {
            let (distribution_id, image_id, kernel_id) = row?;
            references.kernels.insert(kernel_id);
            references.images.insert((distribution_id, image_id));
        }
        Ok(references)
    }

    fn list_prunable_kernels(&self) -> Result<Vec<PrunableKernel>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT k.registry_id, d.absolute_path, d.actual_size_bytes
             FROM kernels k
             JOIN downloads d ON d.id = k.download_id
             WHERE k.download_id IS NOT NULL
             ORDER BY k.registry_id",
        )?;
        let rows = statement.query_map([], |row| {
            let size: i64 = row.get(2)?;
            Ok(PrunableKernel {
                registry_id: row.get(0)?,
                absolute_path: PathBuf::from(row.get::<_, String>(1)?),
                size_bytes: u64::try_from(size).unwrap_or(0),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(SdkError::from)
    }

    fn list_prunable_images(&self) -> Result<Vec<PrunableImage>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT dist.registry_id, di.registry_id, d.absolute_path, d.actual_size_bytes
             FROM distribution_images di
             JOIN distributions dist ON dist.id = di.distribution_id
             JOIN downloads d ON d.id = di.download_id
             ORDER BY dist.registry_id, di.registry_id",
        )?;
        let rows = statement.query_map([], |row| {
            let size: i64 = row.get(3)?;
            Ok(PrunableImage {
                distribution_id: row.get(0)?,
                image_id: row.get(1)?,
                absolute_path: PathBuf::from(row.get::<_, String>(2)?),
                size_bytes: u64::try_from(size).unwrap_or(0),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(SdkError::from)
    }

    fn list_orphan_artifact_downloads(&self) -> Result<Vec<OrphanArtifactDownload>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT d.artifact_key, d.artifact_type, d.absolute_path, d.actual_size_bytes
             FROM downloads d
             WHERE d.artifact_type IN ('kernel', 'distribution_image')
               AND NOT EXISTS (
                    SELECT 1 FROM kernels k WHERE k.download_id = d.id
               )
               AND NOT EXISTS (
                    SELECT 1 FROM distribution_images di WHERE di.download_id = d.id
               )
             ORDER BY d.artifact_key",
        )?;
        let rows = statement.query_map([], |row| {
            let size: i64 = row.get(3)?;
            Ok(OrphanArtifactDownload {
                artifact_key: row.get(0)?,
                artifact_type: row.get(1)?,
                absolute_path: PathBuf::from(row.get::<_, String>(2)?),
                size_bytes: u64::try_from(size).unwrap_or(0),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(SdkError::from)
    }

    fn delete_kernel_if_unreferenced(&self, kernel_id: &str) -> Result<bool, SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        if kernel_is_referenced(&transaction, kernel_id)? {
            return Ok(false);
        }
        let kernel: Option<(i64, Option<i64>)> = transaction
            .query_row(
                "SELECT id, download_id FROM kernels WHERE registry_id = ?1",
                params![kernel_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((kernel_row_id, Some(download_id))) = kernel else {
            return Ok(false);
        };
        transaction.execute(
            "DELETE FROM distribution_kernels WHERE kernel_id = ?1",
            params![kernel_row_id],
        )?;
        transaction.execute("DELETE FROM kernels WHERE id = ?1", params![kernel_row_id])?;
        transaction.execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
        transaction.commit()?;
        Ok(true)
    }

    fn delete_image_if_unreferenced(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<bool, SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        if image_is_referenced(&transaction, distribution_id, image_id)? {
            return Ok(false);
        }
        let image: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT di.id, di.download_id
                 FROM distribution_images di
                 JOIN distributions d ON d.id = di.distribution_id
                 WHERE d.registry_id = ?1 AND di.registry_id = ?2",
                params![distribution_id, image_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((image_row_id, download_id)) = image else {
            return Ok(false);
        };
        transaction.execute(
            "DELETE FROM distribution_images WHERE id = ?1",
            params![image_row_id],
        )?;
        transaction.execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
        transaction.commit()?;
        Ok(true)
    }

    fn delete_orphan_download_if_unreferenced(&self, artifact_key: &str) -> Result<bool, SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let orphan: Option<(i64, String, String, i64)> = transaction
            .query_row(
                "SELECT d.id, d.artifact_type, d.absolute_path,
                        (SELECT COUNT(*) FROM kernels k WHERE k.download_id = d.id)
                          + (SELECT COUNT(*) FROM distribution_images di
                             WHERE di.download_id = d.id)
                 FROM downloads d
                 WHERE d.artifact_key = ?1
                   AND d.artifact_type IN ('kernel', 'distribution_image')",
                params![artifact_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((download_id, artifact_type, absolute_path, member_count)) = orphan else {
            return Ok(false);
        };
        if member_count != 0 {
            return Ok(false);
        }
        let orphan = OrphanArtifactDownload {
            artifact_key: artifact_key.to_owned(),
            artifact_type,
            absolute_path: PathBuf::from(absolute_path),
            size_bytes: 0,
        };
        let Some(identity) = orphan.parse_identity() else {
            return Ok(false);
        };
        match identity {
            OrphanIdentity::Kernel { kernel_id } => {
                if kernel_is_referenced(&transaction, &kernel_id)? {
                    return Ok(false);
                }
            }
            OrphanIdentity::DistributionImage {
                distribution_id,
                image_id,
            } => {
                if image_is_referenced(&transaction, &distribution_id, &image_id)? {
                    return Ok(false);
                }
            }
        }
        transaction.execute("DELETE FROM downloads WHERE id = ?1", params![download_id])?;
        transaction.commit()?;
        Ok(true)
    }
}

impl MicroVmRepository for SqliteRepository {
    fn snapshot_metadata(&self, name: &str) -> Result<SnapshotStoredMetadata, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT COALESCE(sm.distribution_name, d.display_name),
                        COALESCE(sm.distribution_version, d.version),
                        COALESCE(sm.root_device, d.root_device),
                        COALESCE(sm.image_sha256, di.sha256),
                        COALESCE(sm.guest_architecture, k.architecture),
                        sm.kernel_args_json,
                        k.registry_id, k.name, k.display_name, k.version, k.architecture,
                        k.registry_path, k.registry_url, k.filename, k.size_bytes,
                        k.sha256, k.format, k.mime_type, k.modified_at
                 FROM microvms m
                 LEFT JOIN distributions d ON d.registry_id = m.distribution_id
                 LEFT JOIN distribution_images di
                   ON di.distribution_id = d.id AND di.registry_id = m.image_id
                 LEFT JOIN vm_snapshot_metadata sm ON sm.microvm_id = m.id
                 JOIN kernels k ON k.registry_id = m.kernel_id
                 WHERE m.name = ?1",
                params![name],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, i64>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, String>(16)?,
                        row.get::<_, String>(17)?,
                        row.get::<_, String>(18)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| SdkError::InvalidMetadata {
                artifact: name.to_owned(),
                reason: "stored VM provenance or embedded kernel inventory is incomplete"
                    .to_owned(),
            })?;
        let distribution_name = row.0.ok_or_else(|| incomplete_snapshot_metadata(name))?;
        let distribution_version = row.1.ok_or_else(|| incomplete_snapshot_metadata(name))?;
        let root_device = row.2.ok_or_else(|| incomplete_snapshot_metadata(name))?;
        let image_sha256 = row.3.ok_or_else(|| incomplete_snapshot_metadata(name))?;
        let kernel_args = if let Some(encoded) = row.5 {
            serde_json::from_str(&encoded).map_err(|error| SdkError::InvalidMetadata {
                artifact: name.to_owned(),
                reason: format!("stored boot arguments are invalid: {error}"),
            })?
        } else {
            let mut statement = connection.prepare(
                "SELECT argument FROM distribution_boot_args
                 WHERE distribution_id = (SELECT id FROM distributions WHERE registry_id =
                     (SELECT distribution_id FROM microvms WHERE name = ?1))
                 ORDER BY position",
            )?;
            statement
                .query_map(params![name], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let kernel_size = u64::try_from(row.14)
            .map_err(|_| SdkError::Migration("negative kernel size in inventory".to_owned()))?;
        let kernel = Kernel {
            id: row.6,
            name: row.7,
            display_name: row.8,
            version: row.9,
            architecture: parse_architecture(&row.10)?,
            path: row.11,
            url: row.12,
            filename: row.13,
            size_bytes: kernel_size,
            sha256: row.15,
            format: row.16,
            mime_type: row.17,
            elf: None,
            modified_at: row.18,
        };
        Ok(SnapshotStoredMetadata {
            distribution_name,
            distribution_version,
            root_device,
            kernel_args,
            image_sha256,
            guest_architecture: row.4,
            kernel,
        })
    }

    fn record_restore_journal(&self, journal: &RestoreJournal) -> Result<(), SdkError> {
        let connection = self.connection()?;
        let network_json = journal
            .network
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| {
                SdkError::Migration(format!("encode restore journal network: {error}"))
            })?;
        connection.execute(
            "INSERT INTO restore_journal (
                operation_id, vm_name, staging_path, volume_path, volume_created, kernel_path,
                kernel_created, network_json, progress_state, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(operation_id) DO UPDATE SET
                vm_name = excluded.vm_name,
                staging_path = excluded.staging_path,
                volume_path = excluded.volume_path,
                volume_created = excluded.volume_created,
                kernel_path = excluded.kernel_path,
                kernel_created = excluded.kernel_created,
                network_json = excluded.network_json,
                progress_state = excluded.progress_state,
                updated_at = excluded.updated_at",
            params![
                journal.operation_id,
                journal.vm_name,
                journal.staging_path.to_string_lossy().into_owned(),
                journal.volume_path.to_string_lossy().into_owned(),
                bool_to_sqlite(journal.volume_created),
                journal.kernel_path.to_string_lossy().into_owned(),
                bool_to_sqlite(journal.kernel_created),
                network_json,
                journal.progress_state,
                unix_timestamp()?,
            ],
        )?;
        Ok(())
    }

    fn list_restore_journals(&self, vm_name: &str) -> Result<Vec<RestoreJournal>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT operation_id, vm_name, staging_path, volume_path, volume_created, kernel_path,
                    kernel_created, network_json, progress_state
             FROM restore_journal WHERE vm_name = ?1 ORDER BY updated_at",
        )?;
        let rows = statement.query_map(params![vm_name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                PathBuf::from(row.get::<_, String>(2)?),
                PathBuf::from(row.get::<_, String>(3)?),
                row.get::<_, i64>(4)?,
                PathBuf::from(row.get::<_, String>(5)?),
                row.get::<_, i64>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (
                operation_id,
                vm_name,
                staging_path,
                volume_path,
                volume_created,
                kernel_path,
                kernel_created,
                network_json,
                progress_state,
            ) = row?;
            let network = network_json
                .map(|value| {
                    serde_json::from_str(&value).map_err(|error| {
                        SdkError::Migration(format!("decode restore journal network: {error}"))
                    })
                })
                .transpose()?;
            Ok(RestoreJournal {
                operation_id,
                vm_name,
                staging_path,
                volume_path,
                volume_created: volume_created != 0,
                kernel_path,
                kernel_created: kernel_created != 0,
                network,
                progress_state,
            })
        })
        .collect()
    }

    fn delete_restore_journal(&self, operation_id: &str) -> Result<(), SdkError> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM restore_journal WHERE operation_id = ?1",
            params![operation_id],
        )?;
        Ok(())
    }

    fn commit_restored_microvm(&self, commit: RestoredMicroVmCommit<'_>) -> Result<i64, SdkError> {
        let RestoredMicroVmCommit {
            record,
            network,
            credential,
            runtime,
            kernel,
            kernel_spec,
            kernel_integrity,
            snapshot_metadata,
            operation_id,
        } = commit;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = unix_timestamp()?;
        transaction.execute(
            "INSERT INTO microvms (
                name, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            params![
                record.name,
                record.distribution_id,
                record.image_id,
                record.kernel_id,
                record.firecracker_package_id,
                record.firectl_package_id,
                to_sqlite_integer(record.disk_size_bytes, "VM disk size")?,
                to_sqlite_integer(record.memory_bytes, "VM memory")?,
                to_sqlite_integer(record.memory_effective_mib, "VM effective memory")?,
                i64::from(record.vcpu_count),
                record.volume_path.to_string_lossy().into_owned(),
                record.rootfs_path.to_string_lossy().into_owned(),
                record.socket_path.to_string_lossy().into_owned(),
                bool_to_sqlite(record.expose_on_lan),
                record.created_at,
            ],
        )?;
        let vm_id = transaction.last_insert_rowid();
        persist_network_transaction(&transaction, vm_id, network)?;
        transaction.execute(
            "INSERT INTO vm_credentials (
                microvm_id, private_key_path, public_key_path, guest_authorized_keys_path,
                key_type, ssh_user, ssh_port, public_key_fingerprint, file_mode
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                vm_id,
                credential.private_key_path.to_string_lossy().into_owned(),
                credential.public_key_path.to_string_lossy().into_owned(),
                credential.guest_authorized_keys_path,
                credential.key_type,
                credential.ssh_user,
                i64::from(credential.ssh_port),
                credential.public_key_fingerprint,
                credential.file_mode,
            ],
        )?;
        transaction.execute(
            "INSERT INTO vm_runtime (
                microvm_id, firecracker_path, firectl_path, socket_path,
                process_id, process_state, updated_at
             ) VALUES (?1, ?2, ?3, ?4, NULL, 'stopped', ?5)",
            params![
                vm_id,
                runtime.firecracker_path.to_string_lossy().into_owned(),
                runtime.firectl_path.to_string_lossy().into_owned(),
                runtime.socket_path.to_string_lossy().into_owned(),
                now,
            ],
        )?;
        let download_id = persist_download(&transaction, kernel_spec, kernel_integrity)?;
        transaction.execute(
            "INSERT INTO kernels (
                registry_id, download_id, name, display_name, version, architecture,
                registry_path, registry_url, filename, size_bytes, sha256, format,
                mime_type, modified_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
             ON CONFLICT(registry_id) DO UPDATE SET
                download_id = excluded.download_id,
                name = excluded.name,
                display_name = excluded.display_name,
                version = excluded.version,
                architecture = excluded.architecture,
                registry_path = excluded.registry_path,
                registry_url = excluded.registry_url,
                filename = excluded.filename,
                size_bytes = excluded.size_bytes,
                sha256 = excluded.sha256,
                format = excluded.format,
                mime_type = excluded.mime_type,
                modified_at = excluded.modified_at,
                updated_at = excluded.updated_at",
            params![
                kernel.id,
                download_id,
                kernel.name,
                kernel.display_name,
                kernel.version,
                architecture_name(&kernel.architecture),
                kernel.path,
                kernel.url,
                kernel.filename,
                to_sqlite_integer(kernel.size_bytes, &kernel_spec.artifact_key)?,
                kernel.sha256,
                kernel.format,
                kernel.mime_type,
                kernel.modified_at,
                now,
            ],
        )?;
        persist_elf(&transaction, download_id, kernel.elf.as_ref())?;
        let kernel_row_id: i64 = transaction.query_row(
            "SELECT id FROM kernels WHERE registry_id = ?1",
            params![kernel.id],
            |row| row.get(0),
        )?;
        if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM distributions WHERE registry_id = ?1)",
            params![record.distribution_id],
            |row| row.get::<_, i64>(0),
        )? != 0
        {
            let distribution_row_id: i64 = transaction.query_row(
                "SELECT id FROM distributions WHERE registry_id = ?1",
                params![record.distribution_id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO distribution_kernels (distribution_id, kernel_id, is_default)
                 VALUES (?1, ?2, 1)",
                params![distribution_row_id, kernel_row_id],
            )?;
        }
        let kernel_args_json =
            serde_json::to_string(&snapshot_metadata.kernel_args).map_err(|error| {
                SdkError::Migration(format!("encode restored boot arguments: {error}"))
            })?;
        transaction.execute(
            "INSERT INTO vm_snapshot_metadata (
                microvm_id, distribution_name, distribution_version, root_device,
                kernel_args_json, image_sha256, guest_architecture
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                vm_id,
                snapshot_metadata.distribution_name,
                snapshot_metadata.distribution_version,
                snapshot_metadata.root_device,
                kernel_args_json,
                snapshot_metadata.image_sha256,
                snapshot_metadata.guest_architecture,
            ],
        )?;
        transaction.execute(
            "DELETE FROM restore_journal WHERE operation_id = ?1",
            params![operation_id],
        )?;
        transaction.commit()?;
        Ok(vm_id)
    }

    fn find_microvm(&self, name: &str) -> Result<Option<StoredMicroVm>, SdkError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT id, name, distribution_id, image_id, kernel_id,
                        firecracker_package_id, firectl_package_id, disk_size_bytes,
                        memory_requested_bytes, memory_effective_mib, vcpu_count,
                        volume_path, rootfs_path, socket_path, expose_on_lan,
                        created_at, updated_at
                 FROM microvms WHERE name = ?1",
                params![name],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        PathBuf::from(row.get::<_, String>(11)?),
                        PathBuf::from(row.get::<_, String>(12)?),
                        PathBuf::from(row.get::<_, String>(13)?),
                        row.get::<_, i64>(14)?,
                        row.get::<_, i64>(15)?,
                        row.get::<_, i64>(16)?,
                    ))
                },
            )
            .optional()?;
        let Some(row) = row else {
            return Ok(None);
        };
        let record = record_from_row(row)?;
        let network = load_network_record(&connection, record.id)?;
        let credential = load_credential_record(&connection, record.id)?;
        let runtime = load_runtime_record(&connection, record.id)?;
        Ok(Some(StoredMicroVm {
            record,
            network,
            credential,
            runtime,
        }))
    }

    fn list_stored_microvms(&self) -> Result<Vec<StoredMicroVm>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT name FROM microvms ORDER BY name")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SdkError::from)?;
        let mut stored = Vec::with_capacity(names.len());
        for name in names {
            let row = connection
                .query_row(
                    "SELECT id, name, distribution_id, image_id, kernel_id,
                            firecracker_package_id, firectl_package_id, disk_size_bytes,
                            memory_requested_bytes, memory_effective_mib, vcpu_count,
                            volume_path, rootfs_path, socket_path, expose_on_lan,
                            created_at, updated_at
                     FROM microvms WHERE name = ?1",
                    params![name],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, i64>(10)?,
                            PathBuf::from(row.get::<_, String>(11)?),
                            PathBuf::from(row.get::<_, String>(12)?),
                            PathBuf::from(row.get::<_, String>(13)?),
                            row.get::<_, i64>(14)?,
                            row.get::<_, i64>(15)?,
                            row.get::<_, i64>(16)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| {
                    SdkError::Migration(format!(
                        "inventory row for {name} disappeared during bulk listing"
                    ))
                })?;
            let record = record_from_row(row)?;
            let network = load_network_record(&connection, record.id)?;
            let credential = load_credential_record(&connection, record.id)?;
            let runtime = load_runtime_record(&connection, record.id)?;
            stored.push(StoredMicroVm {
                record,
                network,
                credential,
                runtime,
            });
        }
        Ok(stored)
    }

    fn find_volume_owner(&self, volume_path: &Path) -> Result<Option<String>, SdkError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT name FROM microvms WHERE volume_path = ?1",
                params![volume_path.to_string_lossy().into_owned()],
                |row| row.get(0),
            )
            .optional()
            .map_err(SdkError::from)
    }

    fn list_host_only_networks(&self) -> Result<Vec<(String, IpAddr, String)>, SdkError> {
        self.list_network_addresses(
            "SELECT m.name, n.guest_ip, n.tap_name
             FROM microvms m JOIN vm_networks n ON n.microvm_id = m.id
             WHERE n.mode = 'host_only'",
            "guest IP",
        )
    }

    fn list_lan_addresses(&self) -> Result<Vec<(String, IpAddr, String)>, SdkError> {
        self.list_network_addresses(
            "SELECT m.name, n.lan_ip, n.tap_name
             FROM microvms m JOIN vm_networks n ON n.microvm_id = m.id
             WHERE n.mode = 'lan' AND n.lan_ip IS NOT NULL",
            "LAN IP",
        )
    }

    fn insert_microvm(&self, record: &MicroVmRecord) -> Result<i64, SdkError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO microvms (
                name, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            params![
                record.name,
                record.distribution_id,
                record.image_id,
                record.kernel_id,
                record.firecracker_package_id,
                record.firectl_package_id,
                to_sqlite_integer(record.disk_size_bytes, "VM disk size")?,
                to_sqlite_integer(record.memory_bytes, "VM memory")?,
                to_sqlite_integer(record.memory_effective_mib, "VM effective memory")?,
                i64::from(record.vcpu_count),
                record.volume_path.to_string_lossy().into_owned(),
                record.rootfs_path.to_string_lossy().into_owned(),
                record.socket_path.to_string_lossy().into_owned(),
                bool_to_sqlite(record.expose_on_lan),
                record.created_at,
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    fn persist_network(&self, vm_id: i64, network: &PersistedNetwork) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        persist_network_transaction(&transaction, vm_id, network)?;
        transaction.commit()?;
        Ok(())
    }

    fn persist_credential(
        &self,
        vm_id: i64,
        credential: &PersistedCredential,
    ) -> Result<(), SdkError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO vm_credentials (
                microvm_id, private_key_path, public_key_path, guest_authorized_keys_path,
                key_type, ssh_user, ssh_port, public_key_fingerprint, file_mode
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(microvm_id) DO UPDATE SET
                private_key_path = excluded.private_key_path,
                public_key_path = excluded.public_key_path,
                guest_authorized_keys_path = excluded.guest_authorized_keys_path,
                key_type = excluded.key_type,
                ssh_user = excluded.ssh_user,
                ssh_port = excluded.ssh_port,
                public_key_fingerprint = excluded.public_key_fingerprint,
                file_mode = excluded.file_mode",
            params![
                vm_id,
                credential.private_key_path.to_string_lossy().into_owned(),
                credential.public_key_path.to_string_lossy().into_owned(),
                credential.guest_authorized_keys_path,
                credential.key_type,
                credential.ssh_user,
                i64::from(credential.ssh_port),
                credential.public_key_fingerprint,
                credential.file_mode,
            ],
        )?;
        Ok(())
    }

    fn persist_runtime(&self, vm_id: i64, runtime: &PersistedRuntime) -> Result<(), SdkError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO vm_runtime (
                microvm_id, firecracker_path, firectl_path, socket_path,
                process_id, process_state, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(microvm_id) DO UPDATE SET
                firecracker_path = excluded.firecracker_path,
                firectl_path = excluded.firectl_path,
                socket_path = excluded.socket_path,
                process_id = excluded.process_id,
                process_state = excluded.process_state,
                updated_at = excluded.updated_at",
            params![
                vm_id,
                runtime.firecracker_path.to_string_lossy().into_owned(),
                runtime.firectl_path.to_string_lossy().into_owned(),
                runtime.socket_path.to_string_lossy().into_owned(),
                runtime.process_id.map(i64::from),
                runtime.process_state,
                unix_timestamp()?,
            ],
        )?;
        Ok(())
    }

    fn delete_microvm(&self, vm_id: i64) -> Result<(), SdkError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM microvms WHERE id = ?1", params![vm_id])?;
        transaction.execute(
            "UPDATE network_bridges
             SET reference_count = (
                 SELECT COUNT(*) FROM vm_networks WHERE bridge_id = network_bridges.id
             ), updated_at = ?1",
            params![unix_timestamp()?],
        )?;
        transaction.execute(
            "DELETE FROM network_bridges
             WHERE reference_count = 0 AND ownership = 'sdk:taumaru'",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn update_network(&self, vm_id: i64, network: &PersistedNetwork) -> Result<(), SdkError> {
        self.persist_network(vm_id, network)
    }

    fn bridge_has_other_references(
        &self,
        vm_id: i64,
        bridge_name: &str,
        uplink_name: &str,
    ) -> Result<bool, SdkError> {
        let connection = self.connection()?;
        let referenced: i64 = connection.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM vm_networks n
                JOIN network_bridges b ON b.id = n.bridge_id
                WHERE n.microvm_id != ?1
                  AND b.bridge_name = ?2
                  AND b.uplink_name = ?3
                  AND b.ownership = 'sdk:taumaru'
            )",
            params![vm_id, bridge_name, uplink_name],
            |row| row.get(0),
        )?;
        Ok(referenced != 0)
    }

    fn bridge_is_managed(&self, bridge_name: &str, uplink_name: &str) -> Result<bool, SdkError> {
        let connection = self.connection()?;
        let managed: i64 = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM network_bridges
                WHERE bridge_name = ?1
                  AND uplink_name = ?2
                  AND ownership = 'sdk:taumaru'
            )",
            params![bridge_name, uplink_name],
            |row| row.get(0),
        )?;
        Ok(managed != 0)
    }

    fn load_network(&self, vm_id: i64) -> Result<NetworkConfiguration, SdkError> {
        let connection = self.connection()?;
        load_network_record(&connection, vm_id)?
            .map(|network| network.config)
            .ok_or_else(|| SdkError::Migration(format!("network record is missing for VM {vm_id}")))
    }

    fn find_autostart_policy(&self, name: &str) -> Result<Option<AutostartPolicy>, SdkError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT microvms.name, vm_autostart.enabled, vm_autostart.max_start_attempts
                 FROM vm_autostart
                 JOIN microvms ON microvms.id = vm_autostart.microvm_id
                 WHERE microvms.name = ?1",
                params![name],
                autostart_policy_from_row,
            )
            .optional()?
            .transpose()
    }

    fn list_autostart_policies(&self) -> Result<Vec<AutostartPolicy>, SdkError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT microvms.name, vm_autostart.enabled, vm_autostart.max_start_attempts
             FROM vm_autostart
             JOIN microvms ON microvms.id = vm_autostart.microvm_id
             ORDER BY microvms.name",
        )?;
        let rows = statement
            .query_map([], autostart_policy_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().collect()
    }

    fn insert_autostart_policy(
        &self,
        vm_id: i64,
        settings: &AutostartSettings,
    ) -> Result<(), SdkError> {
        let connection = self.connection()?;
        let now = unix_timestamp()?;
        connection.execute(
            "INSERT INTO vm_autostart (
                microvm_id, enabled, max_start_attempts, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![
                vm_id,
                settings.enabled,
                i64::from(settings.max_start_attempts),
                now
            ],
        )?;
        Ok(())
    }

    fn update_autostart_policy(
        &self,
        vm_id: i64,
        settings: &AutostartSettings,
    ) -> Result<(), SdkError> {
        let connection = self.connection()?;
        connection.execute(
            "UPDATE vm_autostart
             SET enabled = ?2, max_start_attempts = ?3, updated_at = ?4
             WHERE microvm_id = ?1",
            params![
                vm_id,
                settings.enabled,
                i64::from(settings.max_start_attempts),
                unix_timestamp()?
            ],
        )?;
        Ok(())
    }

    fn delete_autostart_policy(&self, vm_id: i64) -> Result<bool, SdkError> {
        let connection = self.connection()?;
        let removed = connection.execute(
            "DELETE FROM vm_autostart WHERE microvm_id = ?1",
            params![vm_id],
        )?;
        Ok(removed > 0)
    }
}

fn autostart_policy_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<AutostartPolicy, SdkError>> {
    let name = row.get::<_, String>(0)?;
    let enabled = row.get::<_, bool>(1)?;
    let attempts = row.get::<_, i64>(2)?;
    Ok(u32::try_from(attempts)
        .map(|max_start_attempts| AutostartPolicy {
            name: name.clone(),
            enabled,
            max_start_attempts,
        })
        .map_err(|_| {
            SdkError::Migration(format!(
                "invalid persisted autostart attempts for {name}: {attempts}"
            ))
        }))
}

type MicroVmRow = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    PathBuf,
    PathBuf,
    PathBuf,
    i64,
    i64,
    i64,
);

fn record_from_row(row: MicroVmRow) -> Result<MicroVmRecord, SdkError> {
    let (
        id,
        name,
        distribution_id,
        image_id,
        kernel_id,
        firecracker_package_id,
        firectl_package_id,
        disk_size_bytes,
        memory_bytes,
        memory_effective_mib,
        vcpu_count,
        volume_path,
        rootfs_path,
        socket_path,
        expose_on_lan,
        created_at,
        _updated_at,
    ) = row;
    let disk_size_bytes = from_sqlite_integer(disk_size_bytes, "VM disk size")?;
    let memory_bytes = from_sqlite_integer(memory_bytes, "VM memory")?;
    let memory_effective_mib = from_sqlite_integer(memory_effective_mib, "VM effective memory")?;
    let vcpu_count = u32::try_from(vcpu_count)
        .map_err(|_| SdkError::Migration("persisted vCPU count is invalid".to_owned()))?;
    Ok(MicroVmRecord {
        id,
        name,
        distribution_id,
        image_id,
        kernel_id,
        firecracker_package_id,
        firectl_package_id,
        disk_size_bytes,
        memory_bytes,
        memory_effective_mib,
        vcpu_count,
        volume_path,
        rootfs_path,
        socket_path,
        expose_on_lan: expose_on_lan != 0,
        created_at,
    })
}

fn load_network_record(
    connection: &Connection,
    vm_id: i64,
) -> Result<Option<PersistedNetwork>, SdkError> {
    let row = connection
        .query_row(
            "SELECT n.mode, n.guest_ip, n.prefix_length, n.gateway_ip, n.host_ip,
                    n.tap_name, n.guest_mac, n.bridge_id, b.bridge_name, n.uplink_name,
                    n.dhcp_lease_reference, n.desired_boot_parameters,
                    n.bridge_created_by_sdk, n.uplink_attached_by_sdk,
                    n.forwarding_enabled_by_sdk, n.nat_table_created_by_sdk,
                    n.nat_chain_created_by_sdk, n.host_address_specs,
                    n.default_route_specs, n.lan_ip, n.uplink_cidr,
                    n.proxy_arp_enabled_by_sdk, n.host_route_created_by_sdk,
                    n.proxy_arp_entry_created_by_sdk, n.iptables_forward_specs,
                    n.iptables_nat_spec
             FROM vm_networks n
             LEFT JOIN network_bridges b ON b.id = n.bridge_id
             WHERE n.microvm_id = ?1",
            params![vm_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, i64>(14)?,
                    row.get::<_, i64>(15)?,
                    row.get::<_, i64>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, i64>(21)?,
                    row.get::<_, i64>(22)?,
                    row.get::<_, i64>(23)?,
                    row.get::<_, String>(24)?,
                    row.get::<_, String>(25)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (
        mode,
        guest_ip,
        prefix_length,
        gateway_ip,
        host_ip,
        tap_name,
        guest_mac,
        _bridge_id,
        bridge_name,
        uplink_name,
        dhcp_lease_reference,
        desired_boot_parameters,
        bridge_created_by_sdk,
        uplink_attached_by_sdk,
        forwarding_enabled_by_sdk,
        nat_table_created_by_sdk,
        nat_chain_created_by_sdk,
        host_address_specs,
        default_route_specs,
        lan_ip,
        uplink_cidr,
        proxy_arp_enabled_by_sdk,
        host_route_created_by_sdk,
        proxy_arp_entry_created_by_sdk,
        _iptables_forward_specs,
        _iptables_nat_spec,
    ) = row;
    let mode = NetworkMode::parse(&mode)
        .ok_or_else(|| SdkError::Migration(format!("unknown persisted network mode {mode}")))?;
    let config = NetworkConfiguration {
        mode,
        guest_address: parse_ip(&guest_ip, "guest IP")?,
        prefix_length: u8::try_from(prefix_length)
            .map_err(|_| SdkError::Migration("persisted network prefix is invalid".to_owned()))?,
        gateway: parse_optional_ip(gateway_ip.as_deref(), "gateway IP")?,
        tap_name,
        bridge_name,
        uplink_name,
        lan_address: lan_ip
            .as_deref()
            .map(|value| parse_ip(value, "LAN IP"))
            .transpose()?,
    };

    let mut statement = connection.prepare(
        "SELECT resource_kind, resource_identity, desired_fingerprint, ownership,
                adapter_handle, last_observed
         FROM vm_network_resources WHERE microvm_id = ?1 ORDER BY id",
    )?;
    let resources = statement
        .query_map(params![vm_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .map(|row| {
            let (resource, identity, fingerprint, ownership, adapter_handle, last_observed) = row?;
            let resource = NetworkResource::parse(&resource).ok_or_else(|| {
                SdkError::Migration(format!("unknown persisted network resource {resource}"))
            })?;
            Ok(PersistedNetworkResource {
                resource,
                identity,
                fingerprint,
                ownership,
                adapter_handle,
                last_observed,
            })
        })
        .collect::<Result<Vec<_>, SdkError>>()?;
    Ok(Some(PersistedNetwork {
        config,
        host_address: parse_optional_ip(host_ip.as_deref(), "host IP")?,
        guest_mac,
        dhcp_lease_reference,
        desired_boot_parameters,
        uplink_cidr,
        proxy_arp_enabled_by_sdk: proxy_arp_enabled_by_sdk != 0,
        bridge_created_by_sdk: bridge_created_by_sdk != 0,
        uplink_attached_by_sdk: uplink_attached_by_sdk != 0,
        forwarding_enabled_by_sdk: forwarding_enabled_by_sdk != 0,
        nat_table_created_by_sdk: nat_table_created_by_sdk != 0,
        nat_chain_created_by_sdk: nat_chain_created_by_sdk != 0,
        host_route_created_by_sdk: host_route_created_by_sdk != 0,
        proxy_arp_entry_created_by_sdk: proxy_arp_entry_created_by_sdk != 0,
        host_address_specs: serde_json::from_str(&host_address_specs).map_err(|error| {
            SdkError::Migration(format!("invalid persisted host address state: {error}"))
        })?,
        default_route_specs: serde_json::from_str(&default_route_specs).map_err(|error| {
            SdkError::Migration(format!("invalid persisted default route state: {error}"))
        })?,
        resources,
    }))
}

fn load_credential_record(
    connection: &Connection,
    vm_id: i64,
) -> Result<Option<PersistedCredential>, SdkError> {
    let row = connection
        .query_row(
            "SELECT private_key_path, public_key_path, guest_authorized_keys_path,
                    key_type, ssh_user, ssh_port, public_key_fingerprint, file_mode
             FROM vm_credentials WHERE microvm_id = ?1",
            params![vm_id],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (private_key_path, public_key_path, guest_path, key_type, user, port, fingerprint, mode) =
        row;
    Ok(Some(PersistedCredential {
        private_key_path,
        public_key_path,
        guest_authorized_keys_path: guest_path,
        key_type,
        ssh_user: user,
        ssh_port: u16::try_from(port)
            .map_err(|_| SdkError::Migration("persisted SSH port is invalid".to_owned()))?,
        public_key_fingerprint: fingerprint,
        file_mode: mode,
    }))
}

fn load_runtime_record(
    connection: &Connection,
    vm_id: i64,
) -> Result<Option<PersistedRuntime>, SdkError> {
    let row = connection
        .query_row(
            "SELECT firecracker_path, firectl_path, socket_path, process_id, process_state
             FROM vm_runtime WHERE microvm_id = ?1",
            params![vm_id],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    PathBuf::from(row.get::<_, String>(2)?),
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (firecracker_path, firectl_path, socket_path, process_id, process_state) = row;
    Ok(Some(PersistedRuntime {
        firecracker_path,
        firectl_path,
        socket_path,
        process_id: process_id
            .map(|value| {
                u32::try_from(value)
                    .map_err(|_| SdkError::Migration("persisted process ID is invalid".to_owned()))
            })
            .transpose()?,
        process_state,
    }))
}

fn persist_network_transaction(
    transaction: &Transaction<'_>,
    vm_id: i64,
    network: &PersistedNetwork,
) -> Result<(), SdkError> {
    let previous_bridge_id: Option<i64> = transaction
        .query_row(
            "SELECT bridge_id FROM vm_networks WHERE microvm_id = ?1",
            params![vm_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten();
    let bridge_id = match (&network.config.bridge_name, &network.config.uplink_name) {
        (Some(bridge_name), Some(uplink_name)) => {
            let existing = transaction
                .query_row(
                    "SELECT id, ownership FROM network_bridges
                     WHERE bridge_name = ?1 AND uplink_name = ?2",
                    params![bridge_name, uplink_name],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            match existing {
                Some((id, ownership)) if ownership == "sdk:taumaru" => Some(id),
                Some((_id, ownership)) => {
                    return Err(SdkError::Network {
                        mode: network.config.mode.to_string(),
                        operation: "persist bridge ownership".to_owned(),
                        resource: bridge_name.clone(),
                        reason: format!("bridge is owned by {ownership}"),
                    });
                }
                None => {
                    transaction.execute(
                        "INSERT INTO network_bridges (
                            bridge_name, uplink_name, ownership, reference_count,
                            created_at, updated_at
                        ) VALUES (?1, ?2, 'sdk:taumaru', 0, ?3, ?3)",
                        params![bridge_name, uplink_name, unix_timestamp()?],
                    )?;
                    Some(transaction.last_insert_rowid())
                }
            }
        }
        (None, Some(_)) if network.config.mode == NetworkMode::Lan => None,
        (None, None) => None,
        _ => {
            return Err(SdkError::Network {
                mode: network.config.mode.to_string(),
                operation: "persist network configuration".to_owned(),
                resource: "bridge/uplink".to_owned(),
                reason: "LAN configuration must include both bridge and uplink".to_owned(),
            });
        }
    };
    let guest_ip = network.config.guest_address.to_string();
    let gateway_ip = network.config.gateway.map(|value| value.to_string());
    let host_ip = network.host_address.map(|value| value.to_string());
    let lan_ip = network.config.lan_address.map(|value| value.to_string());
    transaction.execute(
        "INSERT INTO vm_networks (
            microvm_id, mode, guest_ip, prefix_length, gateway_ip, host_ip,
            tap_name, guest_mac, bridge_id, uplink_name, dhcp_lease_reference,
            desired_boot_parameters, bridge_created_by_sdk, uplink_attached_by_sdk,
            forwarding_enabled_by_sdk, nat_table_created_by_sdk, nat_chain_created_by_sdk,
            host_address_specs, default_route_specs, lan_ip, uplink_cidr,
            proxy_arp_enabled_by_sdk, host_route_created_by_sdk,
            proxy_arp_entry_created_by_sdk, iptables_forward_specs, iptables_nat_spec,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)
        ON CONFLICT(microvm_id) DO UPDATE SET
            mode = excluded.mode,
            guest_ip = excluded.guest_ip,
            prefix_length = excluded.prefix_length,
            gateway_ip = excluded.gateway_ip,
            host_ip = excluded.host_ip,
            tap_name = excluded.tap_name,
            guest_mac = excluded.guest_mac,
            bridge_id = excluded.bridge_id,
            uplink_name = excluded.uplink_name,
            dhcp_lease_reference = excluded.dhcp_lease_reference,
            desired_boot_parameters = excluded.desired_boot_parameters,
            bridge_created_by_sdk = excluded.bridge_created_by_sdk,
            uplink_attached_by_sdk = excluded.uplink_attached_by_sdk,
            forwarding_enabled_by_sdk = excluded.forwarding_enabled_by_sdk,
            nat_table_created_by_sdk = excluded.nat_table_created_by_sdk,
            nat_chain_created_by_sdk = excluded.nat_chain_created_by_sdk,
            host_address_specs = excluded.host_address_specs,
            default_route_specs = excluded.default_route_specs,
            lan_ip = excluded.lan_ip,
            uplink_cidr = excluded.uplink_cidr,
            proxy_arp_enabled_by_sdk = excluded.proxy_arp_enabled_by_sdk,
            host_route_created_by_sdk = excluded.host_route_created_by_sdk,
            proxy_arp_entry_created_by_sdk = excluded.proxy_arp_entry_created_by_sdk,
            iptables_forward_specs = excluded.iptables_forward_specs,
            iptables_nat_spec = excluded.iptables_nat_spec,
            updated_at = excluded.updated_at",
        params![
            vm_id,
            network.config.mode.as_str(),
            guest_ip,
            i64::from(network.config.prefix_length),
            gateway_ip,
            host_ip,
            network.config.tap_name,
            network.guest_mac,
            bridge_id,
            network.config.uplink_name,
            network.dhcp_lease_reference,
            network.desired_boot_parameters,
            network.bridge_created_by_sdk,
            network.uplink_attached_by_sdk,
            network.forwarding_enabled_by_sdk,
            network.nat_table_created_by_sdk,
            network.nat_chain_created_by_sdk,
            serde_json::to_string(&network.host_address_specs).map_err(|error| {
                SdkError::Migration(format!("could not encode host address state: {error}"))
            })?,
            serde_json::to_string(&network.default_route_specs).map_err(|error| {
                SdkError::Migration(format!("could not encode default route state: {error}"))
            })?,
            lan_ip,
            network.uplink_cidr,
            network.proxy_arp_enabled_by_sdk,
            network.host_route_created_by_sdk,
            network.proxy_arp_entry_created_by_sdk,
            serde_json::to_string(&Vec::<String>::new()).map_err(|error| {
                SdkError::Migration(format!("could not encode iptables state: {error}"))
            })?,
            serde_json::to_string(&Option::<String>::None).map_err(|error| {
                SdkError::Migration(format!("could not encode iptables state: {error}"))
            })?,
            unix_timestamp()?,
        ],
    )?;
    transaction.execute(
        "DELETE FROM vm_network_resources WHERE microvm_id = ?1",
        params![vm_id],
    )?;
    for resource in &network.resources {
        transaction.execute(
            "INSERT INTO vm_network_resources (
                microvm_id, resource_kind, resource_identity, desired_fingerprint,
                ownership, adapter_handle, last_observed
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                vm_id,
                resource.resource.as_str(),
                resource.identity,
                resource.fingerprint,
                resource.ownership,
                resource.adapter_handle,
                resource.last_observed,
            ],
        )?;
    }
    let now = unix_timestamp()?;
    transaction.execute(
        "UPDATE network_bridges
         SET reference_count = (
             SELECT COUNT(*) FROM vm_networks WHERE bridge_id = network_bridges.id
         ), updated_at = ?1",
        params![now],
    )?;
    if let Some(previous_bridge_id) = previous_bridge_id
        && Some(previous_bridge_id) != bridge_id
    {
        transaction.execute(
            "DELETE FROM network_bridges
             WHERE id = ?1 AND reference_count = 0 AND ownership = 'sdk:taumaru'",
            params![previous_bridge_id],
        )?;
    }
    Ok(())
}

fn parse_ip(value: &str, field: &str) -> Result<IpAddr, SdkError> {
    value
        .parse()
        .map_err(|error| SdkError::Migration(format!("invalid persisted {field} {value}: {error}")))
}

fn parse_optional_ip(value: Option<&str>, field: &str) -> Result<Option<IpAddr>, SdkError> {
    value.map(|value| parse_ip(value, field)).transpose()
}

pub(crate) fn open_connection(database_path: &Path) -> Result<Connection, SdkError> {
    let connection = Connection::open(database_path)?;
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(connection)
}

fn local_artifact_from_row(
    row: Option<(PathBuf, i64, String, i64, String, String)>,
    kind: &str,
    id: &str,
) -> Result<LocalArtifact, SdkError> {
    let Some((path, actual_size, actual_sha256, expected_size, expected_sha256, status)) = row
    else {
        return Err(SdkError::NotFound {
            kind: kind.to_owned(),
            id: id.to_owned(),
        });
    };
    let actual_size = u64::try_from(actual_size)
        .map_err(|_| SdkError::Migration(format!("negative {kind} size in inventory")))?;
    let expected_size = u64::try_from(expected_size)
        .map_err(|_| SdkError::Migration(format!("negative {kind} expected size in inventory")))?;
    if status != "verified" || actual_size != expected_size || actual_sha256 != expected_sha256 {
        return Err(SdkError::ArtifactPrerequisite {
            kind: kind.to_owned(),
            id: id.to_owned(),
            path,
            reason: "the local inventory entry is stale".to_owned(),
        });
    }
    Ok(LocalArtifact {
        path,
        size_bytes: actual_size,
        sha256: actual_sha256,
    })
}

fn persist_download(
    transaction: &Transaction<'_>,
    spec: &DownloadSpec,
    integrity: &FileIntegrity,
) -> Result<i64, SdkError> {
    let now = unix_timestamp()?;
    let relative_path = spec.relative_path.to_string_lossy().into_owned();
    let absolute_path = spec.absolute_path.to_string_lossy().into_owned();
    transaction.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'verified', ?12, ?12, ?12)
        ON CONFLICT(artifact_key) DO UPDATE SET
            artifact_type = excluded.artifact_type,
            registry_path = excluded.registry_path,
            registry_url = excluded.registry_url,
            filename = excluded.filename,
            relative_path = excluded.relative_path,
            absolute_path = excluded.absolute_path,
            expected_size_bytes = excluded.expected_size_bytes,
            expected_sha256 = excluded.expected_sha256,
            actual_size_bytes = excluded.actual_size_bytes,
            actual_sha256 = excluded.actual_sha256,
            verification_status = excluded.verification_status,
            updated_at = excluded.updated_at,
            last_verified_at = excluded.last_verified_at",
        params![
            spec.artifact_key,
            artifact_type_name(&spec.artifact_kind),
            spec.registry_path,
            spec.registry_url,
            spec.filename,
            relative_path,
            absolute_path,
            to_sqlite_integer(spec.expected_size, &spec.artifact_key)?,
            spec.expected_sha256,
            to_sqlite_integer(integrity.size_bytes, &spec.artifact_key)?,
            integrity.sha256,
            now,
        ],
    )?;
    Ok(transaction.query_row(
        "SELECT id FROM downloads WHERE artifact_key = ?1",
        params![spec.artifact_key],
        |row| row.get(0),
    )?)
}

fn member_relation_exists(
    connection: &Connection,
    spec: &DownloadSpec,
    download_id: i64,
) -> Result<bool, SdkError> {
    let exists: i64 = match &spec.artifact_kind {
        ArtifactKind::Kernel => connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM kernels WHERE registry_id = ?1 AND download_id = ?2
            )",
            params![spec.artifact_id, download_id],
            |row| row.get(0),
        )?,
        ArtifactKind::Binary => {
            let component = spec.member_name.as_deref().ok_or_else(|| {
                SdkError::invalid_metadata(&spec.artifact_key, "binary component is missing")
            })?;
            connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM binary_files bf
                    JOIN binary_packages bp ON bp.id = bf.binary_package_id
                    WHERE bp.registry_id = ?1 AND bf.component_name = ?2 AND bf.download_id = ?3
                )",
                params![spec.artifact_id, component, download_id],
                |row| row.get(0),
            )?
        }
        ArtifactKind::DistributionImage => {
            let image = spec.member_name.as_deref().ok_or_else(|| {
                SdkError::invalid_metadata(&spec.artifact_key, "distribution image is missing")
            })?;
            connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM distribution_images di
                    JOIN distributions d ON d.id = di.distribution_id
                    WHERE d.registry_id = ?1 AND di.registry_id = ?2 AND di.download_id = ?3
                )",
                params![spec.artifact_id, image, download_id],
                |row| row.get(0),
            )?
        }
    };
    Ok(exists != 0)
}

fn persist_elf(
    transaction: &Transaction<'_>,
    download_id: i64,
    metadata: Option<&ElfMetadata>,
) -> Result<(), SdkError> {
    transaction.execute(
        "DELETE FROM elf_metadata WHERE download_id = ?1",
        params![download_id],
    )?;
    let Some(metadata) = metadata else {
        return Ok(());
    };

    transaction.execute(
        "INSERT INTO elf_metadata (
            download_id, class, endianness, elf_type, machine, entry_point,
            build_id, interpreter, linkage, stripped
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            download_id,
            metadata.class,
            endianness_name(&metadata.endianness),
            metadata.elf_type,
            metadata.machine,
            metadata.entry_point,
            metadata.build_id,
            metadata.interpreter,
            metadata.linkage.as_ref().map(linkage_name),
            metadata.stripped.map(bool_to_sqlite),
        ],
    )?;
    if let Some(libraries) = &metadata.needed_libraries {
        for (position, library) in libraries.iter().enumerate() {
            transaction.execute(
                "INSERT INTO elf_needed_libraries (download_id, position, library)
                 VALUES (?1, ?2, ?3)",
                params![
                    download_id,
                    i64::try_from(position).map_err(|_| SdkError::Migration(
                        "ELF library position exceeds SQLite INTEGER".to_owned(),
                    ))?,
                    library
                ],
            )?;
        }
    }
    Ok(())
}

fn replace_boot_args(
    transaction: &Transaction<'_>,
    distribution_id: i64,
    arguments: &[String],
) -> Result<(), SdkError> {
    transaction.execute(
        "DELETE FROM distribution_boot_args WHERE distribution_id = ?1",
        params![distribution_id],
    )?;
    for (position, argument) in arguments.iter().enumerate() {
        transaction.execute(
            "INSERT INTO distribution_boot_args (distribution_id, position, argument)
             VALUES (?1, ?2, ?3)",
            params![
                distribution_id,
                i64::try_from(position).map_err(|_| SdkError::Migration(
                    "boot argument position exceeds SQLite INTEGER".to_owned(),
                ))?,
                argument,
            ],
        )?;
    }
    Ok(())
}

fn persist_kernel_references(
    transaction: &Transaction<'_>,
    distribution_id: i64,
    distribution: &Distribution,
    kernels: &[Kernel],
    now: i64,
) -> Result<(), SdkError> {
    transaction.execute(
        "DELETE FROM distribution_kernels WHERE distribution_id = ?1",
        params![distribution_id],
    )?;
    for kernel_id in &distribution.supported_kernels {
        let kernel = kernels
            .iter()
            .find(|candidate| candidate.id == *kernel_id)
            .ok_or_else(|| {
                SdkError::invalid_metadata(
                    &distribution.id,
                    format!("supported kernel {kernel_id} is absent from the manifest"),
                )
            })?;
        ensure_kernel_reference(transaction, kernel, now)?;
        let database_kernel_id: i64 = transaction.query_row(
            "SELECT id FROM kernels WHERE registry_id = ?1",
            params![kernel.id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO distribution_kernels (distribution_id, kernel_id, is_default)
             VALUES (?1, ?2, ?3)",
            params![
                distribution_id,
                database_kernel_id,
                bool_to_sqlite(kernel.id == distribution.default_kernel),
            ],
        )?;
    }
    if !distribution
        .supported_kernels
        .iter()
        .any(|kernel_id| kernel_id == &distribution.default_kernel)
    {
        let kernel = kernels
            .iter()
            .find(|candidate| candidate.id == distribution.default_kernel)
            .ok_or_else(|| {
                SdkError::invalid_metadata(
                    &distribution.id,
                    format!(
                        "default kernel {} is absent from the manifest",
                        distribution.default_kernel
                    ),
                )
            })?;
        ensure_kernel_reference(transaction, kernel, now)?;
        let database_kernel_id: i64 = transaction.query_row(
            "SELECT id FROM kernels WHERE registry_id = ?1",
            params![kernel.id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO distribution_kernels (distribution_id, kernel_id, is_default)
             VALUES (?1, ?2, 1)",
            params![distribution_id, database_kernel_id],
        )?;
    }
    Ok(())
}

fn ensure_kernel_reference(
    transaction: &Transaction<'_>,
    kernel: &Kernel,
    now: i64,
) -> Result<(), SdkError> {
    transaction.execute(
        "INSERT INTO kernels (
            registry_id, download_id, name, display_name, version, architecture,
            registry_path, registry_url, filename, size_bytes, sha256, format,
            mime_type, modified_at, created_at, updated_at
        ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
        ON CONFLICT(registry_id) DO UPDATE SET
            name = excluded.name,
            display_name = excluded.display_name,
            version = excluded.version,
            architecture = excluded.architecture,
            registry_path = excluded.registry_path,
            registry_url = excluded.registry_url,
            filename = excluded.filename,
            size_bytes = excluded.size_bytes,
            sha256 = excluded.sha256,
            format = excluded.format,
            mime_type = excluded.mime_type,
            modified_at = excluded.modified_at,
            updated_at = excluded.updated_at",
        params![
            kernel.id,
            kernel.name,
            kernel.display_name,
            kernel.version,
            architecture_name(&kernel.architecture),
            kernel.path,
            kernel.url,
            kernel.filename,
            to_sqlite_integer(kernel.size_bytes, &kernel.id)?,
            kernel.sha256,
            kernel.format,
            kernel.mime_type,
            kernel.modified_at,
            now,
        ],
    )?;
    Ok(())
}

fn replace_filesystem(
    transaction: &Transaction<'_>,
    image_id: i64,
    filesystem: &crate::domain::registry::FilesystemMetadata,
) -> Result<(), SdkError> {
    transaction.execute(
        "DELETE FROM image_filesystems WHERE distribution_image_id = ?1",
        params![image_id],
    )?;
    transaction.execute(
        "INSERT INTO image_filesystems (
            distribution_image_id, filesystem_type, uuid, block_size, block_count,
            free_blocks, inode_count, free_inodes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            image_id,
            filesystem.filesystem_type,
            filesystem.uuid,
            to_sqlite_integer(filesystem.block_size, &image_id.to_string())?,
            to_sqlite_integer(filesystem.block_count, &image_id.to_string())?,
            to_sqlite_integer(filesystem.free_blocks, &image_id.to_string())?,
            to_sqlite_integer(filesystem.inode_count, &image_id.to_string())?,
            to_sqlite_integer(filesystem.free_inodes, &image_id.to_string())?,
        ],
    )?;
    let filesystem_id = image_id;
    for (position, feature) in filesystem.features.iter().enumerate() {
        transaction.execute(
            "INSERT INTO image_filesystem_features (image_filesystem_id, position, feature)
             VALUES (?1, ?2, ?3)",
            params![
                filesystem_id,
                i64::try_from(position).map_err(|_| SdkError::Migration(
                    "filesystem feature position exceeds SQLite INTEGER".to_owned(),
                ))?,
                feature,
            ],
        )?;
    }
    Ok(())
}

fn replace_capabilities(
    transaction: &Transaction<'_>,
    image_id: i64,
    capabilities: &[String],
) -> Result<(), SdkError> {
    transaction.execute(
        "DELETE FROM image_capabilities WHERE distribution_image_id = ?1",
        params![image_id],
    )?;
    for (position, capability) in capabilities.iter().enumerate() {
        transaction.execute(
            "INSERT INTO image_capabilities (distribution_image_id, position, capability)
             VALUES (?1, ?2, ?3)",
            params![
                image_id,
                i64::try_from(position).map_err(|_| SdkError::Migration(
                    "image capability position exceeds SQLite INTEGER".to_owned(),
                ))?,
                capability,
            ],
        )?;
    }
    Ok(())
}

fn incomplete_snapshot_metadata(name: &str) -> SdkError {
    SdkError::InvalidMetadata {
        artifact: name.to_owned(),
        reason: "stored distribution, image, or boot provenance is incomplete".to_owned(),
    }
}

fn unix_timestamp() -> Result<i64, SdkError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            SdkError::Migration(format!("system clock is before Unix epoch: {error}"))
        })?;
    i64::try_from(duration.as_secs()).map_err(|_| {
        SdkError::Migration("current Unix timestamp exceeds SQLite INTEGER".to_owned())
    })
}

fn to_sqlite_integer(value: u64, context: &str) -> Result<i64, SdkError> {
    i64::try_from(value)
        .map_err(|_| SdkError::invalid_metadata(context, "value exceeds SQLite INTEGER"))
}

fn from_sqlite_integer(value: i64, context: &str) -> Result<u64, SdkError> {
    u64::try_from(value)
        .map_err(|_| SdkError::Migration(format!("negative {context} stored in SQLite")))
}

fn bool_to_sqlite(value: bool) -> i64 {
    i64::from(value)
}

fn artifact_type_name(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Kernel => "kernel",
        ArtifactKind::Binary => "binary",
        ArtifactKind::DistributionImage => "distribution_image",
    }
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

fn parse_architecture(value: &str) -> Result<Architecture, SdkError> {
    match value {
        "x86_64" => Ok(Architecture::X86_64),
        "aarch64" => Ok(Architecture::Aarch64),
        "arm" => Ok(Architecture::Arm),
        "riscv64" => Ok(Architecture::Riscv64),
        "x86" => Ok(Architecture::X86),
        _ => Err(SdkError::invalid_metadata(
            value,
            "unsupported architecture in local inventory",
        )),
    }
}

fn endianness_name(value: &Endianness) -> &'static str {
    match value {
        Endianness::Little => "little",
        Endianness::Big => "big",
    }
}

fn linkage_name(value: &Linkage) -> &'static str {
    match value {
        Linkage::Static => "static",
        Linkage::Dynamic => "dynamic",
    }
}

fn kernel_is_referenced(transaction: &Transaction<'_>, kernel_id: &str) -> Result<bool, SdkError> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM microvms WHERE kernel_id = ?1",
        params![kernel_id],
        |row| row.get(0),
    )?;
    Ok(count != 0)
}

fn image_is_referenced(
    transaction: &Transaction<'_>,
    distribution_id: &str,
    image_id: &str,
) -> Result<bool, SdkError> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM microvms WHERE distribution_id = ?1 AND image_id = ?2",
        params![distribution_id, image_id],
        |row| row.get(0),
    )?;
    Ok(count != 0)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::ports::repository::{ArtifactRepository, MicroVmRepository};

    use super::{SqliteRepository, open_connection};

    fn empty_repository() -> (TempDir, SqliteRepository) {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let database_path = directory.path().join("inventory.db");
        let repository =
            SqliteRepository::initialize(database_path).expect("repository should initialize");
        (directory, repository)
    }

    #[test]
    fn reads_snapshot_boot_metadata_from_sqlite_after_restore_migration() {
        let (directory, repository) = empty_repository();
        let database_path = directory.path().join("inventory.db");
        let connection = open_connection(&database_path).expect("inventory connection should open");
        connection
            .execute(
                "INSERT INTO distributions (
                    registry_id, name, display_name, description, distribution, version,
                    codename, architecture, vendor, homepage, default_kernel_registry_id,
                    root_device, min_memory_mb, min_vcpus, created_at, updated_at
                ) VALUES ('distro-1', 'linux-fixture', 'Linux Fixture', 'fixture', 'Linux',
                          '1.0', 'stable', 'x86_64', 'Taumaru', 'https://fixture.invalid',
                          'kernel-1', '/dev/vda', 128, 1, 1, 1)",
                [],
            )
            .expect("distribution should insert");
        connection
            .execute(
                "INSERT INTO kernels (
                    registry_id, download_id, name, display_name, version, architecture,
                    registry_path, registry_url, filename, size_bytes, sha256, format,
                    mime_type, modified_at, created_at, updated_at
                ) VALUES ('kernel-1', NULL, 'linux', 'Linux Kernel', '1.0', 'x86_64',
                          'kernels/vmlinux', 'https://fixture.invalid/kernel', 'vmlinux',
                          4, ?1, 'elf', 'application/octet-stream', '2026-09-23T00:00:00Z', 1, 1)",
                rusqlite::params!["a".repeat(64)],
            )
            .expect("kernel should insert");
        let distribution_row: i64 = connection
            .query_row(
                "SELECT id FROM distributions WHERE registry_id = 'distro-1'",
                [],
                |row| row.get(0),
            )
            .expect("distribution row should exist");
        connection
            .execute(
                "INSERT INTO distribution_kernels (distribution_id, kernel_id, is_default)
                 VALUES (?1, (SELECT id FROM kernels WHERE registry_id = 'kernel-1'), 1)",
                rusqlite::params![distribution_row],
            )
            .expect("default kernel relation should insert");
        connection
            .execute(
                "INSERT INTO downloads (
                    artifact_key, artifact_type, registry_path, registry_url, filename,
                    relative_path, absolute_path, expected_size_bytes, expected_sha256,
                    actual_size_bytes, actual_sha256, verification_status, created_at, updated_at
                ) VALUES ('distribution_image:distro-1:image-1', 'distribution_image',
                    'images/rootfs.ext4', 'https://fixture.invalid/rootfs', 'rootfs.ext4',
                    'artifacts/rootfs.ext4', ?1, 4, ?2, 4, ?2, 'verified', 1, 1)",
                rusqlite::params![
                    directory
                        .path()
                        .join("rootfs.ext4")
                        .to_string_lossy()
                        .as_ref(),
                    "b".repeat(64)
                ],
            )
            .expect("image download should insert");
        let download_id: i64 = connection
            .query_row(
                "SELECT id FROM downloads WHERE artifact_key = 'distribution_image:distro-1:image-1'",
                [],
                |row| row.get(0),
            )
            .expect("image download should be present");
        connection
            .execute(
                "INSERT INTO distribution_images (
                    distribution_id, download_id, registry_id, name, display_name, description,
                    variant, format, registry_path, registry_url, filename, size_bytes, sha256,
                    mime_type, modified_at, created_at, updated_at
                ) VALUES (?1, ?2, 'image-1', 'minimal', 'Minimal', 'fixture image',
                          'minimal', 'ext4', 'images/rootfs.ext4',
                          'https://fixture.invalid/rootfs', 'rootfs.ext4', 4, ?3,
                          'application/octet-stream', '2026-09-23T00:00:00Z', 1, 1)",
                rusqlite::params![distribution_row, download_id, "b".repeat(64)],
            )
            .expect("distribution image should insert");
        connection
            .execute(
                "INSERT INTO microvms (
                    name, distribution_id, image_id, kernel_id, firecracker_package_id,
                    firectl_package_id, disk_size_bytes, memory_requested_bytes,
                    memory_effective_mib, vcpu_count, volume_path, rootfs_path, socket_path,
                    expose_on_lan, created_at, updated_at
                ) VALUES ('vm-1', 'distro-1', 'image-1', 'kernel-1', 'fc-1', 'firectl-1',
                          4, 134217728, 128, 1, '/tmp/vms/vm-1', '/tmp/vms/vm-1/rootfs.ext4',
                          '/tmp/vms/vm-1/firecracker.sock', 0, 1, 1)",
                [],
            )
            .expect("VM row should insert");
        connection
            .execute(
                "INSERT INTO distribution_boot_args (distribution_id, position, argument)
                 VALUES (?1, 0, 'console=ttyS0'), (?1, 1, 'panic=1')",
                rusqlite::params![distribution_row],
            )
            .expect("boot args should insert");
        let schema_version: i64 = connection
            .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("schema version should be readable");
        drop(connection);

        let metadata = repository
            .snapshot_metadata("vm-1")
            .expect("snapshot metadata should be read from the local inventory");

        assert_eq!(metadata.distribution_name, "Linux Fixture");
        assert_eq!(metadata.distribution_version, "1.0");
        assert_eq!(metadata.root_device, "/dev/vda");
        assert_eq!(metadata.kernel_args, ["console=ttyS0", "panic=1"]);
        assert_eq!(metadata.image_sha256, "b".repeat(64));
        assert_eq!(metadata.guest_architecture, "x86_64");
        assert!(
            schema_version >= 5,
            "restore journal migration should be applied"
        );
    }

    fn seed_binary_repository() -> (TempDir, SqliteRepository) {
        let (directory, repository) = empty_repository();
        let connection = open_connection(&directory.path().join("inventory.db"))
            .expect("inventory connection should open");
        connection
            .execute(
                "INSERT INTO binary_packages (
                    registry_id, name, display_name, description, version, architecture,
                    created_at, updated_at
                ) VALUES ('package-x86_64', 'runtime', 'Runtime', NULL, '1.0.0',
                          'x86_64', 1, 1)",
                [],
            )
            .expect("binary package should insert");
        let package_id: i64 = connection
            .query_row(
                "SELECT id FROM binary_packages WHERE registry_id = 'package-x86_64'",
                [],
                |row| row.get(0),
            )
            .expect("binary package id should be readable");
        let absolute_path = directory
            .path()
            .join("tools/package-x86_64/firecracker/firecracker")
            .to_string_lossy()
            .into_owned();
        connection
            .execute(
                "INSERT INTO downloads (
                    artifact_key, artifact_type, registry_path, registry_url, filename,
                    relative_path, absolute_path, expected_size_bytes, expected_sha256,
                    actual_size_bytes, actual_sha256, verification_status,
                    created_at, updated_at, last_verified_at
                ) VALUES (
                    'binary:package-x86_64:firecracker', 'binary',
                    'binaries/runtime/1.0.0/x86_64/firecracker',
                    'https://example.invalid/firecracker', 'firecracker',
                    'tools/package-x86_64/firecracker/firecracker', ?1, 4, ?2, 4, ?2,
                    'verified', 1, 1, 1
                )",
                rusqlite::params![absolute_path, "a".repeat(64)],
            )
            .expect("binary download should insert");
        let download_id: i64 = connection
            .query_row(
                "SELECT id FROM downloads
                 WHERE artifact_key = 'binary:package-x86_64:firecracker'",
                [],
                |row| row.get(0),
            )
            .expect("binary download id should be readable");
        connection
            .execute(
                "INSERT INTO binary_files (
                    binary_package_id, download_id, component_name, registry_path, registry_url,
                    filename, size_bytes, sha256, mime_type, executable, mode, permissions,
                    format, modified_at, created_at, updated_at
                ) VALUES (?1, ?2, 'firecracker',
                          'binaries/runtime/1.0.0/x86_64/firecracker',
                          'https://example.invalid/firecracker', 'firecracker', 4, ?3,
                          'application/octet-stream', 1, '755', '-rwxr-xr-x',
                          'elf', '2026-09-17T12:00:00Z', 1, 1)",
                rusqlite::params![package_id, download_id, "a".repeat(64)],
            )
            .expect("binary file should insert");
        (directory, repository)
    }

    #[test]
    fn resolves_only_the_complete_package_component_relationship() {
        let (_directory, repository) = seed_binary_repository();

        let installed = repository
            .resolve_binary("package-x86_64", "firecracker")
            .expect("complete binary should resolve");

        assert_eq!(installed.package_id, "package-x86_64");
        assert_eq!(installed.component_name, "firecracker");
        assert_eq!(installed.version, "1.0.0");
        assert!(matches!(
            installed.architecture,
            crate::domain::registry::Architecture::X86_64
        ));
        assert_eq!(installed.size_bytes, 4);
        assert!(installed.executable);
    }

    #[test]
    fn rejects_missing_package_or_component_relationships() {
        let (_directory, repository) = seed_binary_repository();

        assert!(repository.resolve_binary("missing", "firecracker").is_err());
        assert!(
            repository
                .resolve_binary("package-x86_64", "missing")
                .is_err()
        );
    }

    #[test]
    fn does_not_resolve_a_package_without_a_verified_download_join() {
        let (_directory, repository) = empty_repository();
        let connection =
            open_connection(&repository.database_path).expect("inventory connection should open");
        connection
            .execute(
                "INSERT INTO binary_packages (
                    registry_id, name, display_name, description, version, architecture,
                    created_at, updated_at
                ) VALUES ('package-empty', 'runtime', 'Runtime', NULL, '1.0.0',
                          'x86_64', 1, 1)",
                [],
            )
            .expect("binary package should insert");

        assert!(
            repository
                .resolve_binary("package-empty", "firecracker")
                .is_err()
        );
    }

    #[test]
    fn repository_connections_enable_foreign_key_enforcement() {
        let (_directory, repository) = empty_repository();
        let connection = repository
            .connection()
            .expect("inventory connection should open");
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign key pragma should be readable");

        assert_eq!(foreign_keys, 1);
    }
}
