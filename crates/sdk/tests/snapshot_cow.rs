#![cfg(target_os = "linux")]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::TempDir;

const SECTOR_BYTES: u64 = 512;
const SNAPSHOT_CHUNK_SECTORS: u64 = 64;
const SNAPSHOT_CHUNK_BYTES: u64 = SECTOR_BYTES * SNAPSHOT_CHUNK_SECTORS;
const ORIGIN_BYTES: u64 = 128 * 1024 * 1024;
const OVERFLOW_COW_BYTES: u64 = 64 * 1024;

#[test]
#[ignore = "requires root, loop devices, and a disposable Linux Device Mapper host; opt in with TAUMARU_RUN_PRIVILEGED_SNAPSHOT_TEST=1"]
fn nested_classic_snapshot_is_private_detects_overflow_and_cleans_up_in_order() {
    assert_eq!(
        std::env::var("TAUMARU_RUN_PRIVILEGED_SNAPSHOT_TEST").as_deref(),
        Ok("1"),
        "set TAUMARU_RUN_PRIVILEGED_SNAPSHOT_TEST=1 to run this privileged integration test"
    );
    assert_eq!(
        command_text("id", &["-u"]),
        "0",
        "the privileged Device Mapper integration test requires root"
    );

    let directory = tempfile::tempdir().expect("temporary test directory");
    let mut resources = Resources::new(directory);
    let identity = unique_identity();
    let origin_file = resources.directory.path().join("origin.img");
    create_sparse_file(&origin_file, ORIGIN_BYTES);

    let capture_cow_file =
        create_cow_file(resources.directory.path(), "capture.cow", 8 * 1024 * 1024);
    let child_cow_file = create_cow_file(resources.directory.path(), "child.cow", 8 * 1024 * 1024);
    let overflow_cow_file = create_cow_file(
        resources.directory.path(),
        "overflow.cow",
        OVERFLOW_COW_BYTES,
    );

    let origin_loop = resources.attach_loop(&origin_file);
    let capture_cow_loop = resources.attach_loop(&capture_cow_file);
    let child_cow_loop = resources.attach_loop(&child_cow_file);
    let overflow_cow_loop = resources.attach_loop(&overflow_cow_file);

    let sector_count = ORIGIN_BYTES / SECTOR_BYTES;
    let capture_name = format!("{identity}-capture");
    resources.create_snapshot(
        &capture_name,
        &format!(
            "0 {sector_count} snapshot {origin_loop} {capture_cow_loop} PO {SNAPSHOT_CHUNK_SECTORS}"
        ),
        true,
    );

    let capture_path = PathBuf::from(format!("/dev/mapper/{capture_name}"));
    write_at(Path::new(&origin_loop), 0, &[0x5a; 4096]).expect("write through the origin");
    assert_eq!(
        read_exact_at(&capture_path, 0, 4096),
        vec![0; 4096],
        "the read-only capture must retain the origin bytes from snapshot time"
    );
    assert_eq!(
        read_exact_at(Path::new(&origin_loop), 0, 4096),
        vec![0x5a; 4096],
        "the origin write must still be visible through the origin"
    );

    let child_name = format!("{identity}-child");
    resources.create_snapshot(
        &child_name,
        &format!(
            "0 {sector_count} snapshot /dev/mapper/{capture_name} {child_cow_loop} PO {SNAPSHOT_CHUNK_SECTORS}"
        ),
        false,
    );

    let child_path = PathBuf::from(format!("/dev/mapper/{child_name}"));
    let child_offset = 1024 * 1024;
    write_at(&child_path, child_offset, &[0xa7; 4096]).expect("write through the private child");
    assert_eq!(
        read_exact_at(&child_path, child_offset, 4096),
        vec![0xa7; 4096],
        "the writable child must return its private changes"
    );
    assert_eq!(
        read_exact_at(&capture_path, child_offset, 4096),
        vec![0; 4096],
        "private child writes must not change the read-only capture"
    );
    assert_eq!(
        read_exact_at(Path::new(&origin_loop), child_offset, 4096),
        vec![0; 4096],
        "private child writes must not change the origin"
    );

    let overflow_name = format!("{identity}-overflow");
    resources.create_snapshot(
        &overflow_name,
        &format!(
            "0 {sector_count} snapshot /dev/mapper/{capture_name} {overflow_cow_loop} PO {SNAPSHOT_CHUNK_SECTORS}"
        ),
        false,
    );
    let overflow_path = PathBuf::from(format!("/dev/mapper/{overflow_name}"));
    let mut detected_overflow = false;
    for chunk in 0..3_000_u64 {
        let offset = 4 * 1024 * 1024 + chunk * SNAPSHOT_CHUNK_BYTES;
        let write = write_at(&overflow_path, offset, &[chunk as u8; 512]);
        let status = command_output("dmsetup", &["status", &overflow_name]);
        if !status.status.success() {
            panic!(
                "dmsetup status failed after child write: {}",
                output_text(&status)
            );
        }
        if output_text(&status)
            .to_ascii_lowercase()
            .contains("overflow")
        {
            detected_overflow = true;
            break;
        }
        if let Err(error) = write {
            panic!("snapshot write failed before dmsetup reported overflow: {error}");
        }
    }
    assert!(
        detected_overflow,
        "the small child COW store must report overflow"
    );

    let cleanup_order = resources
        .cleanup()
        .expect("dependency-ordered resource cleanup");
    assert_eq!(
        cleanup_order,
        vec![
            format!("mapper:{overflow_name}"),
            format!("mapper:{child_name}"),
            format!("mapper:{capture_name}"),
            format!("loop:{overflow_cow_loop}"),
            format!("loop:{child_cow_loop}"),
            format!("loop:{capture_cow_loop}"),
            format!("loop:{origin_loop}"),
        ],
        "each mapping must be removed before its backing loop device"
    );
}

struct Resources {
    directory: TempDir,
    mappers: Vec<String>,
    loops: Vec<String>,
}

impl Resources {
    fn new(directory: TempDir) -> Self {
        Self {
            directory,
            mappers: Vec::new(),
            loops: Vec::new(),
        }
    }

    fn attach_loop(&mut self, backing_file: &Path) -> String {
        let output = run_checked(
            "losetup",
            &[
                "--find",
                "--show",
                "--nooverlap",
                backing_file.to_str().expect("UTF-8 temporary path"),
            ],
        );
        let device = output_text(&output).trim().to_owned();
        assert!(
            device.starts_with("/dev/loop"),
            "unexpected loop device: {device}"
        );
        self.loops.push(device.clone());
        device
    }

    fn create_snapshot(&mut self, name: &str, table: &str, read_only: bool) {
        let mut arguments = vec!["create", name, "--table", table];
        if read_only {
            arguments.push("--readonly");
        }
        arguments.extend(["--noudevrules", "--noudevsync", "--addnodeoncreate"]);
        let output = command_output("dmsetup", &arguments);
        if !output.status.success() && table.contains(" PO ") && supports_p_fallback(&output) {
            let fallback_table = table.replace(" PO ", " P ");
            arguments[3] = &fallback_table;
            run_checked("dmsetup", &arguments);
        } else {
            assert!(
                output.status.success(),
                "dmsetup create {name} failed: {}",
                output_text(&output)
            );
        }
        self.mappers.push(name.to_owned());
    }

    fn cleanup(&mut self) -> Result<Vec<String>, String> {
        let mut order = Vec::new();
        for name in self.mappers.iter().rev().cloned().collect::<Vec<_>>() {
            let output = command_output("dmsetup", &["remove", &name]);
            if !output.status.success() {
                return Err(format!(
                    "could not remove mapper {name}: {}",
                    output_text(&output)
                ));
            }
            order.push(format!("mapper:{name}"));
            self.mappers.pop();
        }
        for device in self.loops.iter().rev().cloned().collect::<Vec<_>>() {
            let output = command_output("losetup", &["--detach", &device]);
            if !output.status.success() {
                return Err(format!(
                    "could not detach loop {device}: {}",
                    output_text(&output)
                ));
            }
            order.push(format!("loop:{device}"));
            self.loops.pop();
        }
        Ok(order)
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn supports_p_fallback(output: &Output) -> bool {
    let message = output_text(output).to_ascii_lowercase();
    message.contains("invalid argument")
        || message.contains("unknown feature")
        || message.contains("unrecognized")
        || message.contains("mode po")
}

fn unique_identity() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    format!("tm-cow-{}-{timestamp:x}", std::process::id())
}

fn create_sparse_file(path: &Path, length: u64) {
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .expect("create sparse origin image");
    file.set_len(length).expect("size sparse origin image");
}

fn create_cow_file(directory: &Path, name: &str, length: u64) -> PathBuf {
    let path = directory.join(name);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .expect("create COW backing file");
    file.set_len(length).expect("size COW backing file");
    path
}

fn write_at(path: &Path, offset: u64, bytes: &[u8]) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.write_all_at(bytes, offset)?;
    file.sync_all()
}

fn read_exact_at(path: &Path, offset: u64, length: usize) -> Vec<u8> {
    let file = File::open(path).expect("open block view for reading");
    let mut bytes = vec![0; length];
    file.read_exact_at(&mut bytes, offset)
        .expect("read block view");
    bytes
}

fn command_text(program: &str, arguments: &[&str]) -> String {
    output_text(&run_checked(program, arguments))
        .trim()
        .to_owned()
}

fn run_checked(program: &str, arguments: &[&str]) -> Output {
    let output = command_output(program, arguments);
    assert!(
        output.status.success(),
        "{program} {arguments:?} failed: {}",
        output_text(&output)
    );
    output
}

fn command_output(program: &str, arguments: &[&str]) -> Output {
    Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("could not run {program}: {error}"))
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
