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
