mod support;

use std::error::Error;

use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{ArtifactKind, DownloadDisposition, DownloadPhase, MicroVmSdk};

#[tokio::test]
async fn downloads_a_kernel_with_integrity_and_live_progress()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let mut events = Vec::new();

    let result = sdk
        .download_kernel("linux-test-x86_64", |event| events.push(event))
        .await?;

    assert_eq!(result.file.artifact_kind, ArtifactKind::Kernel);
    assert_eq!(result.file.disposition, DownloadDisposition::Downloaded);
    assert_eq!(result.file.size_bytes, 15);
    assert_eq!(
        result.file.sha256,
        "19034b8326bddd75c6fd63c489ada9c7f6a8807312e1dd9318bc23c4898ad498"
    );
    assert_eq!(
        std::fs::read(&result.file.absolute_path)?,
        include_bytes!("fixtures/kernel-fixture")
    );
    assert_eq!(
        events.first().map(|event| &event.phase),
        Some(&DownloadPhase::Downloading)
    );
    assert!(
        events
            .iter()
            .any(|event| event.phase == DownloadPhase::Verifying)
    );
    assert_eq!(
        events.last().map(|event| &event.phase),
        Some(&DownloadPhase::Completed)
    );
    assert!(events.windows(2).all(|events| {
        events[1].aggregate_bytes_received >= events[0].aggregate_bytes_received
    }));
    assert_eq!(server.artifact_request_count(), 1);
    Ok(())
}

#[tokio::test]
async fn adopts_a_correct_existing_file_without_transfer()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let target = home
        .path()
        .join("artifacts/kernels/linux-test-x86_64/vmlinux");
    std::fs::create_dir_all(target.parent().ok_or("target parent is missing")?)?;
    std::fs::write(&target, include_bytes!("fixtures/kernel-fixture"))?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let mut events = Vec::new();

    let result = sdk
        .download_kernel("linux-test-x86_64", |event| events.push(event))
        .await?;

    assert_eq!(
        result.file.disposition,
        DownloadDisposition::AdoptedExisting
    );
    assert_eq!(server.artifact_request_count(), 0);
    assert_eq!(
        events.last().map(|event| &event.phase),
        Some(&DownloadPhase::AdoptedExisting)
    );
    Ok(())
}

#[tokio::test]
async fn skips_a_correct_file_with_a_complete_inventory_relationship()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let first = sdk.download_kernel("linux-test-x86_64", |_| {}).await?;
    let requests_after_first = server.artifact_request_count();
    let second = sdk.download_kernel("linux-test-x86_64", |_| {}).await?;

    assert_eq!(first.file.disposition, DownloadDisposition::Downloaded);
    assert_eq!(
        second.file.disposition,
        DownloadDisposition::SkippedExisting
    );
    assert_eq!(server.artifact_request_count(), requests_after_first);
    Ok(())
}

#[tokio::test]
async fn replaces_a_wrong_existing_file_before_publishing_the_verified_file()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let target = home
        .path()
        .join("artifacts/kernels/linux-test-x86_64/vmlinux");
    std::fs::create_dir_all(target.parent().ok_or("target parent is missing")?)?;
    std::fs::write(&target, b"invalid")?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let result = sdk.download_kernel("linux-test-x86_64", |_| {}).await?;

    assert_eq!(result.file.disposition, DownloadDisposition::Downloaded);
    assert_eq!(
        std::fs::read(&target)?,
        include_bytes!("fixtures/kernel-fixture")
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

#[tokio::test]
async fn downloads_all_binary_components_and_applies_executable_mode()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let mut events = Vec::new();

    let result = sdk
        .download_binary("firecracker-test-1.0.0-x86_64", |event| events.push(event))
        .await?;

    assert_eq!(result.files.len(), 2);
    assert!(
        result
            .files
            .iter()
            .all(|file| file.disposition == DownloadDisposition::Downloaded)
    );
    assert!(
        events
            .iter()
            .any(|event| event.member_name.as_deref() == Some("firecracker"))
    );
    assert!(
        events
            .iter()
            .any(|event| event.member_name.as_deref() == Some("jailer"))
    );
    assert!(
        events
            .iter()
            .all(|event| { event.aggregate_bytes_received <= event.aggregate_total_bytes })
    );
    assert_eq!(server.artifact_request_count(), 2);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for file in result.files {
            assert_eq!(
                std::fs::metadata(file.absolute_path)?.permissions().mode() & 0o777,
                0o755
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn downloads_all_distribution_images_and_persists_kernel_compatibility()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let result = sdk.download_distribution("alpine-test-1.0", |_| {}).await?;

    assert_eq!(result.images.len(), 2);
    assert!(
        result
            .images
            .iter()
            .all(|file| file.disposition == DownloadDisposition::Downloaded)
    );
    assert!(
        result
            .images
            .iter()
            .all(|file| file.absolute_path.starts_with(home.path()))
    );
    assert_eq!(server.artifact_request_count(), 2);
    Ok(())
}
