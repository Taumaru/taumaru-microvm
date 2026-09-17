use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::artifact::{ArtifactKind, DownloadSpec, FileIntegrity, InstalledBinary};
use crate::domain::registry::{
    Architecture, BinaryFile, BinaryPackage, Distribution, DistributionImage, ElfMetadata,
    Endianness, Kernel, Linkage,
};
use crate::error::SdkError;
use crate::ports::repository::{ArtifactRepository, InventoryState};

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
        let physical_match = integrity.is_some_and(|value| {
            artifact_type == artifact_type_name(&spec.artifact_kind)
                && registry_path == spec.registry_path
                && registry_url == spec.registry_url
                && filename == spec.filename
                && relative_path == expected_relative_path
                && absolute_path == expected_absolute_path
                && value.size_bytes == spec.expected_size
                && value.sha256 == spec.expected_sha256
                && value.size_bytes == expected_size
                && value.sha256 == expected_sha256
                && actual_size == value.size_bytes
                && actual_sha256 == value.sha256
                && status == "verified"
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
}

pub(crate) fn open_connection(database_path: &Path) -> Result<Connection, SdkError> {
    let connection = Connection::open(database_path)?;
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(connection)
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::ports::repository::ArtifactRepository;

    use super::{SqliteRepository, open_connection};

    fn empty_repository() -> (TempDir, SqliteRepository) {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let database_path = directory.path().join("inventory.db");
        let repository =
            SqliteRepository::initialize(database_path).expect("repository should initialize");
        (directory, repository)
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
