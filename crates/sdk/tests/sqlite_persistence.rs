mod support;

use std::path::Path;

use rusqlite::{Connection, params};
use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{MicroVmSdk, SdkError};

fn open_inventory(home: &Path) -> rusqlite::Result<Connection> {
    Connection::open(home.join("state").join("inventory.db"))
}

fn insert_download(connection: &Connection, key: &str, path: &str) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES (?1, 'kernel', 'kernels/test/file', 'https://example.invalid/file',
                  'file', ?2, ?3, 1, ?4, 1, ?4, 'verified', 1, 1, 1)",
        params![key, path, format!("/tmp/{path}"), "0".repeat(64)],
    )
}

#[test]
fn constructor_creates_idempotent_inventory_schema() {
    let directory = tempdir().expect("temporary directory should be created");
    let first = MicroVmSdk::new(directory.path()).expect("first SDK construction should work");
    drop(first);
    let connection = open_inventory(directory.path()).expect("inventory should open");
    insert_download(&connection, "kernel:preserved", "kernels/preserved")
        .expect("existing inventory row should insert");
    drop(connection);
    let second = MicroVmSdk::new(directory.path()).expect("second SDK construction should work");
    drop(second);

    let connection = open_inventory(directory.path()).expect("inventory should open");
    let table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN (
                'schema_migrations', 'downloads', 'kernels', 'binary_packages',
                'binary_files', 'distributions', 'distribution_images',
                'distribution_kernels', 'network_bridges', 'microvms',
                'vm_networks', 'vm_network_resources', 'vm_credentials',
                'vm_runtime', 'vm_autostart'
            )",
            [],
            |row| row.get(0),
        )
        .expect("schema table count should be readable");

    assert_eq!(table_count, 15);

    let migration_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("migration ledger should be readable");
    assert_eq!(migration_count, 6);
    let preserved_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM downloads WHERE artifact_key = 'kernel:preserved'",
            [],
            |row| row.get(0),
        )
        .expect("existing inventory row should remain readable");
    assert_eq!(preserved_count, 1);
}

#[test]
fn runtime_disk_mappings_stay_transient_across_sdk_reopen() {
    let directory = tempdir().expect("temporary directory should be created");
    let sdk = MicroVmSdk::new(directory.path()).expect("SDK construction should work");
    let volume = directory.path().join("vms").join("transient_vm");
    std::fs::create_dir_all(&volume).expect("VM volume should be created");
    let rootfs_path = volume.join("rootfs.ext4");
    std::fs::write(&rootfs_path, b"persistent root disk").expect("root disk should be written");
    let socket_path = volume.join("firecracker.sock");
    let connection = open_inventory(directory.path()).expect("inventory should open");
    connection
        .execute(
            "INSERT INTO microvms (
                name, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan,
                created_at, updated_at
             ) VALUES (
                'transient_vm', 'distribution', 'image', 'kernel',
                'firecracker', 'firectl', 20, 134217728, 128, 1,
                ?1, ?2, ?3, 0, 1, 1
             )",
            params![
                volume.to_string_lossy(),
                rootfs_path.to_string_lossy(),
                socket_path.to_string_lossy()
            ],
        )
        .expect("VM inventory row should insert");
    let vm_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO vm_runtime (
                microvm_id, firecracker_path, firectl_path, socket_path,
                process_id, process_state, updated_at
             ) VALUES (?1, '/tools/firecracker', '/tools/firectl', ?2, NULL, 'stopped', 1)",
            params![vm_id, socket_path.to_string_lossy()],
        )
        .expect("runtime row should insert");
    drop(connection);
    drop(sdk);

    let reopened = MicroVmSdk::new(directory.path()).expect("SDK home should reopen");
    drop(reopened);
    let connection = open_inventory(directory.path()).expect("inventory should reopen");
    let runtime_columns: Vec<String> = {
        let mut statement = connection
            .prepare("SELECT name FROM pragma_table_info('vm_runtime') ORDER BY cid")
            .expect("runtime columns should be queryable");
        statement
            .query_map([], |row| row.get(0))
            .expect("runtime columns should load")
            .collect::<Result<Vec<_>, _>>()
            .expect("runtime column names should decode")
    };
    assert_eq!(
        runtime_columns,
        [
            "microvm_id",
            "firecracker_path",
            "firectl_path",
            "socket_path",
            "process_id",
            "process_state",
            "updated_at",
        ]
    );
    let persisted_rootfs: String = connection
        .query_row(
            "SELECT rootfs_path FROM microvms WHERE name = 'transient_vm'",
            [],
            |row| row.get(0),
        )
        .expect("rootfs inventory path should survive reopening");
    assert_eq!(persisted_rootfs, rootfs_path.to_string_lossy());
    assert!(rootfs_path.is_file());
    assert!(!runtime_columns.iter().any(|column| {
        let column = column.to_ascii_lowercase();
        column.contains("mapper") || column.contains("loop")
    }));
}

#[test]
fn constructor_rejects_migration_checksum_drift_without_dropping_the_database() {
    let directory = tempdir().expect("temporary directory should be created");
    std::fs::create_dir_all(directory.path().join("state"))
        .expect("state directory should be created");
    let connection = open_inventory(directory.path()).expect("inventory should open");
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                checksum TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            );
            INSERT INTO schema_migrations (version, name, checksum, applied_at)
            VALUES (1, '0001_artifact_inventory.sql',
                    '0000000000000000000000000000000000000000000000000000000000000000', 1);",
        )
        .expect("drift fixture should be created");
    drop(connection);

    let result = MicroVmSdk::new(directory.path());

    assert!(matches!(
        result,
        Err(taumaru_microvm::SdkError::Migration(_))
    ));
    let connection = open_inventory(directory.path()).expect("inventory should remain readable");
    let migration_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("migration ledger should remain readable");
    assert_eq!(migration_count, 1);
}

#[test]
fn inventory_rejects_duplicate_artifact_and_absolute_paths() {
    let directory = tempdir().expect("temporary directory should be created");
    let sdk = MicroVmSdk::new(directory.path()).expect("SDK construction should work");
    drop(sdk);
    let connection = open_inventory(directory.path()).expect("inventory should open");

    insert_download(&connection, "kernel:first", "kernels/first").expect("first row should insert");
    assert!(insert_download(&connection, "kernel:first", "kernels/second").is_err());
    assert!(insert_download(&connection, "kernel:second", "kernels/first").is_err());
}

#[test]
fn inventory_rejects_negative_sizes_and_invalid_verification_rows() {
    let directory = tempdir().expect("temporary directory should be created");
    let sdk = MicroVmSdk::new(directory.path()).expect("SDK construction should work");
    drop(sdk);
    let connection = open_inventory(directory.path()).expect("inventory should open");

    let negative_size = connection.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES ('kernel:negative', 'kernel', 'kernels/test/file',
                  'https://example.invalid/file', 'file', 'kernels/negative',
                  '/tmp/kernels/negative', -1, ?1, 1, ?1, 'verified', 1, 1, 1)",
        params!["0".repeat(64)],
    );
    assert!(negative_size.is_err());

    let null_actual = connection.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES ('kernel:null-actual', 'kernel', 'kernels/test/file',
                  'https://example.invalid/file', 'file', 'kernels/null-actual',
                  '/tmp/kernels/null-actual', 1, ?1, NULL, NULL, 'verified', 1, 1, NULL)",
        params!["0".repeat(64)],
    );
    assert!(null_actual.is_err());
}

#[tokio::test]
async fn persists_each_member_and_distribution_relationship_in_normalized_tables()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let directory = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(directory.path(), server.base_url())?;

    sdk.download_distribution("alpine-test-1.0", |_| {}).await?;
    sdk.download_binary("firecracker-test-1.0.0-x86_64", |_| {})
        .await?;

    let connection = open_inventory(directory.path())?;
    let image_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM distribution_images", [], |row| {
            row.get(0)
        })?;
    let image_sizes: Vec<i64> = {
        let mut statement = connection
            .prepare("SELECT size_bytes FROM distribution_images ORDER BY registry_id")?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let distribution_kernel_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM distribution_kernels", [], |row| {
            row.get(0)
        })?;
    let default_kernel_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM distribution_kernels WHERE is_default = 1",
        [],
        |row| row.get(0),
    )?;
    let boot_args: Vec<String> = {
        let mut statement = connection.prepare(
            "SELECT argument FROM distribution_boot_args
             ORDER BY distribution_id, position",
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let binary_file_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM binary_files", [], |row| row.get(0))?;
    let binary_download_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM downloads WHERE artifact_type = 'binary'",
        [],
        |row| row.get(0),
    )?;
    let downloaded_kernel_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM kernels WHERE download_id IS NOT NULL",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(image_count, 2);
    assert_eq!(image_sizes, [22, 15]);
    assert_eq!(distribution_kernel_count, 2);
    assert_eq!(default_kernel_count, 1);
    assert_eq!(boot_args, ["console=ttyS0", "root=/dev/vda", "rw"]);
    assert_eq!(binary_file_count, 2);
    assert_eq!(binary_download_count, 2);
    assert_eq!(downloaded_kernel_count, 0);

    sdk.download_kernel("linux-test-x86_64", |_| {}).await?;
    let downloaded_kernel_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM kernels WHERE download_id IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let retained_relationship_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM distribution_kernels", [], |row| {
            row.get(0)
        })?;
    assert_eq!(downloaded_kernel_count, 1);
    assert_eq!(retained_relationship_count, 2);
    Ok(())
}

#[tokio::test]
async fn resolves_distinct_binary_versions_and_architectures_after_restart()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let directory = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(directory.path(), server.base_url())?;
    sdk.download_binary("firecracker-test-1.0.0-x86_64", |_| {})
        .await?;
    sdk.download_binary("firecracker-test-1.0.0-aarch64", |_| {})
        .await?;
    drop(sdk);

    let restarted = MicroVmSdk::with_registry_base_url(directory.path(), server.base_url())?;
    let x86_64 = restarted
        .resolve_binary("firecracker-test-1.0.0-x86_64", "firecracker")
        .await?;
    let aarch64 = restarted
        .resolve_binary("firecracker-test-1.0.0-aarch64", "firecracker")
        .await?;
    let jailer = restarted
        .resolve_binary("firecracker-test-1.0.0-x86_64", "jailer")
        .await?;

    assert_ne!(x86_64.path, aarch64.path);
    assert_ne!(x86_64.path, jailer.path);
    assert_eq!(x86_64.package_id, "firecracker-test-1.0.0-x86_64");
    assert_eq!(x86_64.component_name, "firecracker");
    assert_eq!(jailer.component_name, "jailer");
    Ok(())
}

#[tokio::test]
async fn invalid_kernel_cleanup_removes_its_physical_and_relation_rows()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let directory = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(directory.path(), server.base_url())?;
    sdk.download_distribution("alpine-test-1.0", |_| {}).await?;
    sdk.download_kernel("linux-test-x86_64", |_| {}).await?;
    std::fs::write(
        directory
            .path()
            .join("artifacts/kernels/linux-test-x86_64/vmlinux"),
        b"corrupt",
    )?;
    server.set_kernel_payload(b"bad-replacement".to_vec())?;

    let result = sdk.download_kernel("linux-test-x86_64", |_| {}).await;
    assert!(matches!(result, Err(SdkError::IntegrityMismatch { .. })));

    let connection = open_inventory(directory.path())?;
    let kernel_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM kernels WHERE registry_id = 'linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    let relation_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM distribution_kernels dk
         JOIN kernels k ON k.id = dk.kernel_id
         WHERE k.registry_id = 'linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(kernel_count, 0);
    assert_eq!(relation_count, 0);
    Ok(())
}

#[test]
fn migration_v4_removes_the_persisted_state_column_and_keeps_vm_data() {
    use sha2::{Digest, Sha256};
    let directory = tempdir().expect("temporary directory should be created");
    std::fs::create_dir_all(directory.path().join("state"))
        .expect("state directory should be created");
    let database = directory.path().join("state").join("inventory.db");
    let names = [
        "0001_artifact_inventory.sql",
        "0002_microvm_creation.sql",
        "0003_routed_lan.sql",
    ];
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let connection = Connection::open(&database).expect("inventory should open");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                checksum TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            )",
        )
        .expect("ledger should exist");
    for (index, name) in names.iter().enumerate() {
        let version = index as i64 + 1;
        let sql = std::fs::read_to_string(crate_dir.join("migrations").join(name))
            .unwrap_or_else(|_| panic!("{name} should be readable"));
        connection
            .execute_batch(&sql)
            .expect("seed migration should apply");
        let mut digest = Sha256::new();
        digest.update(sql.as_bytes());
        let checksum: String = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name, checksum, applied_at)
                 VALUES (?1, ?2, ?3, 1)",
                rusqlite::params![version, name, checksum],
            )
            .expect("ledger row should insert");
    }
    connection
        .execute(
            "INSERT INTO microvms (
                name, state, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
            ) VALUES ('legacy-vm', 'running', 'd', 'i', 'k', 'fc', 'fr',
                      8, 134217728, 128, 1, '/tmp/legacy', '/tmp/legacy/rootfs.ext4',
                      '/tmp/legacy/firecracker.sock', 0, 1, 1)",
            [],
        )
        .expect("legacy VM row should insert");
    drop(connection);
    let sdk = MicroVmSdk::new(directory.path()).expect("v4 migration should apply");
    drop(sdk);
    let connection = open_inventory(directory.path()).expect("inventory should open");
    let state_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('microvms') WHERE name = 'state'",
            [],
            |row| row.get(0),
        )
        .expect("microvms columns should be readable");
    assert_eq!(state_columns, 0);
    let migration_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("migration ledger should be readable");
    assert_eq!(migration_count, 6);
    let vm_name: String = connection
        .query_row(
            "SELECT name FROM microvms WHERE volume_path = '/tmp/legacy'",
            [],
            |row| row.get(0),
        )
        .expect("legacy VM data should survive");
    assert_eq!(vm_name, "legacy-vm");
}
