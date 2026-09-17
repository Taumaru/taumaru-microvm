mod support;

use std::error::Error;

use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::MicroVmSdk;

#[tokio::test]
async fn lists_all_registry_collections_and_nested_members()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), server.base_url())?;

    let kernels = sdk.list_kernels().await?;
    let binaries = sdk.list_binaries().await?;
    let distributions = sdk.list_distributions().await?;

    assert_eq!(kernels.len(), 2);
    assert_eq!(binaries.len(), 2);
    assert_eq!(binaries[0].files.len(), 2);
    assert_eq!(distributions.len(), 1);
    assert_eq!(distributions[0].images.len(), 2);
    assert_eq!(distributions[0].supported_kernels.len(), 2);
    assert!(server.request_count() >= 3);
    Ok(())
}

#[tokio::test]
async fn returns_a_typed_error_when_the_registry_is_unavailable()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let home = tempdir()?;
    let sdk = MicroVmSdk::with_registry_base_url(home.path(), "http://127.0.0.1:1/v1/")?;

    let error = sdk
        .list_kernels()
        .await
        .expect_err("unavailable registry should fail");
    assert!(matches!(
        error,
        taumaru_microvm::SdkError::RegistryTransport(_)
    ));
    Ok(())
}
