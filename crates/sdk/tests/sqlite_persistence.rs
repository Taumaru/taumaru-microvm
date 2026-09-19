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
                'vm_runtime'
            )",
            [],
            |row| row.get(0),
        )
        .expect("schema table count should be readable");

    assert_eq!(table_count, 14);

    let migration_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("migration ledger should be readable");
    assert_eq!(migration_count, 3);
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
