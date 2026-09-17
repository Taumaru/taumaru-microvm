mod support;

use std::error::Error;

use rusqlite::Connection;
use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{DownloadCancellation, DownloadPhase, MicroVmSdk, SdkError};

#[tokio::test]
async fn rejects_a_truncated_transfer_without_publishing_or_recording_it()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start_with_kernel_payload(b"truncated".to_vec()).await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let error = sdk
        .download_kernel("linux-test-x86_64", |_| {})
        .await
        .expect_err("truncated payload should fail verification");

    assert!(matches!(error, SdkError::IntegrityMismatch { .. }));
    assert!(
        !home
            .path()
            .join("artifacts/kernels/linux-test-x86_64/vmlinux")
            .exists()
    );
    let connection = Connection::open(home.path().join("state/inventory.db"))?;
    let download_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM downloads WHERE artifact_key = 'kernel:linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(download_count, 0);
    Ok(())
}

#[tokio::test]
async fn removes_a_wrong_target_before_a_failed_replacement()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start_with_kernel_payload(b"wrong".to_vec()).await?;
    let home = tempdir()?;
    let target = home
        .path()
        .join("artifacts/kernels/linux-test-x86_64/vmlinux");
    std::fs::create_dir_all(target.parent().ok_or("target parent is missing")?)?;
    std::fs::write(&target, b"old-invalid-file")?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let result = sdk.download_kernel("linux-test-x86_64", |_| {}).await;

    assert!(matches!(result, Err(SdkError::IntegrityMismatch { .. })));
    assert!(!target.exists());
    let connection = Connection::open(home.path().join("state/inventory.db"))?;
    let row_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM downloads WHERE artifact_key = 'kernel:linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(row_count, 0);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn removes_an_invalid_symlink_target_before_replacement()
-> Result<(), Box<dyn Error + Send + Sync>> {
    use std::os::unix::fs::symlink;

    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let target = home
        .path()
        .join("artifacts/kernels/linux-test-x86_64/vmlinux");
    let outside_target = home.path().join("outside-kernel");
    std::fs::create_dir_all(target.parent().ok_or("target parent is missing")?)?;
    std::fs::write(&outside_target, b"outside-target")?;
    symlink(&outside_target, &target)?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    sdk.download_kernel("linux-test-x86_64", |_| {}).await?;

    let metadata = std::fs::symlink_metadata(&target)?;
    assert!(metadata.is_file());
    assert!(!metadata.file_type().is_symlink());
    assert_eq!(std::fs::read(&outside_target)?, b"outside-target");
    Ok(())
}

#[tokio::test]
async fn a_callback_does_not_change_the_download_failure_contract()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start_with_kernel_payload(b"bad".to_vec()).await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let mut callback_events = 0_u64;

    let error = sdk
        .download_kernel("linux-test-x86_64", |_| callback_events += 1)
        .await
        .expect_err("bad payload should fail regardless of callback");

    assert!(matches!(error, SdkError::IntegrityMismatch { .. }));
    assert!(callback_events > 0);
    Ok(())
}

#[tokio::test]
async fn rejects_deleted_or_replaced_binary_files_during_resolution()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let downloaded = sdk
        .download_binary("firecracker-test-1.0.0-x86_64", |_| {})
        .await?;
    let firecracker = downloaded
        .files
        .iter()
        .find(|file| file.member_name.as_deref() == Some("firecracker"))
        .ok_or("firecracker result is missing")?;

    std::fs::remove_file(&firecracker.absolute_path)?;
    let deleted = sdk
        .resolve_binary("firecracker-test-1.0.0-x86_64", "firecracker")
        .await
        .expect_err("deleted binary should be stale");
    assert!(matches!(deleted, SdkError::StaleBinary { .. }));

    std::fs::write(&firecracker.absolute_path, b"replacement")?;
    let replaced = sdk
        .resolve_binary("firecracker-test-1.0.0-x86_64", "firecracker")
        .await
        .expect_err("replaced binary should fail integrity validation");
    assert!(matches!(replaced, SdkError::IntegrityMismatch { .. }));
    Ok(())
}

#[tokio::test]
async fn removes_invalid_downloaded_kernel_records_and_compatibility_links()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    sdk.download_distribution("alpine-test-1.0", |_| {}).await?;
    sdk.download_kernel("linux-test-x86_64", |_| {}).await?;
    std::fs::write(
        home.path()
            .join("artifacts/kernels/linux-test-x86_64/vmlinux"),
        b"corrupted-on-disk",
    )?;
    server.set_kernel_payload(b"invalid-kernel".to_vec())?;

    let result = sdk.download_kernel("linux-test-x86_64", |_| {}).await;

    assert!(matches!(result, Err(SdkError::IntegrityMismatch { .. })));
    let connection = Connection::open(home.path().join("state/inventory.db"))?;
    let kernel_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM kernels WHERE registry_id = 'linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    let download_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM downloads WHERE artifact_key = 'kernel:linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;
    let compatibility_count: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM distribution_kernels dk
         JOIN kernels k ON k.id = dk.kernel_id
         WHERE k.registry_id = 'linux-test-x86_64'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(kernel_count, 0);
    assert_eq!(download_count, 0);
    assert_eq!(compatibility_count, 0);
    assert!(
        !home
            .path()
            .join("artifacts/kernels/linux-test-x86_64/vmlinux")
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn does_not_keep_a_binary_inventory_row_after_invalid_transfer()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start_with_binary_payload(b"invalid-binary".to_vec()).await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let result = sdk
        .download_binary("firecracker-test-1.0.0-x86_64", |_| {})
        .await;

    assert!(matches!(result, Err(SdkError::IntegrityMismatch { .. })));
    let connection = Connection::open(home.path().join("state/inventory.db"))?;
    let download_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM downloads WHERE artifact_type = 'binary'",
        [],
        |row| row.get(0),
    )?;
    let binary_file_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM binary_files", [], |row| row.get(0))?;
    assert_eq!(download_count, 0);
    assert_eq!(binary_file_count, 0);
    Ok(())
}

#[test]
fn reports_a_typed_filesystem_error_for_a_file_used_as_sdk_home() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let file_home = directory.path().join("not-a-directory");
    std::fs::write(&file_home, b"file")?;

    let result = MicroVmSdk::new(&file_home);

    assert!(matches!(result, Err(SdkError::Filesystem { .. })));
    Ok(())
}

#[tokio::test]
async fn cancellation_emits_a_typed_terminal_phase_and_leaves_no_partial_file()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let cancellation = DownloadCancellation::new();
    let callback_cancellation = cancellation.clone();
    let mut phases = Vec::new();

    let result = sdk
        .download_kernel_with_cancellation("linux-test-x86_64", &cancellation, |event| {
            phases.push(event.phase.clone());
            if event.phase == DownloadPhase::Downloading {
                callback_cancellation.cancel();
            }
        })
        .await;

    assert!(matches!(result, Err(SdkError::Cancelled)));
    assert!(phases.contains(&DownloadPhase::Cancelled));
    assert!(
        !home
            .path()
            .join("artifacts/kernels/linux-test-x86_64/vmlinux")
            .exists()
    );
    assert!(!home.path().join("tmp").read_dir()?.any(|entry| {
        entry.ok().is_some_and(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "part")
        })
    }));
    Ok(())
}
