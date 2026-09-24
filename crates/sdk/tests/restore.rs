use taumaru_microvm::{MicroVmSdk, RestoreRequest, SdkError};

#[tokio::test]
async fn public_restore_api_reports_a_missing_archive_as_a_typed_error() {
    let home = tempfile::tempdir().expect("SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");

    let error = sdk
        .restore_snapshot(RestoreRequest {
            archive_path: home.path().join("missing.tmvmsnap"),
        })
        .await
        .expect_err("missing archive should fail");

    assert!(matches!(error, SdkError::Filesystem { .. }));
    assert!(
        sdk.list_microvms()
            .await
            .expect("inventory should read")
            .is_empty()
    );
}

#[tokio::test]
async fn legacy_age_archive_returns_actionable_error_without_publishing_vm_state() {
    use std::fs;

    let home = tempfile::tempdir().expect("SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    let archive_path = home.path().join("legacy.tmvmsnap");
    fs::write(
        &archive_path,
        b"age-encryption.org/v1\n-> scrypt\nlegacy fixture",
    )
    .expect("legacy archive fixture should be written");

    let error = sdk
        .restore_snapshot(RestoreRequest {
            archive_path: archive_path.clone(),
        })
        .await
        .expect_err("legacy encrypted archives should be rejected");

    assert!(matches!(error, SdkError::RestoreArchive { .. }));
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("new unencrypted snapshot")
    );
    assert!(archive_path.is_file());
    assert!(
        sdk.list_microvms()
            .await
            .expect("inventory should be readable")
            .is_empty()
    );
}
