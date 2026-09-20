use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use crate::error::SdkError;
use crate::ports::runtime::{RuntimeController, StartRequest};

/// How long to wait for the control socket to answer after a launch.
const READINESS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Interval between socket readiness probes.
const READINESS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Internal firectl/Firecracker process adapter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FirecrackerRuntime;

impl RuntimeController for FirecrackerRuntime {
    fn validate_host(&self) -> Result<(), SdkError> {
        let kvm = Path::new("/dev/kvm");
        let metadata =
            fs::symlink_metadata(kvm).map_err(|error| SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: format!("/dev/kvm is unavailable: {error}"),
            })?;
        if !metadata.file_type().is_char_device() {
            return Err(SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: "/dev/kvm is not a character device".to_owned(),
            });
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(kvm)
            .map(|_| ())
            .map_err(|error| SdkError::RuntimeIncompatible {
                component: "kvm".to_owned(),
                package_id: "host".to_owned(),
                version: "unknown".to_owned(),
                architecture: std::env::consts::ARCH.to_owned(),
                reason: format!("/dev/kvm is not readable and writable: {error}"),
            })
    }

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError> {
        if path_entry_exists(socket_path)? {
            return Err(SdkError::TemporaryRuntime {
                component: "firecracker.sock".to_owned(),
                reason: "the VM socket is still present after stopping".to_owned(),
                stopped: false,
            });
        }
        Ok(())
    }

    fn process_references_vm(
        &self,
        process_id: u32,
        socket_path: &Path,
        firecracker_path: &Path,
    ) -> Result<bool, SdkError> {
        process_references_vm(process_id, socket_path, firecracker_path)
    }

    fn socket_answers(&self, socket_path: &Path) -> Result<bool, SdkError> {
        socket_answers(socket_path)
    }

    fn launch_detached(&self, request: &StartRequest) -> Result<u32, SdkError> {
        launch_detached(request)
    }

    fn wait_for_socket(&self, socket_path: &Path) -> Result<bool, SdkError> {
        wait_for_socket(socket_path, READINESS_DEADLINE)
    }

    fn terminate_spawned(&self, process_id: u32) -> Result<(), SdkError> {
        terminate_spawned(process_id)
    }
}

/// Builds the `firectl` argument vector for one detached VM launch.
pub(crate) fn start_arguments(request: &StartRequest) -> Vec<String> {
    let root_drive = format!("{}:rw", request.rootfs_path.display());
    vec![
        format!(
            "--firecracker-binary={}",
            request.firecracker_path.display()
        ),
        format!("--kernel={}", request.kernel_path.display()),
        format!("--kernel-opts={}", request.kernel_options),
        format!("--root-drive={root_drive}"),
        format!("--tap-device={}/{}", request.tap_name, request.guest_mac),
        format!("--ncpus={}", request.vcpu_count),
        format!("--memory={}", request.memory_effective_mib),
        format!("--socket-path={}", request.socket_path.display()),
        format!(
            "--firecracker-log={}",
            request.log_path.with_extension("vmm.log").display()
        ),
    ]
}

fn process_references_vm(
    process_id: u32,
    socket_path: &Path,
    firecracker_path: &Path,
) -> Result<bool, SdkError> {
    if process_id == 0 {
        return Ok(false);
    }
    let command_line_path = PathBuf::from(format!("/proc/{process_id}/cmdline"));
    let bytes = match fs::read(&command_line_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(SdkError::filesystem(
                "inspect machine process",
                &command_line_path,
                error,
            ));
        }
    };
    if bytes.is_empty() {
        return Ok(false);
    }
    let socket_text = socket_path.to_string_lossy();
    let firecracker_text = firecracker_path.to_string_lossy();
    let references_vm = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .any(|part| {
            let text = String::from_utf8_lossy(part);
            text.contains(socket_text.as_ref()) || text.contains(firecracker_text.as_ref())
        });
    Ok(references_vm)
}

fn socket_answers(socket_path: &Path) -> Result<bool, SdkError> {
    use std::os::unix::net::UnixStream;

    let mut stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(SdkError::filesystem(
                "connect to machine control socket",
                socket_path,
                error,
            ));
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| {
            SdkError::filesystem("configure machine control socket", socket_path, error)
        })?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| {
            SdkError::filesystem("configure machine control socket", socket_path, error)
        })?;
    let request =
        b"GET /machine-config HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n";
    if stream.write_all(request).is_err() {
        return Ok(false);
    }
    let mut response = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                if response.len() >= 8192 {
                    break;
                }
                if response.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return Ok(false),
        }
    }
    let status_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .unwrap_or_default();
    Ok(status_line.contains("200"))
}

fn launch_detached(request: &StartRequest) -> Result<u32, SdkError> {
    let log_path = request.log_path.clone();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SdkError::filesystem("create machine log directory", parent, error))?;
    }
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| SdkError::filesystem("open machine log file", &log_path, error))?;
    let stdout = log_file
        .try_clone()
        .map_err(|error| SdkError::filesystem("duplicate machine log file", &log_path, error))?;
    let mut command = std::process::Command::new(&request.firectl_path);
    command
        .args(start_arguments(request))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log_file));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn().map_err(|error| SdkError::HostCommand {
        program: "firectl".to_owned(),
        reason: format!(
            "could not launch the machine for VM {}: {error}",
            request.vm_name
        ),
    })?;
    let process_id = child.id();
    std::mem::forget(child);
    Ok(process_id)
}

fn wait_for_socket(socket_path: &Path, deadline: std::time::Duration) -> Result<bool, SdkError> {
    let started = std::time::Instant::now();
    loop {
        if socket_answers(socket_path)? {
            return Ok(true);
        }
        if started.elapsed() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(READINESS_POLL_INTERVAL);
    }
}

fn terminate_spawned(process_id: u32) -> Result<(), SdkError> {
    if process_id == 0 {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let raw = format!("{process_id}");
        let status = std::process::Command::new("kill")
            .args(["-KILL", "--", &raw])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| SdkError::HostCommand {
                program: "kill".to_owned(),
                reason: error.to_string(),
            })?;
        if status.success() {
            return Ok(());
        }
        if !process_exists(process_id)? {
            return Ok(());
        }
        Err(SdkError::HostCommand {
            program: "kill".to_owned(),
            reason: format!("could not terminate spawned process {process_id}"),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = process_id;
        Err(SdkError::TemporaryRuntime {
            component: "firecracker".to_owned(),
            reason: "process termination is only supported on Unix hosts".to_owned(),
            stopped: false,
        })
    }
}

fn process_exists(process_id: u32) -> Result<bool, SdkError> {
    let probe = PathBuf::from(format!("/proc/{process_id}/cmdline"));
    match fs::symlink_metadata(&probe) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem(
            "inspect spawned machine process",
            &probe,
            error,
        )),
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, SdkError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SdkError::filesystem("inspect runtime socket", path, error)),
    }
}
