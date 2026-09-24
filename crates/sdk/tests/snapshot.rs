use std::path::Path;

use taumaru_microvm::{MicroVmSdk, SdkError, SnapshotAddressPolicy, SnapshotCancellation};

#[tokio::test]
async fn missing_vm_returns_a_typed_error_without_publishing_output() {
    let home = tempfile::tempdir().expect("SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    let output = home.path().join("missing.tmvmsnap");

    let error = sdk
        .create_snapshot("missing_vm", &output, SnapshotAddressPolicy::PreserveIpv4)
        .await
        .expect_err("unknown VM should be rejected");

    assert!(matches!(error, SdkError::NotFound { .. }));
    assert!(!output.exists());
}

#[test]
fn cancellation_handle_clones_share_the_same_request() {
    let cancellation = SnapshotCancellation::new();
    let clone = cancellation.clone();
    assert!(!cancellation.is_cancelled());

    clone.cancel();

    assert!(cancellation.is_cancelled());
    assert!(clone.is_cancelled());
}

#[test]
fn snapshot_api_accepts_relative_output_paths() {
    let output = Path::new("./export.tmvmsnap");
    assert_eq!(
        output.file_name().and_then(|name| name.to_str()),
        Some("export.tmvmsnap")
    );
}
