use std::convert::TryFrom;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::error::SdkError;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_artifact_inventory.sql",
    sql: include_str!("../../../migrations/0001_artifact_inventory.sql"),
}];

pub(crate) fn apply_pending(connection: &mut Connection) -> Result<(), SdkError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL CHECK (
                length(checksum) = 64
                AND checksum = lower(checksum)
                AND checksum NOT GLOB '*[^0-9a-f]*'
            ),
            applied_at INTEGER NOT NULL
        )",
    )?;
    verify_table_columns(
        connection,
        "schema_migrations",
        &["version", "name", "checksum", "applied_at"],
    )?;

    for migration in MIGRATIONS {
        let expected_checksum = checksum(migration.sql);
        let applied = connection
            .query_row(
                "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
                params![migration.version],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        match applied {
            Some((name, actual_checksum)) => {
                if name != migration.name || actual_checksum != expected_checksum {
                    return Err(SdkError::Migration(format!(
                        "migration {} drifted: recorded name/checksum do not match {}",
                        migration.version, migration.name
                    )));
                }
            }
            None => {
                let transaction = connection.transaction()?;
                transaction.execute_batch(migration.sql)?;
                let applied_at = unix_timestamp()?;
                transaction.execute(
                    "INSERT INTO schema_migrations (version, name, checksum, applied_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        migration.version,
                        migration.name,
                        expected_checksum,
                        applied_at
                    ],
                )?;
                transaction.commit()?;
            }
        }
    }

    verify_required_schema(connection)
}

pub(crate) fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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

fn verify_required_schema(connection: &Connection) -> Result<(), SdkError> {
    const REQUIRED_TABLES: &[(&str, &[&str])] = &[
        (
            "schema_migrations",
            &["version", "name", "checksum", "applied_at"],
        ),
        (
            "downloads",
            &[
                "id",
                "artifact_key",
                "artifact_type",
                "registry_path",
                "registry_url",
                "filename",
                "relative_path",
                "absolute_path",
                "expected_size_bytes",
                "expected_sha256",
                "actual_size_bytes",
                "actual_sha256",
                "verification_status",
                "created_at",
                "updated_at",
                "last_verified_at",
            ],
        ),
        ("kernels", &["id", "registry_id", "download_id"]),
        ("binary_packages", &["id", "registry_id"]),
        (
            "binary_files",
            &["id", "binary_package_id", "download_id", "component_name"],
        ),
        ("distributions", &["id", "registry_id"]),
        (
            "distribution_boot_args",
            &["distribution_id", "position", "argument"],
        ),
        (
            "distribution_images",
            &["id", "distribution_id", "download_id", "registry_id"],
        ),
        (
            "distribution_kernels",
            &["distribution_id", "kernel_id", "is_default"],
        ),
        (
            "elf_metadata",
            &["download_id", "class", "endianness", "elf_type"],
        ),
        (
            "elf_needed_libraries",
            &["download_id", "position", "library"],
        ),
        (
            "image_filesystems",
            &["distribution_image_id", "filesystem_type"],
        ),
        (
            "image_filesystem_features",
            &["image_filesystem_id", "position", "feature"],
        ),
        (
            "image_capabilities",
            &["distribution_image_id", "position", "capability"],
        ),
    ];

    for (table, columns) in REQUIRED_TABLES {
        verify_table_columns(connection, table, columns)?;
    }

    Ok(())
}

fn verify_table_columns(
    connection: &Connection,
    table: &str,
    required_columns: &[&str],
) -> Result<(), SdkError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info('{table}')"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        return Err(SdkError::Migration(format!(
            "required inventory table {table} is missing"
        )));
    }
    for required_column in required_columns {
        if !columns.iter().any(|column| column == required_column) {
            return Err(SdkError::Migration(format!(
                "required inventory column {table}.{required_column} is missing"
            )));
        }
    }
    Ok(())
}
