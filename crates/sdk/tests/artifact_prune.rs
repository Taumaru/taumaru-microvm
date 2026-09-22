mod support;

use std::error::Error;
use std::path::Path;

use rusqlite::{Connection, params};
use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{MicroVmSdk, PrunedImageId, SdkError};

fn open_inventory(home: &Path) -> rusqlite::Result<Connection> {
    Connection::open(home.join("state").join("inventory.db"))
}

fn insert_vm(
    connection: &Connection,
    name: &str,
    distribution_id: &str,
    image_id: &str,
    kernel_id: &str,
) {
    let volume = format!("/tmp/prune-{name}");
    connection
        .execute(
            "INSERT INTO microvms (
                name, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, 'fc', 'firectl',
                      1, 1, 1, 1, ?5, ?6, ?7, 0, 1, 1)",
            params![
                name,
                distribution_id,
                image_id,
                kernel_id,
                volume.clone(),
                format!("{volume}/rootfs.ext4"),
                format!("{volume}/firecracker.sock"),
            ],
        )
        .expect("seed VM row should insert");
}

fn kernel_file(home: &Path) -> std::path::PathBuf {
    home.join("artifacts/kernels/linux-test-x86_64/vmlinux")
}

fn image_file(home: &Path, image: &str) -> std::path::PathBuf {
    home.join(format!(
        "artifacts/rootfs/alpine-test-1.0/{image}/{image}.ext4"
    ))
}

fn download_count(connection: &Connection, key: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM downloads WHERE artifact_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .expect("download count should be readable")
}

async fn seed_kernel_and_images(
    server: &FixtureServer,
    home: &Path,
) -> Result<MicroVmSdk, Box<dyn Error + Send + Sync>> {
    let sdk = MicroVmSdk::with_registry_base_url(home, server.base_url())?;
    sdk.download_kernel("linux-test-x86_64", |_| {}).await?;
    sdk.download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |_| {})
        .await?;
    sdk.download_distribution_image("alpine-test-1.0", "alpine-test-debug", |_| {})
        .await?;
    Ok(sdk)
}

#[tokio::test]
async fn prunes_only_unreferenced_artifacts_and_reports_ordered_bytes()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let connection = open_inventory(home.path())?;
    insert_vm(
        &connection,
        "owner_vm",
        "alpine-test-1.0",
        "alpine-test-minimal",
        "linux-test-x86_64",
    );
    drop(connection);

    let kernel_size = std::fs::metadata(kernel_file(home.path()))?.len();
    let minimal_size = std::fs::metadata(image_file(home.path(), "alpine-test-minimal"))?.len();
    let debug_size = std::fs::metadata(image_file(home.path(), "alpine-test-debug"))?.len();

    let summary = sdk.prune_unused_artifacts().await?;

    assert!(summary.removed_kernels.is_empty());
    assert_eq!(
        summary.removed_images,
        vec![PrunedImageId {
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-debug".to_owned(),
        }]
    );
    assert!(summary.skipped_artifact_keys.is_empty());
    assert_eq!(summary.freed_bytes_kernels, 0);
    assert_eq!(summary.freed_bytes_images, debug_size);
    assert_eq!(summary.freed_bytes_total, debug_size);
    assert!(!image_file(home.path(), "alpine-test-debug").exists());
    assert!(kernel_file(home.path()).is_file());
    assert!(image_file(home.path(), "alpine-test-minimal").is_file());
    assert_eq!(
        std::fs::metadata(kernel_file(home.path()))?.len(),
        kernel_size
    );
    assert_eq!(
        std::fs::metadata(image_file(home.path(), "alpine-test-minimal"))?.len(),
        minimal_size
    );
    let connection = open_inventory(home.path())?;
    assert_eq!(download_count(&connection, "kernel:linux-test-x86_64"), 1);
    assert_eq!(
        download_count(
            &connection,
            "distribution_image:alpine-test-1.0:alpine-test-minimal"
        ),
        1
    );
    assert_eq!(
        download_count(
            &connection,
            "distribution_image:alpine-test-1.0:alpine-test-debug"
        ),
        0
    );
    assert!(
        sdk.is_distribution_image_ready("alpine-test-1.0", "alpine-test-minimal")
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn prunes_everything_when_no_microvm_exists() -> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;

    let summary = sdk.prune_unused_artifacts().await?;

    assert_eq!(
        summary.removed_kernels,
        vec!["linux-test-x86_64".to_owned()]
    );
    assert_eq!(
        summary.removed_images,
        vec![
            PrunedImageId {
                distribution_id: "alpine-test-1.0".to_owned(),
                image_id: "alpine-test-debug".to_owned(),
            },
            PrunedImageId {
                distribution_id: "alpine-test-1.0".to_owned(),
                image_id: "alpine-test-minimal".to_owned(),
            },
        ]
    );
    assert!(summary.skipped_artifact_keys.is_empty());
    assert!(summary.freed_bytes_total > 0);
    assert_eq!(
        summary.freed_bytes_total,
        summary.freed_bytes_kernels + summary.freed_bytes_images
    );
    Ok(())
}

#[tokio::test]
async fn keeps_a_shared_kernel_while_one_owner_remains() -> Result<(), Box<dyn Error + Send + Sync>>
{
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let connection = open_inventory(home.path())?;
    insert_vm(
        &connection,
        "first_vm",
        "alpine-test-1.0",
        "alpine-test-minimal",
        "linux-test-x86_64",
    );
    insert_vm(
        &connection,
        "second_vm",
        "alpine-test-1.0",
        "alpine-test-debug",
        "linux-test-x86_64",
    );
    connection.execute("DELETE FROM microvms WHERE name = 'second_vm'", [])?;
    drop(connection);

    let summary = sdk.prune_unused_artifacts().await?;

    assert!(summary.removed_kernels.is_empty());
    assert_eq!(
        summary.removed_images,
        vec![PrunedImageId {
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-debug".to_owned(),
        }]
    );
    assert!(kernel_file(home.path()).is_file());
    Ok(())
}

#[tokio::test]
async fn ignores_metadata_kernel_rows_never_downloaded_and_stray_files()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let connection = open_inventory(home.path())?;
    insert_vm(
        &connection,
        "ghost_vm",
        "alpine-test-1.0",
        "never-downloaded",
        "linux-test-aarch64",
    );
    drop(connection);
    let stray = home.path().join("artifacts/kernels/stray-dir/stray-file");
    std::fs::create_dir_all(stray.parent().expect("stray parent should exist"))?;
    std::fs::write(&stray, b"stray")?;

    let summary = sdk.prune_unused_artifacts().await?;

    assert_eq!(
        summary.removed_kernels,
        vec!["linux-test-x86_64".to_owned()]
    );
    assert!(stray.is_file());
    let connection = open_inventory(home.path())?;
    let metadata_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM kernels WHERE registry_id = 'linux-test-aarch64'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(metadata_count, 1);
    Ok(())
}

#[tokio::test]
async fn reclaims_orphan_rows_and_drops_stale_records_at_zero_bytes()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let connection = open_inventory(home.path())?;
    let orphan_file = home.path().join("artifacts/kernels/orphan-kernel/vmlinux");
    std::fs::create_dir_all(orphan_file.parent().expect("orphan parent should exist"))?;
    std::fs::write(&orphan_file, b"orph")?;
    connection.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES ('kernel:orphan-kernel', 'kernel', 'kernels/x', 'https://example.invalid/x',
                  'vmlinux', 'artifacts/kernels/orphan-kernel/vmlinux', ?1, 4, ?2, 4, ?2,
                  'verified', 1, 1, 1)",
        params![orphan_file.to_string_lossy().into_owned(), "0".repeat(64)],
    )?;
    drop(connection);
    std::fs::remove_file(image_file(home.path(), "alpine-test-debug"))?;

    let summary = sdk.prune_unused_artifacts().await?;

    assert!(
        summary
            .removed_kernels
            .contains(&"orphan-kernel".to_owned())
    );
    assert!(
        summary
            .removed_kernels
            .contains(&"linux-test-x86_64".to_owned())
    );
    assert_eq!(
        summary.removed_images,
        vec![
            PrunedImageId {
                distribution_id: "alpine-test-1.0".to_owned(),
                image_id: "alpine-test-debug".to_owned(),
            },
            PrunedImageId {
                distribution_id: "alpine-test-1.0".to_owned(),
                image_id: "alpine-test-minimal".to_owned(),
            },
        ]
    );
    Ok(())
}

#[tokio::test]
async fn noop_and_repeat_report_zero_removals_without_writes()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let connection = open_inventory(home.path())?;
    insert_vm(
        &connection,
        "owner_vm",
        "alpine-test-1.0",
        "alpine-test-minimal",
        "linux-test-x86_64",
    );
    insert_vm(
        &connection,
        "debug_vm",
        "alpine-test-1.0",
        "alpine-test-debug",
        "linux-test-x86_64",
    );
    drop(connection);

    let first = sdk.prune_unused_artifacts().await?;
    assert!(first.removed_kernels.is_empty());
    assert!(first.removed_images.is_empty());
    assert!(first.skipped_artifact_keys.is_empty());
    assert_eq!(first.freed_bytes_total, 0);
    let second = sdk.prune_unused_artifacts().await?;
    assert!(second.removed_kernels.is_empty());
    assert!(second.removed_images.is_empty());
    assert_eq!(second.freed_bytes_total, 0);

    let empty_home = tempdir()?;
    let empty_sdk = MicroVmSdk::new(empty_home.path())?;
    let empty = empty_sdk.prune_unused_artifacts().await?;
    assert!(empty.removed_kernels.is_empty());
    assert!(empty.removed_images.is_empty());
    assert_eq!(empty.freed_bytes_total, 0);
    Ok(())
}

#[tokio::test]
async fn partial_failure_keeps_successes_reports_causes_and_repairs_on_retry()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let debug_dir = image_file(home.path(), "alpine-test-debug")
        .parent()
        .expect("debug parent should exist")
        .to_path_buf();
    let debug_file = image_file(home.path(), "alpine-test-debug");
    let debug_bytes = std::fs::read(&debug_file)?;
    std::fs::remove_file(&debug_file)?;
    std::fs::create_dir_all(&debug_file)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&debug_file, std::fs::Permissions::from_mode(0o555))?;

    let error = sdk
        .prune_unused_artifacts()
        .await
        .expect_err(" undeletable debug path should fail");
    let SdkError::PruneIncomplete { summary, failures } = error else {
        panic!("expected PruneIncomplete, got a different error");
    };
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].artifact_key,
        "distribution_image:alpine-test-1.0:alpine-test-debug"
    );
    assert!(!failures[0].reason.is_empty());
    assert_eq!(
        summary.removed_kernels,
        vec!["linux-test-x86_64".to_owned()]
    );
    assert_eq!(
        summary.removed_images,
        vec![PrunedImageId {
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-minimal".to_owned(),
        }]
    );
    assert!(summary.freed_bytes_total > 0);
    assert!(!kernel_file(home.path()).exists());

    std::fs::remove_dir_all(&debug_file)?;
    std::fs::create_dir_all(&debug_dir)?;
    std::fs::write(&debug_file, debug_bytes)?;
    let retry = sdk.prune_unused_artifacts().await?;
    assert_eq!(
        retry.removed_images,
        vec![PrunedImageId {
            distribution_id: "alpine-test-1.0".to_owned(),
            image_id: "alpine-test-debug".to_owned(),
        }]
    );
    assert!(!debug_file.exists());
    let repeat = sdk.prune_unused_artifacts().await?;
    assert!(repeat.removed_kernels.is_empty());
    assert!(repeat.removed_images.is_empty());
    Ok(())
}

#[tokio::test]
async fn unparseable_orphan_keys_become_failures_with_rows_kept()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let connection = open_inventory(home.path())?;
    connection.execute(
        "INSERT INTO downloads (
            artifact_key, artifact_type, registry_path, registry_url, filename,
            relative_path, absolute_path, expected_size_bytes, expected_sha256,
            actual_size_bytes, actual_sha256, verification_status, created_at,
            updated_at, last_verified_at
        ) VALUES ('bogus-key-without-prefix', 'kernel', 'kernels/x',
                  'https://example.invalid/x', 'vmlinux', 'artifacts/kernels/bogus/vmlinux',
                  ?1, 1, ?2, 1, ?2, 'verified', 1, 1, 1)",
        params![
            home.path()
                .join("artifacts/kernels/bogus/vmlinux")
                .to_string_lossy()
                .into_owned(),
            "0".repeat(64)
        ],
    )?;
    drop(connection);

    let error = sdk
        .prune_unused_artifacts()
        .await
        .expect_err("unparseable orphan should fail");
    let SdkError::PruneIncomplete { failures, .. } = error else {
        panic!("expected PruneIncomplete, got a different error");
    };
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].artifact_key, "bogus-key-without-prefix");
    let connection = open_inventory(home.path())?;
    assert_eq!(download_count(&connection, "bogus-key-without-prefix"), 1);
    Ok(())
}

#[tokio::test]
async fn prune_is_silent_on_success_and_partial_failure() -> Result<(), Box<dyn Error + Send + Sync>>
{
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = seed_kernel_and_images(&server, home.path()).await?;
    let summary = sdk.prune_unused_artifacts().await?;
    assert!(!summary.removed_kernels.is_empty());
    let repeat = sdk.prune_unused_artifacts().await?;
    assert!(repeat.removed_kernels.is_empty());
    Ok(())
}
