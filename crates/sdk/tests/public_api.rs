use std::panic::catch_unwind;

use taumaru_microvm::example_message;

#[test]
fn example_message_is_deterministic_and_panic_free() {
    let first = catch_unwind(example_message).ok();
    let second = catch_unwind(example_message).ok();

    assert_eq!(first, Some("taumaru-microvm SDK is ready"));
    assert_eq!(second, Some("taumaru-microvm SDK is ready"));
    assert_eq!(first, second);
}

#[test]
fn public_artifact_types_are_exported() {
    use taumaru_microvm::{
        Architecture, ArtifactKind, DownloadDisposition, DownloadPhase, DownloadProgress,
        DownloadedBinary, DownloadedDistribution, DownloadedFile, DownloadedKernel,
        InstalledBinary, Kernel, MicroVmSdk, SdkError,
    };

    let _ = std::mem::size_of::<Architecture>();
    let _ = std::mem::size_of::<ArtifactKind>();
    let _ = std::mem::size_of::<DownloadDisposition>();
    let _ = std::mem::size_of::<DownloadPhase>();
    let _ = std::mem::size_of::<DownloadProgress>();
    let _ = std::mem::size_of::<DownloadedBinary>();
    let _ = std::mem::size_of::<DownloadedDistribution>();
    let _ = std::mem::size_of::<DownloadedFile>();
    let _ = std::mem::size_of::<DownloadedKernel>();
    let _ = std::mem::size_of::<InstalledBinary>();
    let _ = std::mem::size_of::<Kernel>();
    let _ = std::mem::size_of::<MicroVmSdk>();
    let _ = std::mem::size_of::<SdkError>();
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
