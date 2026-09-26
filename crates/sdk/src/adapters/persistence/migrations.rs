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

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_artifact_inventory.sql",
        sql: include_str!("../../../migrations/0001_artifact_inventory.sql"),
    },
    Migration {
        version: 2,
        name: "0002_microvm_creation.sql",
        sql: include_str!("../../../migrations/0002_microvm_creation.sql"),
    },
    Migration {
        version: 3,
        name: "0003_routed_lan.sql",
        sql: include_str!("../../../migrations/0003_routed_lan.sql"),
    },
    Migration {
        version: 4,
        name: "0004_drop_microvm_state.sql",
        sql: include_str!("../../../migrations/0004_drop_microvm_state.sql"),
    },
    Migration {
        version: 5,
        name: "0005_snapshot_restore.sql",
        sql: include_str!("../../../migrations/0005_snapshot_restore.sql"),
    },
    Migration {
        version: 6,
        name: "0006_microvm_autostart.sql",
        sql: include_str!("../../../migrations/0006_microvm_autostart.sql"),
    },
];

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
        (
            "vm_snapshot_metadata",
            &[
                "microvm_id",
                "distribution_name",
                "distribution_version",
                "root_device",
                "kernel_args_json",
                "image_sha256",
                "guest_architecture",
            ],
        ),
        (
            "restore_journal",
            &[
                "operation_id",
                "vm_name",
                "staging_path",
                "volume_path",
                "volume_created",
                "kernel_path",
                "kernel_created",
                "network_json",
                "progress_state",
                "updated_at",
            ],
        ),
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
        (
            "network_bridges",
            &[
                "id",
                "bridge_name",
                "uplink_name",
                "ownership",
                "reference_count",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "microvms",
            &[
                "id",
                "name",
                "distribution_id",
                "image_id",
                "kernel_id",
                "firecracker_package_id",
                "firectl_package_id",
                "disk_size_bytes",
                "memory_requested_bytes",
                "memory_effective_mib",
                "vcpu_count",
                "volume_path",
                "rootfs_path",
                "socket_path",
                "expose_on_lan",
            ],
        ),
        (
            "vm_networks",
            &[
                "id",
                "microvm_id",
                "mode",
                "guest_ip",
                "prefix_length",
                "gateway_ip",
                "host_ip",
                "tap_name",
                "guest_mac",
                "bridge_id",
                "uplink_name",
                "dhcp_lease_reference",
                "desired_boot_parameters",
                "bridge_created_by_sdk",
                "uplink_attached_by_sdk",
                "forwarding_enabled_by_sdk",
                "nat_table_created_by_sdk",
                "nat_chain_created_by_sdk",
                "host_address_specs",
                "default_route_specs",
                "lan_ip",
                "uplink_cidr",
                "proxy_arp_enabled_by_sdk",
                "host_route_created_by_sdk",
                "proxy_arp_entry_created_by_sdk",
                "iptables_forward_specs",
                "iptables_nat_spec",
            ],
        ),
        (
            "vm_network_resources",
            &[
                "id",
                "microvm_id",
                "resource_kind",
                "resource_identity",
                "desired_fingerprint",
                "ownership",
                "adapter_handle",
                "last_observed",
            ],
        ),
        (
            "vm_credentials",
            &[
                "microvm_id",
                "private_key_path",
                "public_key_path",
                "guest_authorized_keys_path",
                "key_type",
                "ssh_user",
                "ssh_port",
                "public_key_fingerprint",
                "file_mode",
            ],
        ),
        (
            "vm_runtime",
            &[
                "microvm_id",
                "firecracker_path",
                "firectl_path",
                "socket_path",
                "process_id",
                "process_state",
                "updated_at",
            ],
        ),
        (
            "vm_autostart",
            &[
                "microvm_id",
                "enabled",
                "max_start_attempts",
                "created_at",
                "updated_at",
            ],
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
