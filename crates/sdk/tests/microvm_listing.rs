use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

use rusqlite::{Connection, params};
use tempfile::tempdir;

use taumaru_microvm::{MicroVmSdk, MicroVmState};

fn open_inventory(home: &std::path::Path) -> rusqlite::Result<Connection> {
    Connection::open(home.join("state").join("inventory.db"))
}

fn insert_vm(connection: &Connection, name: &str, socket_path: &std::path::Path) {
    let volume = socket_path
        .parent()
        .expect("socket should have a volume parent");
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
                socket_path.to_string_lossy().into_owned(),
            ],
        )
        .expect("seed VM row should insert");
    let vm_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO vm_networks (
                microvm_id, mode, guest_ip, prefix_length, tap_name, guest_mac,
                desired_boot_parameters, updated_at
            ) VALUES (?1, 'host_only', '10.200.8.2', 30, ?2, ?3, 'ip=none', 1)",
            params![
                vm_id,
                format!("tap-{name}"),
                format!("02:00:00:{:02x}:00:{:02x}", name.len(), {
                    let mut digest = 0u8;
                    for byte in name.bytes() {
                        digest ^= byte;
                    }
                    digest
                })
            ],
        )
        .expect("seed network row should insert");
    connection
        .execute(
            "INSERT INTO vm_credentials (
                microvm_id, private_key_path, public_key_path, guest_authorized_keys_path,
                key_type, ssh_user, ssh_port, public_key_fingerprint, file_mode
            ) VALUES (?1, ?2, ?3, '/root/.ssh/authorized_keys', 'ed25519', 'root', 22, 'SHA256:test', '0600')",
            params![
                vm_id,
                format!("/tmp/{name}/id_ed25519"),
                format!("/tmp/{name}/id_ed25519.pub"),
            ],
        )
        .expect("seed credential row should insert");
    connection
        .execute(
            "INSERT INTO vm_runtime (
                microvm_id, firecracker_path, firectl_path, socket_path,
                process_id, process_state, updated_at
            ) VALUES (?1, '/tmp/firecracker', '/tmp/firectl', ?2, NULL, 'stopped', 1)",
            params![vm_id, socket_path.to_string_lossy().into_owned()],
        )
        .expect("seed runtime row should insert");
}

fn seed_vm(home: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let volume = home.path().join("vms").join(name);
    std::fs::create_dir_all(&volume).expect("seed volume should be created");
    let socket = volume.join("firecracker.sock");
    let connection = open_inventory(home.path()).expect("inventory should open");
    insert_vm(&connection, name, &socket);
    socket
}

fn serve_machine_config(listener: UnixListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        listener
            .set_nonblocking(false)
            .expect("listener should block");
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let mut buffer = [0_u8; 512];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        }
    })
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
async fn seeded_vms_report_verified_states_ordered_by_name() {
    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let live_socket = seed_vm(&home, "web-01");
    let live_listener = UnixListener::bind(&live_socket).expect("live socket should bind");
    let _server = serve_machine_config(live_listener);

    let stale_socket = seed_vm(&home, "web-02");
    std::fs::write(&stale_socket, b"not a socket").expect("stale file should be written");

    seed_vm(&home, "db-01");

    let listed = sdk
        .list_microvms()
        .await
        .expect("listing seeded VMs should work");

    let names: Vec<&str> = listed.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["db-01", "web-01", "web-02"]);
    assert_eq!(listed[0].state, MicroVmState::Stopped);
    assert_eq!(listed[1].state, MicroVmState::Running);
    assert_eq!(listed[2].state, MicroVmState::Stopped);
    for item in &listed {
        assert_eq!(item.vcpu_count, 1);
        assert_eq!(item.memory_bytes, 1);
        assert_eq!(item.disk_size_bytes, 1);
        assert_eq!(item.distribution_id, "distro");
        assert_eq!(item.image_id, "image");
        assert_eq!(
            item.network_mode,
            Some(taumaru_microvm::NetworkMode::HostOnly)
        );
        assert_eq!(
            item.guest_address,
            Some("10.200.8.2".parse().expect("seed guest IP should parse"))
        );
        assert_eq!(item.lan_address, None);
    }

    let incomplete_home = tempdir().expect("temporary SDK home should be created");
    let incomplete_sdk =
        MicroVmSdk::new(incomplete_home.path()).expect("SDK construction should work");
    let incomplete_socket = seed_vm(&incomplete_home, "half-01");
    let connection = open_inventory(incomplete_home.path()).expect("inventory should open");
    connection
        .execute(
            "DELETE FROM vm_networks WHERE microvm_id = (SELECT id FROM microvms WHERE name = 'half-01')",
            [],
        )
        .expect("seed network row should delete");
    drop(connection);
    let _ = &incomplete_socket;
    let incomplete = incomplete_sdk
        .list_microvms()
        .await
        .expect("listing an incomplete VM should work");
    assert_eq!(incomplete.len(), 1);
    assert_eq!(incomplete[0].name, "half-01");
    assert_eq!(incomplete[0].vcpu_count, 1);
    assert_eq!(incomplete[0].distribution_id, "distro");
    assert_eq!(incomplete[0].network_mode, None);
    assert_eq!(incomplete[0].guest_address, None);
    assert_eq!(incomplete[0].lan_address, None);

    let running = sdk
        .list_running_microvms()
        .await
        .expect("running listing should work");
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].name, "web-01");
}

#[tokio::test]
async fn recycled_pid_with_silent_socket_reports_stopped() {
    let home = tempdir().expect("temporary SDK home should be created");
    let sdk = MicroVmSdk::new(home.path()).expect("SDK construction should work");

    let socket = seed_vm(&home, "recycled");
    let connection = open_inventory(home.path()).expect("inventory should open");
    connection
        .execute(
            "UPDATE vm_runtime SET process_id = ?1 WHERE socket_path = ?2",
            params![std::process::id(), socket.to_string_lossy().into_owned()],
        )
        .expect("runtime PID should update");
    drop(connection);

    let _probe = UnixStream::connect(&socket).expect_err("silent path should refuse");
    let listed = sdk.list_microvms().await.expect("listing should work");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, MicroVmState::Stopped);
}

#[test]
fn listing_summary_type_is_exported() {
    let _ = std::mem::size_of::<taumaru_microvm::MicroVmSummary>();
}
