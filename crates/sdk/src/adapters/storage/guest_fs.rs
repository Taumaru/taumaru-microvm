use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::SdkError;

static INJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Injects one public SSH key into an offline ext4 image.
///
/// The image is edited with `debugfs` in userspace. A kernel loop mount would
/// require `CAP_SYS_ADMIN`, so `mount -o loop` fails with exit status 32 for
/// regular unprivileged users.
///
/// Besides the client key in `/root/.ssh/authorized_keys`, this also ensures
/// the server host keys (`/etc/ssh/ssh_host_*_key`) exist so `sshd` can start
/// on images that ship without them. Existing host keys are preserved.
pub(crate) fn inject_public_key(rootfs_path: &Path, public_key: &str) -> Result<(), SdkError> {
    let key_line = public_key.trim_end_matches(['\r', '\n']);
    if key_line.is_empty() {
        return Err(SdkError::GuestFilesystem {
            operation: "verify public SSH key".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "the public key is empty".to_owned(),
        });
    }
    if key_line.contains('\n') || key_line.contains('\r') {
        return Err(SdkError::GuestFilesystem {
            operation: "verify public SSH key".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "the public key must be a single line".to_owned(),
        });
    }

    ensure_guest_directory(
        rootfs_path,
        "root",
        "inspect guest root directory",
        "verify guest root directory",
        "create guest root directory",
        "the expected /root directory is unavailable",
    )?;
    ensure_guest_directory(
        rootfs_path,
        "root/.ssh",
        "inspect guest SSH directory",
        "verify guest SSH directory",
        "create guest SSH directory",
        "the expected /root/.ssh directory is unavailable",
    )?;
    chmod_guest(rootfs_path, "root/.ssh", "040700")?;

    match stat_guest(
        rootfs_path,
        "root/.ssh/authorized_keys",
        "inspect guest authorized_keys",
    )? {
        None => {
            let mut contents = String::with_capacity(key_line.len() + 1);
            contents.push_str(key_line);
            contents.push('\n');
            write_guest_file(
                rootfs_path,
                "root/.ssh/authorized_keys",
                contents.as_bytes(),
                "inject public SSH key",
            )?;
        }
        Some(stat) => {
            if stat.is_symlink || !stat.is_regular {
                return Err(SdkError::GuestFilesystem {
                    operation: "verify guest authorized_keys".to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: "the expected authorized_keys path is not a regular file".to_owned(),
                });
            }
            let existing = cat_guest(rootfs_path, "root/.ssh/authorized_keys")?;
            if !existing.lines().any(|line| line.trim() == key_line) {
                let mut updated = existing;
                if !updated.is_empty() && !updated.ends_with('\n') {
                    updated.push('\n');
                }
                updated.push_str(key_line);
                updated.push('\n');
                replace_guest_file(rootfs_path, "root/.ssh/authorized_keys", updated.as_bytes())?;
            }
        }
    }

    chmod_guest(rootfs_path, "root/.ssh/authorized_keys", "0100600")?;
    ensure_host_keys(rootfs_path)
}

/// Server key types provisioned when the image ships without host keys.
///
/// This mirrors the default `ssh-keygen -A` set on Ubuntu images.
const HOST_KEY_TYPES: &[(&str, &str, Option<&str>)] = &[
    ("ed25519", "ssh_host_ed25519_key", None),
    ("rsa", "ssh_host_rsa_key", Some("-b 3072")),
    ("ecdsa", "ssh_host_ecdsa_key", None),
];

fn ensure_host_keys(rootfs_path: &Path) -> Result<(), SdkError> {
    ensure_guest_directory(
        rootfs_path,
        "etc",
        "inspect guest etc directory",
        "verify guest etc directory",
        "create guest etc directory",
        "the expected /etc directory is unavailable",
    )?;
    ensure_guest_directory(
        rootfs_path,
        "etc/ssh",
        "inspect guest SSH server directory",
        "verify guest SSH server directory",
        "create guest SSH server directory",
        "the expected /etc/ssh directory is unavailable",
    )?;
    for (key_type, file_name, extra_args) in HOST_KEY_TYPES {
        let guest_path = format!("etc/ssh/{file_name}");
        match stat_guest(rootfs_path, &guest_path, "inspect guest host key")? {
            Some(stat) => {
                if stat.is_symlink || !stat.is_regular {
                    return Err(SdkError::GuestFilesystem {
                        operation: "verify guest host key".to_owned(),
                        path: rootfs_path.to_path_buf(),
                        reason: format!("the expected {guest_path} path is not a regular file"),
                    });
                }
            }
            None => generate_host_key(rootfs_path, key_type, &guest_path, *extra_args)?,
        }
    }
    Ok(())
}

fn generate_host_key(
    rootfs_path: &Path,
    key_type: &str,
    guest_path: &str,
    extra_args: Option<&str>,
) -> Result<(), SdkError> {
    let sequence = INJECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let host_path = std::env::temp_dir().join(format!(
        ".taumaru-hostkey-{}-{sequence}",
        std::process::id()
    ));
    let host_arg = host_path.display().to_string();
    let mut arguments = vec!["-t", key_type, "-N", "", "-f", &host_arg, "-q"];
    if let Some(extra) = extra_args {
        arguments.extend(extra.split_whitespace());
    }
    let output = Command::new("ssh-keygen")
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "ssh-keygen".to_owned(),
            reason: error.to_string(),
        })?;
    if !output.status.success() {
        let _ = fs::remove_file(&host_path);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let mut reason = format!("ssh-keygen exited with {}", output.status);
        if !stderr.is_empty() {
            reason.push_str(": ");
            reason.push_str(&stderr.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        return Err(SdkError::HostCommand {
            program: "ssh-keygen".to_owned(),
            reason,
        });
    }
    let public_path = host_path.with_extension("pub");
    let private_bytes = fs::read(&host_path)
        .map_err(|error| SdkError::filesystem("read generated host key", &host_path, error))?;
    let public_bytes = fs::read(&public_path).map_err(|error| {
        SdkError::filesystem("read generated host public key", &public_path, error)
    })?;
    let _ = fs::remove_file(&host_path);
    let _ = fs::remove_file(&public_path);
    let public_guest_path = format!("{guest_path}.pub");
    write_guest_file(
        rootfs_path,
        guest_path,
        &private_bytes,
        "inject guest host key",
    )?;
    if let Err(error) = write_guest_file(
        rootfs_path,
        &public_guest_path,
        &public_bytes,
        "inject guest host public key",
    ) {
        let _ = run_debugfs(rootfs_path, &format!("rm {guest_path}"), true);
        return Err(error);
    }
    chmod_guest(rootfs_path, guest_path, "0100600")?;
    chmod_guest(rootfs_path, &public_guest_path, "0100644")
}

pub(crate) fn write_guest_network_config(
    rootfs_path: &Path,
    guest_address: std::net::Ipv4Addr,
    gateway: std::net::Ipv4Addr,
) -> Result<(), SdkError> {
    for parent in ["etc", "etc/systemd"] {
        ensure_guest_directory(
            rootfs_path,
            parent,
            "inspect guest network parent",
            "verify guest network parent",
            "create guest network parent",
            "the expected parent directory is unavailable",
        )?;
    }
    ensure_guest_directory(
        rootfs_path,
        "etc/systemd/network",
        "inspect guest network directory",
        "verify guest network directory",
        "create guest network directory",
        "the expected /etc/systemd/network directory is unavailable",
    )?;
    let contents = format!(
        "[Match]\nName=eth0\n\n[Network]\nAddress={guest_address}/30\nGateway={gateway}\nDNS=1.1.1.1\nDNS=8.8.8.8\nDHCP=no\n"
    );
    match stat_guest(
        rootfs_path,
        "etc/systemd/network/10-taumaru.network",
        "inspect guest network unit",
    )? {
        Some(stat) => {
            if stat.is_symlink || !stat.is_regular {
                return Err(SdkError::GuestFilesystem {
                    operation: "verify guest network unit".to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: "the expected 10-taumaru.network path is not a regular file".to_owned(),
                });
            }
            replace_guest_file(
                rootfs_path,
                "etc/systemd/network/10-taumaru.network",
                contents.as_bytes(),
            )?;
        }
        None => {
            write_guest_file(
                rootfs_path,
                "etc/systemd/network/10-taumaru.network",
                contents.as_bytes(),
                "inject guest network unit",
            )?;
        }
    }
    chmod_guest(
        rootfs_path,
        "etc/systemd/network/10-taumaru.network",
        "0100644",
    )
}

struct GuestStat {
    is_symlink: bool,
    is_directory: bool,
    is_regular: bool,
}

fn ensure_guest_directory(
    rootfs_path: &Path,
    guest_path: &str,
    inspect_operation: &str,
    verify_operation: &str,
    create_operation: &str,
    unavailable_reason: &str,
) -> Result<(), SdkError> {
    let stat = stat_guest(rootfs_path, guest_path, inspect_operation)?;
    let stat = match stat {
        Some(stat) => stat,
        None => {
            mkdir_guest(rootfs_path, guest_path, create_operation)?;
            stat_guest(rootfs_path, guest_path, inspect_operation)?.ok_or_else(|| {
                SdkError::GuestFilesystem {
                    operation: verify_operation.to_owned(),
                    path: rootfs_path.to_path_buf(),
                    reason: unavailable_reason.to_owned(),
                }
            })?
        }
    };
    if stat.is_symlink || !stat.is_directory {
        return Err(SdkError::GuestFilesystem {
            operation: verify_operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: unavailable_reason.to_owned(),
        });
    }
    Ok(())
}

fn stat_guest(
    rootfs_path: &Path,
    guest_path: &str,
    inspect_operation: &str,
) -> Result<Option<GuestStat>, SdkError> {
    let output = run_debugfs(rootfs_path, &format!("stat {guest_path}"), false)?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: inspect_operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if let Some(stat) = parse_stat(&stdout) {
        return Ok(Some(stat));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.contains("File not found") {
        return Ok(None);
    }
    Err(SdkError::GuestFilesystem {
        operation: inspect_operation.to_owned(),
        path: rootfs_path.to_path_buf(),
        reason: guest_debugfs_reason(&stderr),
    })
}

fn parse_stat(stdout: &str) -> Option<GuestStat> {
    if !stdout.contains("Inode:") {
        return None;
    }
    let kind = stdout.split("Type:").nth(1)?.split_whitespace().next()?;
    Some(GuestStat {
        is_symlink: kind == "symlink",
        is_directory: kind == "directory",
        is_regular: kind == "regular",
    })
}

fn cat_guest(rootfs_path: &Path, guest_path: &str) -> Result<String, SdkError> {
    let output = run_debugfs(rootfs_path, &format!("cat {guest_path}"), false)?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "read guest authorized_keys".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.contains("File not found") || debugfs_failed(&stderr) {
        return Err(SdkError::GuestFilesystem {
            operation: "read guest authorized_keys".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: guest_debugfs_reason(&stderr),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| SdkError::GuestFilesystem {
        operation: "read guest authorized_keys".to_owned(),
        path: rootfs_path.to_path_buf(),
        reason: "the file is not valid UTF-8".to_owned(),
    })
}

fn mkdir_guest(rootfs_path: &Path, guest_path: &str, operation: &str) -> Result<(), SdkError> {
    let output = run_debugfs(rootfs_path, &format!("mkdir {guest_path}"), true)?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.contains("already exists") {
        return Ok(());
    }
    if debugfs_failed(&stderr) {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: guest_debugfs_reason(&stderr),
        });
    }
    Ok(())
}

fn replace_guest_file(
    rootfs_path: &Path,
    guest_path: &str,
    contents: &[u8],
) -> Result<(), SdkError> {
    let output = run_debugfs(rootfs_path, &format!("rm {guest_path}"), true)?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "remove guest authorized_keys for update".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !stderr.contains("File not found") && debugfs_failed(&stderr) {
        return Err(SdkError::GuestFilesystem {
            operation: "remove guest authorized_keys for update".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: guest_debugfs_reason(&stderr),
        });
    }
    write_guest_file(rootfs_path, guest_path, contents, "inject public SSH key")
}

fn write_guest_file(
    rootfs_path: &Path,
    guest_path: &str,
    contents: &[u8],
    operation: &str,
) -> Result<(), SdkError> {
    let host_path = write_temp_payload(contents)?;
    let request = format!("write {} {guest_path}", quote_host_path(&host_path));
    let output = match run_debugfs(rootfs_path, &request, true) {
        Ok(output) => output,
        Err(error) => {
            let _ = fs::remove_file(&host_path);
            return Err(error);
        }
    };
    let _ = fs::remove_file(&host_path);
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.contains("already exists") {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "the guest file already exists".to_owned(),
        });
    }
    if debugfs_failed(&stderr) {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: guest_debugfs_reason(&stderr),
        });
    }
    if !stdout.contains("Allocated inode") {
        return Err(SdkError::GuestFilesystem {
            operation: operation.to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: "debugfs did not confirm the write".to_owned(),
        });
    }
    Ok(())
}

fn chmod_guest(rootfs_path: &Path, guest_path: &str, mode: &str) -> Result<(), SdkError> {
    let output = run_debugfs(rootfs_path, &format!("sif {guest_path} mode {mode}"), true)?;
    if !output.status.success() {
        return Err(SdkError::GuestFilesystem {
            operation: "set guest SSH permissions".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: format!("debugfs exited with {}", output.status),
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if debugfs_failed(&stderr) {
        return Err(SdkError::GuestFilesystem {
            operation: "set guest SSH permissions".to_owned(),
            path: rootfs_path.to_path_buf(),
            reason: guest_debugfs_reason(&stderr),
        });
    }
    Ok(())
}

fn quote_host_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text.chars().any(|candidate| {
        candidate.is_whitespace() || matches!(candidate, '"' | '\\' | '\'' | '`' | '$' | '!' | '*')
    }) {
        let mut quoted = String::with_capacity(text.len() + 2);
        quoted.push('"');
        for character in text.chars() {
            if matches!(character, '"' | '\\') {
                quoted.push('\\');
            }
            quoted.push(character);
        }
        quoted.push('"');
        quoted
    } else {
        text.into_owned()
    }
}

fn write_temp_payload(contents: &[u8]) -> Result<PathBuf, SdkError> {
    let sequence = INJECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        ".taumaru-ssh-{}-{sequence}.tmp",
        std::process::id()
    ));
    fs::write(&path, contents)
        .map_err(|error| SdkError::filesystem("write SSH injection payload", &path, error))?;
    Ok(path)
}

fn run_debugfs(
    rootfs_path: &Path,
    request: &str,
    write: bool,
) -> Result<std::process::Output, SdkError> {
    let mut command = Command::new("debugfs");
    if write {
        command.arg("-w");
    }
    command
        .arg("-R")
        .arg(request)
        .arg(rootfs_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.output().map_err(|error| SdkError::HostCommand {
        program: "debugfs".to_owned(),
        reason: error.to_string(),
    })
}

/// Reports whether `debugfs` stderr carries a real failure.
///
/// `debugfs` always prints its version banner, and some commands print
/// library notices, so only known failure markers count as errors.
fn debugfs_failed(stderr: &str) -> bool {
    const MARKERS: &[&str] = &[
        "file not found",
        "already exists",
        "bad magic",
        "not open",
        "read-only",
        "denied",
        "no such",
        "invalid",
        "could not",
        "cannot",
        "failed",
        "error",
    ];
    let relevant: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("debugfs")
                && !line.contains("Using EXT2FS Library")
        })
        .collect();
    if relevant.is_empty() {
        return false;
    }
    let joined = relevant.join("\n").to_lowercase();
    MARKERS.iter().any(|marker| joined.contains(marker))
}

fn guest_debugfs_reason(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| {
            !line.is_empty()
                && !line.starts_with("debugfs")
                && !line.contains("Using EXT2FS Library")
        })
        .unwrap_or("debugfs reported an error")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use tempfile::tempdir;

    use super::{inject_public_key, quote_host_path};
    use crate::error::SdkError;

    const FIRST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1taumaru-first taumaru-test";
    const SECOND_KEY: &str = "ssh-ed25519 AAAAC3NzaC1taumaru-second taumaru-test";

    fn host_tool_available(program: &str) -> bool {
        Command::new(program)
            .arg("-V")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok()
    }

    fn e2fsprogs_available() -> bool {
        host_tool_available("debugfs") && host_tool_available("mkfs.ext4")
    }

    fn create_ext4_image(path: &Path) {
        const IMAGE_SIZE_BYTES: u64 = 64 * 1024 * 1024;
        let file = fs::File::create(path).expect("image file should be created");
        file.set_len(IMAGE_SIZE_BYTES)
            .expect("image file should be sized");
        drop(file);
        let status = Command::new("mkfs.ext4")
            .arg("-q")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("mkfs.ext4 should execute");
        assert!(status.success(), "mkfs.ext4 should succeed");
    }

    fn debugfs_exec(image: &Path, request: &str) {
        let output = Command::new("debugfs")
            .args(["-w", "-R", request])
            .arg(image)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("debugfs should execute");
        assert!(
            output.status.success(),
            "debugfs {request} should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn guest_file_content(image: &Path, guest_path: &str) -> String {
        let output = Command::new("debugfs")
            .args(["-R", &format!("cat {guest_path}")])
            .arg(image)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .expect("debugfs should execute");
        assert!(output.status.success(), "debugfs cat should succeed");
        String::from_utf8(output.stdout).expect("guest file should be UTF-8")
    }

    fn guest_stat(image: &Path, guest_path: &str) -> String {
        let output = Command::new("debugfs")
            .args(["-R", &format!("stat {guest_path}")])
            .arg(image)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .expect("debugfs should execute");
        assert!(output.status.success(), "debugfs stat should succeed");
        String::from_utf8(output.stdout).expect("debugfs stat should be UTF-8")
    }

    fn guest_file_bytes(image: &Path, guest_path: &str) -> Vec<u8> {
        let output = Command::new("debugfs")
            .args(["-R", &format!("cat {guest_path}")])
            .arg(image)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .expect("debugfs should execute");
        assert!(output.status.success(), "debugfs cat should succeed");
        output.stdout
    }

    #[test]
    fn injects_a_key_without_host_mount_privileges() {
        if !e2fsprogs_available() {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);

        inject_public_key(&image, FIRST_KEY).expect("first injection should succeed");
        inject_public_key(&image, FIRST_KEY).expect("repeated injection should succeed");

        assert_eq!(
            guest_file_content(&image, "root/.ssh/authorized_keys"),
            format!("{FIRST_KEY}\n")
        );
        assert!(
            guest_stat(&image, "root/.ssh").contains("0700"),
            "guest SSH directory should be private"
        );
        assert!(
            guest_stat(&image, "root/.ssh/authorized_keys").contains("0600"),
            "guest authorized_keys should be private"
        );
    }

    #[test]
    fn provisions_host_keys_when_the_image_has_none() {
        if !e2fsprogs_available() || !host_tool_available("ssh-keygen") {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);

        inject_public_key(&image, FIRST_KEY).expect("injection should succeed");

        for key_type in ["ed25519", "rsa", "ecdsa"] {
            let guest_path = format!("etc/ssh/ssh_host_{key_type}_key");
            let stat = guest_stat(&image, &guest_path);
            assert!(stat.contains("0600"), "{guest_path} should be private");
            let public_stat = guest_stat(&image, &format!("{guest_path}.pub"));
            assert!(
                public_stat.contains("0644"),
                "{guest_path}.pub should be readable"
            );
        }
        let private = guest_file_bytes(&image, "etc/ssh/ssh_host_ed25519_key");
        assert!(private.starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----"));
    }

    #[test]
    fn preserves_existing_host_keys_on_repeated_injection() {
        if !e2fsprogs_available() || !host_tool_available("ssh-keygen") {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);

        inject_public_key(&image, FIRST_KEY).expect("first injection should succeed");
        let before = guest_file_bytes(&image, "etc/ssh/ssh_host_ed25519_key");
        inject_public_key(&image, SECOND_KEY).expect("second injection should succeed");
        let after = guest_file_bytes(&image, "etc/ssh/ssh_host_ed25519_key");

        assert_eq!(before, after, "existing host keys must be preserved");
        assert_eq!(
            guest_file_content(&image, "root/.ssh/authorized_keys"),
            format!("{FIRST_KEY}\n{SECOND_KEY}\n")
        );
    }

    #[test]
    fn rejects_a_symlinked_host_key_without_following_it() {
        if !e2fsprogs_available() || !host_tool_available("ssh-keygen") {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);
        debugfs_exec(&image, "mkdir etc");
        debugfs_exec(&image, "mkdir etc/ssh");
        debugfs_exec(&image, "symlink etc/ssh/ssh_host_ed25519_key some-target");

        let error =
            inject_public_key(&image, FIRST_KEY).expect_err("symlink injection should fail");

        assert!(
            matches!(error, SdkError::GuestFilesystem { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn appends_a_second_key_preserving_existing_entries() {
        if !e2fsprogs_available() {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);
        debugfs_exec(&image, "mkdir root");
        debugfs_exec(&image, "mkdir root/.ssh");
        let initial = directory.path().join("initial.txt");
        fs::write(&initial, FIRST_KEY).expect("initial key file should be written");
        debugfs_exec(
            &image,
            &format!("write {} root/.ssh/authorized_keys", initial.display()),
        );

        inject_public_key(&image, SECOND_KEY).expect("injection should succeed");

        assert_eq!(
            guest_file_content(&image, "root/.ssh/authorized_keys"),
            format!("{FIRST_KEY}\n{SECOND_KEY}\n")
        );
    }

    #[test]
    fn rejects_a_symlinked_authorized_keys_without_following_it() {
        if !e2fsprogs_available() {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);
        debugfs_exec(&image, "mkdir root");
        debugfs_exec(&image, "mkdir root/.ssh");
        debugfs_exec(&image, "symlink root/.ssh/authorized_keys some-target");

        let error =
            inject_public_key(&image, FIRST_KEY).expect_err("symlink injection should fail");

        assert!(
            matches!(error, SdkError::GuestFilesystem { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_an_empty_public_key_before_touching_the_image() {
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");

        let error = inject_public_key(&image, "\n").expect_err("empty key should fail");

        assert!(
            matches!(error, SdkError::GuestFilesystem { .. }),
            "unexpected error: {error}"
        );
        assert!(!image.exists());
    }

    #[test]
    fn quotes_host_payload_paths_with_whitespace() {
        use std::path::PathBuf;

        assert_eq!(
            quote_host_path(&PathBuf::from("/tmp/payload.tmp")),
            "/tmp/payload.tmp"
        );
        assert_eq!(
            quote_host_path(&PathBuf::from("/tmp/dir with space/payload.tmp")),
            "\"/tmp/dir with space/payload.tmp\""
        );
    }

    #[test]
    fn writes_a_static_network_unit_with_allocated_addresses() {
        use std::net::Ipv4Addr;

        if !e2fsprogs_available() {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);
        let guest = Ipv4Addr::new(172, 30, 0, 6);
        let gateway = Ipv4Addr::new(172, 30, 0, 5);

        super::write_guest_network_config(&image, guest, gateway)
            .expect("network unit should be written");
        super::write_guest_network_config(&image, guest, gateway)
            .expect("repeated write should overwrite");

        assert_eq!(
            guest_file_content(&image, "etc/systemd/network/10-taumaru.network"),
            "[Match]\nName=eth0\n\n[Network]\nAddress=172.30.0.6/30\nGateway=172.30.0.5\nDNS=1.1.1.1\nDNS=8.8.8.8\nDHCP=no\n"
        );
        assert!(
            guest_stat(&image, "etc/systemd/network/10-taumaru.network").contains("0644"),
            "network unit should be readable"
        );
    }

    #[test]
    fn rejects_a_symlinked_network_unit_without_following_it() {
        use std::net::Ipv4Addr;

        if !e2fsprogs_available() {
            return;
        }
        let directory = tempdir().expect("temporary directory should exist");
        let image = directory.path().join("rootfs.ext4");
        create_ext4_image(&image);
        debugfs_exec(&image, "mkdir etc");
        debugfs_exec(&image, "mkdir etc/systemd");
        debugfs_exec(&image, "mkdir etc/systemd/network");
        debugfs_exec(
            &image,
            "symlink etc/systemd/network/10-taumaru.network some-target",
        );

        let error = super::write_guest_network_config(
            &image,
            Ipv4Addr::new(172, 30, 0, 6),
            Ipv4Addr::new(172, 30, 0, 5),
        )
        .expect_err("symlink unit should fail");

        assert!(
            matches!(error, SdkError::GuestFilesystem { .. }),
            "unexpected error: {error}"
        );
    }
}
