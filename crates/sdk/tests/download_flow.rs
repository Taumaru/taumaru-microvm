mod support;

use std::error::Error;

use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{
    ArtifactKind, DownloadCancellation, DownloadDisposition, DownloadPhase, MicroVmSdk, SdkError,
};

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

#[tokio::test]
async fn cancellation_does_not_publish_or_record_a_kernel()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let cancellation = DownloadCancellation::new();
    cancellation.cancel();
    let mut phases = Vec::new();

    let result = sdk
        .download_kernel_with_cancellation("linux-test-x86_64", &cancellation, |event| {
            phases.push(event.phase);
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
    assert_eq!(server.artifact_request_count(), 0);
    Ok(())
}

#[tokio::test]
async fn cancellation_after_a_verified_binary_member_preserves_that_member()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let cancellation = DownloadCancellation::new();
    let callback_cancellation = cancellation.clone();

    let result = sdk
        .download_binary_with_cancellation(
            "firecracker-test-1.0.0-x86_64",
            &cancellation,
            move |event| {
                if event.phase == DownloadPhase::Completed
                    && event.member_name.as_deref() == Some("firecracker")
                {
                    callback_cancellation.cancel();
                }
            },
        )
        .await;

    assert!(matches!(result, Err(SdkError::Cancelled)));
    assert!(
        home.path()
            .join("tools/firecracker-test-1.0.0-x86_64/firecracker/firecracker")
            .exists()
    );
    assert!(
        !home
            .path()
            .join("tools/firecracker-test-1.0.0-x86_64/jailer/jailer")
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn downloads_one_distribution_image_without_touching_its_sibling()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let mut events = Vec::new();

    let result = sdk
        .download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |event| {
            events.push(event);
        })
        .await?;

    assert_eq!(result.distribution.id, "alpine-test-1.0");
    assert_eq!(result.image.id, "alpine-test-minimal");
    assert_eq!(result.file.artifact_kind, ArtifactKind::DistributionImage);
    assert_eq!(
        result.file.member_name.as_deref(),
        Some("alpine-test-minimal")
    );
    assert_eq!(result.file.disposition, DownloadDisposition::Downloaded);
    assert!(result.file.absolute_path.starts_with(home.path()));
    assert!(
        home.path()
            .join("artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4")
            .exists()
    );
    assert!(
        !home
            .path()
            .join("artifacts/rootfs/alpine-test-1.0/alpine-test-debug")
            .exists()
    );
    assert!(
        events
            .iter()
            .all(|event| event.member_name.as_deref() == Some("alpine-test-minimal"))
    );
    assert_eq!(
        events.last().map(|event| &event.phase),
        Some(&DownloadPhase::Completed)
    );
    assert_eq!(server.artifact_request_count(), 1);
    Ok(())
}

#[tokio::test]
async fn reuses_a_downloaded_image_and_keeps_whole_distribution_working()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let first = sdk
        .download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |_| {})
        .await?;
    let requests_after_first = server.artifact_request_count();
    let second = sdk
        .download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |_| {})
        .await?;

    assert_eq!(first.file.disposition, DownloadDisposition::Downloaded);
    assert_eq!(
        second.file.disposition,
        DownloadDisposition::SkippedExisting
    );
    assert_eq!(server.artifact_request_count(), requests_after_first);

    let whole = sdk.download_distribution("alpine-test-1.0", |_| {}).await?;
    assert_eq!(whole.images.len(), 2);
    assert!(
        whole
            .images
            .iter()
            .any(|file| file.member_name.as_deref() == Some("alpine-test-minimal"))
    );
    assert!(
        whole
            .images
            .iter()
            .any(|file| file.member_name.as_deref() == Some("alpine-test-debug"))
    );
    Ok(())
}

#[tokio::test]
async fn rejects_an_image_outside_its_named_distribution()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let missing_image = sdk
        .download_distribution_image("alpine-test-1.0", "no-such-image", |_| {})
        .await
        .expect_err("unknown image should not download");
    assert!(matches!(missing_image, SdkError::NotFound { kind, id }
            if kind == "distribution image" && id == "no-such-image"));

    let missing_distribution = sdk
        .download_distribution_image("no-such-distro", "alpine-test-minimal", |_| {})
        .await
        .expect_err("unknown distribution should not download");
    assert!(
        matches!(missing_distribution, SdkError::NotFound { kind, id }
            if kind == "distribution" && id == "no-such-distro")
    );
    assert_eq!(server.artifact_request_count(), 0);
    Ok(())
}

#[tokio::test]
async fn cancelled_single_image_download_publishes_nothing()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let cancellation = DownloadCancellation::new();
    cancellation.cancel();
    let mut phases = Vec::new();

    let result = sdk
        .download_distribution_image_with_cancellation(
            "alpine-test-1.0",
            "alpine-test-minimal",
            &cancellation,
            |event| {
                phases.push(event.phase);
            },
        )
        .await;

    assert!(matches!(result, Err(SdkError::Cancelled)));
    assert!(phases.contains(&DownloadPhase::Cancelled));
    assert!(
        !home
            .path()
            .join("artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4")
            .exists()
    );
    assert_eq!(server.artifact_request_count(), 0);
    Ok(())
}

#[tokio::test]
async fn image_readiness_tracks_download_repair_and_rejection()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;
    let image_path = home
        .path()
        .join("artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4");

    assert!(
        !sdk.is_distribution_image_ready("alpine-test-1.0", "alpine-test-minimal")
            .await?
    );
    assert!(!image_path.exists());

    sdk.download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |_| {})
        .await?;
    assert!(
        sdk.is_distribution_image_ready("alpine-test-1.0", "alpine-test-minimal")
            .await?
    );
    let requests_after_download = server.artifact_request_count();

    std::fs::write(&image_path, b"stale-bytes")?;
    assert!(
        !sdk.is_distribution_image_ready("alpine-test-1.0", "alpine-test-minimal")
            .await?
    );

    let unknown_distribution = sdk
        .is_distribution_image_ready("no-such-distro", "alpine-test-minimal")
        .await
        .expect_err("unknown distribution should not report readiness");
    assert!(
        matches!(unknown_distribution, SdkError::NotFound { kind, id }
            if kind == "distribution" && id == "no-such-distro")
    );

    let wrong_distribution = sdk
        .is_distribution_image_ready("alpine-test-1.0", "no-such-image")
        .await
        .expect_err("unknown image should not report readiness");
    assert!(matches!(wrong_distribution, SdkError::NotFound { kind, id }
            if kind == "distribution image" && id == "no-such-image"));

    for (distribution, image) in [("", "alpine-test-minimal"), ("alpine-test-1.0", "")] {
        assert!(
            sdk.is_distribution_image_ready(distribution, image)
                .await
                .is_err(),
            "blank IDs should not report readiness"
        );
    }
    assert_eq!(server.artifact_request_count(), requests_after_download);
    Ok(())
}

#[tokio::test]
async fn present_images_list_reports_inventory_without_registry_or_hashing()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    assert!(sdk.list_present_distribution_images().await?.is_empty());

    sdk.download_distribution_image("alpine-test-1.0", "alpine-test-minimal", |_| {})
        .await?;
    let requests_after_download = server.artifact_request_count();
    let present = sdk.list_present_distribution_images().await?;
    assert_eq!(
        present,
        vec![(
            "alpine-test-1.0".to_owned(),
            "alpine-test-minimal".to_owned()
        )]
    );

    let image_path = home
        .path()
        .join("artifacts/rootfs/alpine-test-1.0/alpine-test-minimal/alpine-test-minimal.ext4");
    std::fs::write(&image_path, b"stale-bytes")?;
    assert!(
        !sdk.is_distribution_image_ready("alpine-test-1.0", "alpine-test-minimal")
            .await?
    );
    assert_eq!(
        sdk.list_present_distribution_images().await?,
        vec![(
            "alpine-test-1.0".to_owned(),
            "alpine-test-minimal".to_owned()
        )]
    );
    assert_eq!(server.artifact_request_count(), requests_after_download);
    Ok(())
}
