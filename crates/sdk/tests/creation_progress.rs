mod support;

use std::error::Error;

use tempfile::tempdir;

use support::FixtureServer;
use taumaru_microvm::{
    CreateMicroVmRequest, CreationEventPhase, CreationOutcome, CreationProgress, CreationStage,
    MicroVmSdk, SdkError, TOTAL_CREATION_STEPS,
};

fn invalid_request() -> CreateMicroVmRequest {
    CreateMicroVmRequest {
        name: String::from("not path friendly"),
        distribution_id: String::from("alpine-test-1.0"),
        image_id: String::from("alpine-test-minimal"),
        disk_size_bytes: 1024,
        vcpu_count: 1,
        memory_bytes: 128 * 1024 * 1024,
        expose_on_lan: false,
        lan_address: None,
        volume_path: None,
    }
}

#[tokio::test]
async fn observer_sees_failed_terminal() -> Result<(), Box<dyn Error + Send + Sync>> {
    let _server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::new(home.path())?;
    let mut events: Vec<CreationProgress> = Vec::new();
    let error = sdk
        .create_microvm(
            invalid_request(),
            Some(|event: CreationProgress| events.push(event)),
        )
        .await
        .expect_err("invalid request should fail");
    assert!(matches!(error, SdkError::InvalidRequest { .. }));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].phase, CreationEventPhase::Started);
    assert_eq!(events[0].overall_percent, 0);
    assert_eq!(events[1].stage, CreationStage::Validation);
    assert_eq!(events[1].completed_steps, 0);
    assert_eq!(events[1].total_steps, TOTAL_CREATION_STEPS);
    assert_eq!(events[1].overall_percent, 0);
    assert_eq!(events[1].bytes_completed, None);
    assert_eq!(events[1].expected_bytes, None);
    assert_eq!(
        events[1].outcome,
        Some(CreationOutcome::Failed {
            stage: CreationStage::Validation,
        })
    );
    Ok(())
}

#[tokio::test]
async fn no_observer_reports_same_error() -> Result<(), Box<dyn Error + Send + Sync>> {
    let _server = FixtureServer::start().await?;
    let home = tempdir()?;
    let sdk = MicroVmSdk::new(home.path())?;
    let error = sdk
        .create_microvm(invalid_request(), None::<fn(CreationProgress)>)
        .await
        .expect_err("invalid request should fail");
    assert!(matches!(error, SdkError::InvalidRequest { .. }));
    Ok(())
}
