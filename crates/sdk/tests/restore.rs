use taumaru_microvm::{MicroVmSdk, RestoreRequest, SdkError};

#[tokio::test]
async fn public_restore_api_rejects_an_empty_password_before_reading_the_archive() {
    let home = tempfile::tempdir().expect("SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");

    let error = sdk
        .restore_snapshot(RestoreRequest {
            archive_path: home.path().join("missing.tmvmsnap"),
            password: String::new(),
        })
        .await
        .expect_err("empty restore password should fail");

    assert!(matches!(
        error,
        SdkError::InvalidRequest { ref field, .. } if field == "password"
    ));
}

#[tokio::test]
async fn public_restore_api_reports_a_missing_archive_as_a_typed_error() {
    let home = tempfile::tempdir().expect("SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");

    let error = sdk
        .restore_snapshot(RestoreRequest {
            archive_path: home.path().join("missing.tmvmsnap"),
            password: "restore-password".to_owned(),
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
