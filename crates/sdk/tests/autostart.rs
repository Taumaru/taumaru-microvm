use rusqlite::{Connection, params};
use tempfile::tempdir;

use taumaru_microvm::{
    AutostartPolicy, AutostartPolicyUpdate, AutostartSettings, MicroVmSdk, SdkError,
};

fn insert_vm(home: &std::path::Path, name: &str) {
    let connection =
        Connection::open(home.join("state").join("inventory.db")).expect("inventory should open");
    let volume = home.join("vms").join(name);
    connection
        .execute(
            "INSERT INTO microvms (
                name, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
            ) VALUES (?1, 'distro', 'image', 'kernel', 'fc', 'firectl',
                      1, 1, 1, 1, ?2, ?3, ?4, 0, 1, 1)",
            params![
                name,
                volume.to_string_lossy().into_owned(),
                volume.join("rootfs.ext4").to_string_lossy().into_owned(),
                volume
                    .join("firecracker.sock")
                    .to_string_lossy()
                    .into_owned(),
            ],
        )
        .expect("seed VM row should insert");
}

fn policy(name: &str, enabled: bool, max_start_attempts: u32) -> AutostartPolicy {
    AutostartPolicy {
        name: name.to_owned(),
        enabled,
        max_start_attempts,
    }
}

#[tokio::test]
async fn policies_can_be_created_updated_listed_and_removed() {
    let home = tempdir().expect("home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    insert_vm(home.path(), "web-01");
    insert_vm(home.path(), "db-01");

    let created = sdk
        .create_autostart_policy("web-01", AutostartSettings::default())
        .await
        .expect("policy should be created");
    assert_eq!(created, policy("web-01", true, 3));
    sdk.create_autostart_policy(
        "db-01",
        AutostartSettings {
            enabled: true,
            max_start_attempts: 5,
        },
    )
    .await
    .expect("second policy should be created");

    let updated = sdk
        .update_autostart_policy(
            "web-01",
            AutostartPolicyUpdate {
                enabled: Some(false),
                max_start_attempts: None,
            },
        )
        .await
        .expect("policy should be updated");
    assert_eq!(updated, policy("web-01", false, 3));

    let listed = sdk
        .list_autostart_policies()
        .await
        .expect("policies should list");
    assert_eq!(
        listed,
        vec![policy("db-01", true, 5), policy("web-01", false, 3)]
    );

    let removed = sdk
        .delete_autostart_policy("web-01")
        .await
        .expect("policy should be removed");
    assert!(removed.removed);
    let again = sdk
        .delete_autostart_policy("web-01")
        .await
        .expect("repeated removal should be idempotent");
    assert!(!again.removed);
    assert_eq!(
        sdk.autostart_policy("web-01")
            .await
            .expect("lookup should succeed"),
        None
    );
}

#[tokio::test]
async fn creating_an_identical_policy_is_idempotent_and_a_different_one_conflicts() {
    let home = tempdir().expect("home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    insert_vm(home.path(), "web-01");
    let settings = AutostartSettings::default();

    sdk.create_autostart_policy("web-01", settings)
        .await
        .expect("policy should be created");
    let repeated = sdk
        .create_autostart_policy("web-01", settings)
        .await
        .expect("identical policy should be accepted");
    assert_eq!(repeated, policy("web-01", true, 3));

    let conflict = sdk
        .create_autostart_policy(
            "web-01",
            AutostartSettings {
                enabled: true,
                max_start_attempts: 7,
            },
        )
        .await;
    assert!(matches!(
        conflict,
        Err(SdkError::ConfigurationConflict { field, .. }) if field == "autostart.max_start_attempts"
    ));
}

#[tokio::test]
async fn invalid_and_missing_targets_return_typed_errors() {
    let home = tempdir().expect("home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    insert_vm(home.path(), "web-01");

    let missing_vm = sdk
        .create_autostart_policy("ghost", AutostartSettings::default())
        .await;
    assert!(matches!(missing_vm, Err(SdkError::NotFound { kind, .. }) if kind == "MicroVM"));

    let invalid_name = sdk
        .create_autostart_policy("not path friendly", AutostartSettings::default())
        .await;
    assert!(matches!(invalid_name, Err(SdkError::InvalidRequest { field, .. }) if field == "name"));

    let invalid_attempts = sdk
        .create_autostart_policy(
            "web-01",
            AutostartSettings {
                enabled: true,
                max_start_attempts: 0,
            },
        )
        .await;
    assert!(matches!(
        invalid_attempts,
        Err(SdkError::InvalidRequest { field, .. }) if field == "max_start_attempts"
    ));

    let missing_policy = sdk
        .update_autostart_policy(
            "web-01",
            AutostartPolicyUpdate {
                enabled: Some(true),
                max_start_attempts: None,
            },
        )
        .await;
    assert!(
        matches!(missing_policy, Err(SdkError::NotFound { kind, .. }) if kind == "autostart policy")
    );
}

#[tokio::test]
async fn removing_a_microvm_row_removes_its_policy() {
    let home = tempdir().expect("home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    insert_vm(home.path(), "web-01");
    sdk.create_autostart_policy("web-01", AutostartSettings::default())
        .await
        .expect("policy should be created");

    let connection = Connection::open(home.path().join("state").join("inventory.db"))
        .expect("inventory should open");
    connection
        .execute_batch("PRAGMA foreign_keys = ON; DELETE FROM microvms WHERE name = 'web-01';")
        .expect("VM row should be deleted");

    assert!(
        sdk.list_autostart_policies()
            .await
            .expect("policies should list")
            .is_empty()
    );
}

#[tokio::test]
async fn autostart_run_without_policies_reports_nothing() {
    let home = tempdir().expect("home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK should initialize");
    let report = sdk
        .start_autostart_microvms()
        .await
        .expect("empty run should complete");
    assert!(report.outcomes.is_empty());
    assert!(!report.has_failures());
}
