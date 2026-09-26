use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::error::CliError;

const UNIT_PREFIX: &str = "taumaru-microvm-autostart";

/// Boot integration for autostart policies through one systemd template unit.
///
/// Each Taumaru home gets its own unit instance whose name is the systemd path escape of the
/// home, so homes selected through `TAUMARU_HOME` never overwrite each other.
pub(crate) struct SystemdAutostart {
    unit_directory: PathBuf,
    systemd_runtime_directory: PathBuf,
}

impl Default for SystemdAutostart {
    fn default() -> Self {
        Self {
            unit_directory: PathBuf::from("/etc/systemd/system"),
            systemd_runtime_directory: PathBuf::from("/run/systemd/system"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BootState {
    Enabled { unit: String },
    Disabled { unit: String },
}

impl SystemdAutostart {
    pub(crate) fn ensure_available(&self) -> Result<(), CliError> {
        if self.systemd_runtime_directory.is_dir() {
            return Ok(());
        }
        Err(CliError::creation(
            "Autostart requires systemd",
            "this host is not running systemd, so no boot service can start MicroVMs",
            "Start machines with `microvm start` from your own boot scripts instead",
        ))
    }

    pub(crate) async fn sync(
        &self,
        home: &Path,
        executable: &Path,
        any_enabled: bool,
    ) -> Result<BootState, CliError> {
        let home = absolute_home(home)?;
        let unit = instance_unit(&home);
        if any_enabled {
            if self.write_template(executable)? {
                systemctl(&["daemon-reload"], &unit).await?;
            }
            systemctl(&["enable", &unit], &unit).await?;
            Ok(BootState::Enabled { unit })
        } else {
            if self.template_path().is_file() {
                systemctl(&["disable", &unit], &unit).await?;
            }
            Ok(BootState::Disabled { unit })
        }
    }

    fn template_path(&self) -> PathBuf {
        self.unit_directory.join(format!("{UNIT_PREFIX}@.service"))
    }

    fn write_template(&self, executable: &Path) -> Result<bool, CliError> {
        let path = self.template_path();
        let content = render_template_unit(executable);
        if fs::read_to_string(&path).is_ok_and(|current| current == content) {
            return Ok(false);
        }
        fs::create_dir_all(&self.unit_directory).map_err(|error| boot_file_error(&path, error))?;
        fs::write(&path, content).map_err(|error| boot_file_error(&path, error))?;
        Ok(true)
    }
}

pub(crate) fn instance_unit(home: &Path) -> String {
    format!("{UNIT_PREFIX}@{}.service", escape_path(home.as_os_str()))
}

pub(crate) fn render_template_unit(executable: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=Start Taumaru MicroVMs configured for autostart in %f\n\
         Wants=network-online.target\n\
         After=network-online.target local-fs.target\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         RemainAfterExit=yes\n\
         Environment=TAUMARU_HOME=%f\n\
         ExecStart={} autostart run\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        quote_exec_argument(executable)
    )
}

// Mirrors `systemd-escape --path` so the instance name round-trips through `%f`.
pub(crate) fn escape_path(path: &OsStr) -> String {
    let trimmed = trim_slashes(path.as_bytes());
    if trimmed.is_empty() {
        return "-".to_owned();
    }
    let mut escaped = String::with_capacity(trimmed.len());
    for (index, byte) in trimmed.iter().copied().enumerate() {
        match byte {
            b'/' => escaped.push('-'),
            b'.' if index == 0 => escaped.push_str("\\x2e"),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b':' | b'_' | b'.' => {
                escaped.push(char::from(byte));
            }
            _ => escaped.push_str(&format!("\\x{byte:02x}")),
        }
    }
    escaped
}

fn trim_slashes(bytes: &[u8]) -> Vec<u8> {
    let mut collapsed = Vec::with_capacity(bytes.len());
    for byte in bytes.iter().copied() {
        if byte == b'/' && collapsed.last() == Some(&b'/') {
            continue;
        }
        collapsed.push(byte);
    }
    let start = collapsed.iter().position(|byte| *byte != b'/');
    let end = collapsed.iter().rposition(|byte| *byte != b'/');
    match (start, end) {
        (Some(start), Some(end)) => collapsed[start..=end].to_vec(),
        _ => Vec::new(),
    }
}

fn quote_exec_argument(executable: &Path) -> String {
    let raw = executable.to_string_lossy();
    let escaped = raw
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

fn absolute_home(home: &Path) -> Result<PathBuf, CliError> {
    fs::canonicalize(home).map_err(|error| {
        CliError::creation(
            "Taumaru home could not be resolved",
            format!("{}: {error}", home.display()),
            "Set TAUMARU_HOME to an existing directory, then try again",
        )
    })
}

fn boot_file_error(path: &Path, error: std::io::Error) -> CliError {
    CliError::creation(
        "Boot service file could not be written",
        format!("{}: {error}", path.display()),
        "Check that /etc/systemd/system is writable by root, then run the same command again",
    )
}

async fn systemctl(arguments: &[&str], unit: &str) -> Result<(), CliError> {
    let output = tokio::process::Command::new("systemctl")
        .args(arguments)
        .output()
        .await
        .map_err(|error| systemctl_error(arguments, unit, &error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(systemctl_error(arguments, unit, stderr.trim()))
}

fn systemctl_error(arguments: &[&str], unit: &str, reason: &str) -> CliError {
    CliError::creation(
        "Boot service could not be updated",
        format!("systemctl {} failed: {reason}", arguments.join(" ")),
        format!(
            "The autostart policy is saved; check `systemctl status {unit}`, then run the same command again"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::{BootState, SystemdAutostart, escape_path, instance_unit, render_template_unit};

    #[test]
    fn paths_escape_like_systemd_escape() {
        assert_eq!(
            escape_path(OsStr::new("/home/dev/.taumaru-microvm")),
            "home-dev-.taumaru\\x2dmicrovm"
        );
        assert_eq!(
            escape_path(OsStr::new("/var/lib//taumaru/")),
            "var-lib-taumaru"
        );
        assert_eq!(escape_path(OsStr::new("/.hidden")), "\\x2ehidden");
        assert_eq!(escape_path(OsStr::new("/srv/my vms")), "srv-my\\x20vms");
        assert_eq!(escape_path(OsStr::new("/")), "-");
    }

    #[test]
    fn instance_unit_is_scoped_to_the_home() {
        assert_eq!(
            instance_unit(Path::new("/var/lib/taumaru")),
            "taumaru-microvm-autostart@var-lib-taumaru.service"
        );
    }

    #[test]
    fn template_unit_runs_the_autostart_command_for_the_instance_home() {
        let unit = render_template_unit(Path::new("/usr/local/bin/microvm"));
        assert!(unit.contains("ExecStart=\"/usr/local/bin/microvm\" autostart run\n"));
        assert!(unit.contains("Environment=TAUMARU_HOME=%f\n"));
        assert!(unit.contains("After=network-online.target"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn template_unit_escapes_specifiers_and_quotes_in_the_executable() {
        let unit = render_template_unit(Path::new("/opt/100%/micro\"vm"));
        assert!(unit.contains("ExecStart=\"/opt/100%%/micro\\\"vm\" autostart run\n"));
    }

    #[test]
    fn template_is_only_rewritten_when_its_content_changes() {
        let directory = tempfile::tempdir().expect("unit directory should be created");
        let boot = SystemdAutostart {
            unit_directory: directory.path().to_path_buf(),
            systemd_runtime_directory: directory.path().to_path_buf(),
        };
        let executable = Path::new("/usr/bin/microvm");
        assert!(boot.write_template(executable).expect("first write"));
        assert!(!boot.write_template(executable).expect("unchanged write"));
        assert!(
            boot.write_template(Path::new("/usr/local/bin/microvm"))
                .expect("changed write")
        );
    }

    #[test]
    fn missing_systemd_is_reported_before_any_change() {
        let directory = tempfile::tempdir().expect("directory should be created");
        let boot = SystemdAutostart {
            unit_directory: directory.path().join("units"),
            systemd_runtime_directory: directory.path().join("absent"),
        };
        let error = boot
            .ensure_available()
            .expect_err("missing systemd should fail");
        assert!(error.user_message(false).contains("requires systemd"));
        assert!(!directory.path().join("units").exists());
    }

    #[tokio::test]
    async fn disabling_without_an_installed_template_needs_no_systemctl() {
        let directory = tempfile::tempdir().expect("directory should be created");
        let boot = SystemdAutostart {
            unit_directory: directory.path().join("units"),
            systemd_runtime_directory: directory.path().to_path_buf(),
        };
        let state = boot
            .sync(directory.path(), Path::new("/usr/bin/microvm"), false)
            .await
            .expect("disable should succeed");
        assert!(matches!(state, BootState::Disabled { .. }));
    }
}
