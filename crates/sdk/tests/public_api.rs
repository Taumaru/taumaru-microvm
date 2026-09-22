#[test]
fn public_artifact_types_are_exported() {
    use taumaru_microvm::{
        Architecture, ArtifactFile, ArtifactKind, CreateMicroVmRequest, CreationEventPhase,
        CreationOutcome, CreationProgress, CreationStage, DownloadDisposition, DownloadPhase,
        DownloadProgress, DownloadedBinary, DownloadedDistribution, DownloadedDistributionImage,
        DownloadedFile, DownloadedKernel, InstalledBinary, Kernel, MicroVmCreationResult,
        MicroVmSdk, MicroVmState, NetworkConfiguration, NetworkConfigurationResult, NetworkMode,
        NetworkResource, SdkError, SshConnectionInfo, TOTAL_CREATION_STEPS,
    };
    let _ = std::mem::size_of::<Architecture>();
    let _ = std::mem::size_of::<ArtifactFile>();
    let _ = std::mem::size_of::<ArtifactKind>();
    let _ = std::mem::size_of::<CreationEventPhase>();
    let _ = std::mem::size_of::<CreationOutcome>();
    let _ = std::mem::size_of::<CreationProgress>();
    let _ = std::mem::size_of::<CreationStage>();
    assert_eq!(TOTAL_CREATION_STEPS, 6);
    let _ = std::mem::size_of::<CreateMicroVmRequest>();
    let _ = std::mem::size_of::<DownloadDisposition>();
    let _ = std::mem::size_of::<DownloadPhase>();
    let _ = std::mem::size_of::<DownloadProgress>();
    let _ = std::mem::size_of::<DownloadedBinary>();
    let _ = std::mem::size_of::<DownloadedDistribution>();
    let _ = std::mem::size_of::<DownloadedDistributionImage>();
    let _ = std::mem::size_of::<DownloadedFile>();
    let _ = std::mem::size_of::<DownloadedKernel>();
    let _ = std::mem::size_of::<InstalledBinary>();
    let _ = std::mem::size_of::<Kernel>();
    let _ = std::mem::size_of::<MicroVmCreationResult>();
    let _ = std::mem::size_of::<MicroVmSdk>();
    let _ = std::mem::size_of::<MicroVmState>();
    let _ = std::mem::size_of::<NetworkConfiguration>();
    let _ = std::mem::size_of::<NetworkConfigurationResult>();
    let _ = std::mem::size_of::<NetworkMode>();
    let _ = std::mem::size_of::<NetworkResource>();
    let _ = std::mem::size_of::<SdkError>();
    let _ = std::mem::size_of::<SshConnectionInfo>();
}
#[test]
fn public_resolver_method_is_available_without_process_local_state() {
    use taumaru_microvm::MicroVmSdk;

    fn use_resolver(sdk: &MicroVmSdk) {
        let future = sdk.resolve_binary("package", "component");
        drop(future);
    }

    let _ = use_resolver;
}

#[tokio::test]
async fn network_reconciliation_reports_unknown_vms_as_typed_errors() {
    use taumaru_microvm::{MicroVmSdk, SdkError};
    use tempfile::tempdir;

    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let result = sdk.configure_network("missing_vm").await;

    assert!(
        matches!(result, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "missing_vm")
    );
}

#[tokio::test]
async fn creation_validates_input_before_registry_or_host_mutation() {
    use std::fs;
    use taumaru_microvm::{CreateMicroVmRequest, CreationProgress, MicroVmSdk, SdkError};
    use tempfile::tempdir;

    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");
    let result = sdk
        .create_microvm(
            CreateMicroVmRequest {
                name: "not path friendly".to_owned(),
                distribution_id: "alpine-test-1.0".to_owned(),
                image_id: "alpine-test-minimal".to_owned(),
                disk_size_bytes: 1024,
                vcpu_count: 1,
                memory_bytes: 128 * 1024 * 1024,
                expose_on_lan: false,
                lan_address: None,
                volume_path: None,
            },
            None::<fn(CreationProgress)>,
        )
        .await;
    assert!(matches!(result, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));
    assert!(
        fs::read_dir(home.path().join("vms"))
            .expect("managed VM directory should exist")
            .next()
            .is_none()
    );
}

#[test]
fn start_result_types_are_exported_with_running_state() {
    use taumaru_microvm::{MicroVmStartResult, MicroVmState};
    let _ = std::mem::size_of::<MicroVmStartResult>();
    let _ = MicroVmState::Running;
    let _ = MicroVmState::Stopped;
}
#[test]
fn stop_result_types_are_exported_with_stopped_state() {
    use taumaru_microvm::{MicroVmState, MicroVmStopResult};
    let _ = std::mem::size_of::<MicroVmStopResult>();
    let _ = MicroVmState::Running;
    let _ = MicroVmState::Stopped;
}

#[tokio::test]
async fn stop_validates_names_and_reports_unknown_vms_as_typed_errors() {
    use taumaru_microvm::{MicroVmSdk, SdkError};
    use tempfile::tempdir;

    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let invalid = sdk.stop_microvm("not path friendly").await;
    assert!(matches!(invalid, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));

    let missing = sdk.stop_microvm("missing_vm").await;
    assert!(
        matches!(missing, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "missing_vm")
    );
}

#[tokio::test]
async fn start_validates_names_and_reports_unknown_vms_as_typed_errors() {
    use taumaru_microvm::{MicroVmSdk, SdkError};
    use tempfile::tempdir;

    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let invalid = sdk.start_microvm("not path friendly").await;
    assert!(matches!(invalid, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));

    let missing = sdk.start_microvm("missing_vm").await;
    assert!(
        matches!(missing, Err(SdkError::NotFound { kind, id }) if kind == "MicroVM" && id == "missing_vm")
    );
}

#[test]
fn prune_types_are_exported_with_ordered_summary_shape() {
    use taumaru_microvm::{PruneFailure, PruneSummary, PrunedImageId, SdkError};
    let _ = std::mem::size_of::<PruneSummary>();
    let _ = std::mem::size_of::<PrunedImageId>();
    let _ = std::mem::size_of::<PruneFailure>();
    let summary = PruneSummary {
        removed_kernels: vec!["kernel-b".to_owned(), "kernel-a".to_owned()],
        removed_images: vec![
            PrunedImageId {
                distribution_id: "distro".to_owned(),
                image_id: "image-b".to_owned(),
            },
            PrunedImageId {
                distribution_id: "distro".to_owned(),
                image_id: "image-a".to_owned(),
            },
        ],
        skipped_artifact_keys: vec!["kernel:skipped".to_owned()],
        freed_bytes_kernels: 10,
        freed_bytes_images: 20,
        freed_bytes_total: 30,
    };
    assert_eq!(summary.removed_kernels.len(), 2);
    assert_eq!(summary.removed_images.len(), 1 + 1);
    assert_eq!(summary.freed_bytes_total, 30);
    let error = SdkError::PruneIncomplete {
        summary,
        failures: vec![PruneFailure {
            artifact_key: "kernel:failed".to_owned(),
            reason: "the recorded path is not a regular file".to_owned(),
        }],
    };
    assert!(matches!(error, SdkError::PruneIncomplete { .. }));
    assert!(error.to_string().contains("kernel:failed"));
}
