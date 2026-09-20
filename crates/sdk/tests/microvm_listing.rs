use rusqlite::{Connection, params};
use tempfile::tempdir;

use taumaru_microvm::{MicroVmSdk, MicroVmState};

fn open_inventory(home: &std::path::Path) -> rusqlite::Result<Connection> {
    Connection::open(home.join("state").join("inventory.db"))
}

fn insert_vm(connection: &Connection, name: &str, state: &str) {
    let volume = format!("/tmp/{name}");
    connection
        .execute(
            "INSERT INTO microvms (
                name, state, distribution_id, image_id, kernel_id,
                firecracker_package_id, firectl_package_id, disk_size_bytes,
                memory_requested_bytes, memory_effective_mib, vcpu_count,
                volume_path, rootfs_path, socket_path, expose_on_lan, created_at, updated_at
            ) VALUES (?1, ?2, 'distro', 'image', 'kernel', 'fc', 'firectl',
                      1, 1, 1, 1, ?3, ?4, ?5, 0, 1, 1)",
            params![
                name,
                state,
                volume.clone(),
                format!("{volume}/rootfs.ext4"),
                format!("{volume}/firecracker.sock"),
            ],
        )
        .expect("seed VM row should insert");
}

#[tokio::test]
async fn empty_inventory_lists_no_microvms() {
    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let listed = sdk
        .list_microvms()
        .await
        .expect("listing an empty inventory should work");

    assert!(listed.is_empty());
}

#[tokio::test]
async fn seeded_vms_list_ordered_by_name_with_persisted_states() {
    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");
    let connection = open_inventory(home.path()).expect("inventory should open");
    insert_vm(&connection, "web-02", "configured");
    insert_vm(&connection, "web-01", "running");
    insert_vm(&connection, "db-01", "creating");
    drop(connection);

    let listed = sdk
        .list_microvms()
        .await
        .expect("listing seeded VMs should work");

    let names: Vec<&str> = listed.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["db-01", "web-01", "web-02"]);
    assert_eq!(listed[0].state, MicroVmState::Creating);
    assert_eq!(listed[1].state, MicroVmState::Running);
    assert_eq!(listed[2].state, MicroVmState::Configured);
}

#[test]
fn listing_summary_type_is_exported() {
    let _ = std::mem::size_of::<taumaru_microvm::MicroVmSummary>();
}
