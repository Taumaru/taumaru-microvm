use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitStatus;

use crate::context::TerminalCapabilities;
use crate::error::CliError;

pub(crate) const ESCALATED_MARKER: &str = "TAUMARU_ESCALATED";
const TAUMARU_HOME: &str = "TAUMARU_HOME";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EscalationBackend {
    Sudo,
    Polkit,
}

impl EscalationBackend {
    fn program(&self) -> &'static str {
        match self {
            Self::Sudo => "sudo",
            Self::Polkit => "pkexec",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EscalationPlan {
    pub(crate) backend: EscalationBackend,
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EscalationDecision {
    AlreadyPrivileged,
    Escalate(EscalationPlan),
    Unavailable,
}

pub(crate) trait Privilege {
    fn effective_user_id(&self) -> u32;
    fn escalated_marker(&self) -> bool;
    fn executable_path(&self) -> Result<PathBuf, CliError>;
    fn backend_path(&self, program: &str) -> Option<PathBuf>;
    async fn spawn_elevated(&self, plan: &EscalationPlan) -> Result<ExitStatus, CliError>;
}

pub(crate) struct SystemPrivilege;

impl Privilege for SystemPrivilege {
    fn effective_user_id(&self) -> u32 {
        #[cfg(unix)]
        {
            unsafe { libc_euid() }
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    fn escalated_marker(&self) -> bool {
        std::env::var_os(ESCALATED_MARKER).is_some_and(|value| !value.is_empty())
    }

    fn executable_path(&self) -> Result<PathBuf, CliError> {
        std::env::current_exe().map_err(CliError::from)
    }

    fn backend_path(&self, program: &str) -> Option<PathBuf> {
        backend_path(program)
    }

    async fn spawn_elevated(&self, plan: &EscalationPlan) -> Result<ExitStatus, CliError> {
        let mut command = tokio::process::Command::new(&plan.program);
        command.args(&plan.arguments);
        let mut child = command.spawn().map_err(CliError::from)?;
        let status = tokio::select! {
            status = child.wait() => status.map_err(CliError::from)?,
            _ = tokio::signal::ctrl_c() => {
                match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await {
                    Ok(status) => status.map_err(CliError::from)?,
                    Err(_) => {
                        child.kill().await.map_err(CliError::from)?;
                        child.wait().await.map_err(CliError::from)?
                    }
                }
            }
        };
        Ok(status)
    }
}

#[cfg(unix)]
unsafe fn libc_euid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

pub(crate) fn backend_path(program: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(format!("/usr/bin/{program}"));
    if direct.is_file() {
        return Some(direct);
    }
    let sbin = PathBuf::from(format!("/usr/sbin/{program}"));
    if sbin.is_file() {
        return Some(sbin);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
    })
}

pub(crate) fn decide<P: Privilege + ?Sized>(
    privilege: &P,
    backend: Option<EscalationBackend>,
) -> EscalationDecision {
    if privilege.effective_user_id() == 0 {
        return EscalationDecision::AlreadyPrivileged;
    }
    if privilege.escalated_marker() {
        return EscalationDecision::Unavailable;
    }
    let backends: &[EscalationBackend] = match backend {
        Some(backend) => match backend {
            EscalationBackend::Sudo => &[EscalationBackend::Sudo],
            EscalationBackend::Polkit => &[EscalationBackend::Polkit],
        },
        None => &[EscalationBackend::Sudo, EscalationBackend::Polkit],
    };
    for backend in backends {
        if privilege.backend_path(backend.program()).is_some() {
            return EscalationDecision::Escalate(EscalationPlan {
                backend: *backend,
                program: PathBuf::from(backend.program()),
                arguments: Vec::new(),
            });
        }
    }
    EscalationDecision::Unavailable
}

pub(crate) fn child_arguments(
    backend: EscalationBackend,
    executable: PathBuf,
    home: Option<PathBuf>,
    extra_env: &[(String, String)],
    command: Vec<OsString>,
) -> Vec<OsString> {
    let mut prefix = Vec::with_capacity(extra_env.len() + 2);
    if let Some(home) = home {
        prefix.push((TAUMARU_HOME.to_owned(), home.to_string_lossy().into_owned()));
    }
    prefix.push((ESCALATED_MARKER.to_owned(), "1".to_owned()));
    prefix.extend(extra_env.iter().cloned());
    let mut arguments = Vec::with_capacity(command.len() + prefix.len() + 2);
    match backend {
        EscalationBackend::Sudo => {
            for (name, value) in &prefix {
                arguments.push(OsString::from(format!("{name}={value}")));
            }
            arguments.push(executable.into_os_string());
        }
        EscalationBackend::Polkit => {
            arguments.push(OsString::from("env"));
            for (name, value) in &prefix {
                arguments.push(OsString::from(format!("{name}={value}")));
            }
            arguments.push(executable.into_os_string());
        }
    }
    arguments.extend(command);
    arguments
}

pub(crate) fn resolve_plan<P: Privilege + ?Sized>(
    privilege: &P,
    home: Option<PathBuf>,
    extra_env: &[(String, String)],
    command: Vec<OsString>,
) -> Result<EscalationDecision, CliError> {
    let executable = privilege.executable_path()?;
    Ok(match decide(privilege, None) {
        EscalationDecision::Escalate(plan) => {
            let program = privilege
                .backend_path(plan.backend.program())
                .unwrap_or(plan.program);
            EscalationDecision::Escalate(EscalationPlan {
                arguments: child_arguments(plan.backend, executable, home, extra_env, command),
                program,
                backend: plan.backend,
            })
        }
        decision => decision,
    })
}

pub(crate) async fn require_privileged<P: Privilege + ?Sized>(
    privilege: &P,
    terminal: TerminalCapabilities,
    non_interactive: bool,
    home: Option<PathBuf>,
    extra_env: &[(String, String)],
    command: Vec<OsString>,
    retry_hint: &str,
) -> Result<Option<u8>, CliError> {
    if privilege.effective_user_id() == 0 {
        return Ok(None);
    }
    if non_interactive || !terminal.interactive {
        return Err(CliError::privileged(retry_hint));
    }
    match resolve_plan(privilege, home, extra_env, command)? {
        EscalationDecision::AlreadyPrivileged => Ok(None),
        EscalationDecision::Unavailable => Err(CliError::escalation_unavailable()),
        EscalationDecision::Escalate(plan) => {
            let status = privilege.spawn_elevated(&plan).await?;
            Ok(Some(status.code().map(|code| code as u8).unwrap_or(130)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EscalationBackend, EscalationDecision, child_arguments, decide};
    use crate::context::TerminalCapabilities;
    use crate::error::CliError;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::process::ExitStatus;

    struct FakePrivilege {
        user_id: u32,
        escalated: bool,
        backends: HashMap<&'static str, PathBuf>,
    }

    impl super::Privilege for FakePrivilege {
        fn effective_user_id(&self) -> u32 {
            self.user_id
        }

        fn escalated_marker(&self) -> bool {
            self.escalated
        }

        fn executable_path(&self) -> Result<PathBuf, CliError> {
            Ok(PathBuf::from("/usr/bin/microvm"))
        }

        fn backend_path(&self, program: &str) -> Option<PathBuf> {
            self.backends.get(program).cloned()
        }

        async fn spawn_elevated(
            &self,
            _plan: &super::EscalationPlan,
        ) -> Result<ExitStatus, CliError> {
            Err(CliError::privileged("retry with elevated rights"))
        }
    }

    fn privilege() -> FakePrivilege {
        FakePrivilege {
            user_id: 1000,
            escalated: false,
            backends: HashMap::from([
                ("sudo", PathBuf::from("/usr/bin/sudo")),
                ("pkexec", PathBuf::from("/usr/bin/pkexec")),
            ]),
        }
    }

    #[test]
    fn root_needs_no_escalation() {
        let mut backend = privilege();
        backend.user_id = 0;
        assert_eq!(
            decide(&backend, None),
            EscalationDecision::AlreadyPrivileged
        );
    }

    #[test]
    fn escalated_child_never_re_escalates() {
        let mut backend = privilege();
        backend.escalated = true;
        assert_eq!(decide(&backend, None), EscalationDecision::Unavailable);
    }

    #[test]
    fn sudo_wins_over_polkit() {
        let backend = privilege();
        let EscalationDecision::Escalate(plan) = decide(&backend, None) else {
            panic!("escalation should be available");
        };
        assert_eq!(plan.backend, EscalationBackend::Sudo);
    }

    #[test]
    fn polkit_is_used_when_sudo_is_missing() {
        let mut backend = privilege();
        backend.backends.remove("sudo");
        let EscalationDecision::Escalate(plan) = decide(&backend, None) else {
            panic!("escalation should be available");
        };
        assert_eq!(plan.backend, EscalationBackend::Polkit);
    }

    #[test]
    fn no_backend_reports_unavailable() {
        let mut backend = privilege();
        backend.backends.clear();
        assert_eq!(decide(&backend, None), EscalationDecision::Unavailable);
    }

    #[test]
    fn sudo_carries_home_and_marker_inline() {
        let arguments = child_arguments(
            EscalationBackend::Sudo,
            PathBuf::from("/usr/bin/microvm"),
            Some(PathBuf::from("/home/user/.taumaru-microvm")),
            &[],
            vec![OsString::from("new"), OsString::from("web-01")],
        );
        assert_eq!(
            arguments,
            vec![
                OsString::from("TAUMARU_HOME=/home/user/.taumaru-microvm"),
                OsString::from("TAUMARU_ESCALATED=1"),
                OsString::from("/usr/bin/microvm"),
                OsString::from("new"),
                OsString::from("web-01"),
            ]
        );
    }

    #[test]
    fn polkit_carries_home_through_env() {
        let arguments = child_arguments(
            EscalationBackend::Polkit,
            PathBuf::from("/usr/bin/microvm"),
            Some(PathBuf::from("/home/user/.taumaru-microvm")),
            &[("TAUMARU_NEW_KERNEL".to_owned(), "linux-6.18".to_owned())],
            vec![OsString::from("new")],
        );
        assert_eq!(
            arguments,
            vec![
                OsString::from("env"),
                OsString::from("TAUMARU_HOME=/home/user/.taumaru-microvm"),
                OsString::from("TAUMARU_ESCALATED=1"),
                OsString::from("TAUMARU_NEW_KERNEL=linux-6.18"),
                OsString::from("/usr/bin/microvm"),
                OsString::from("new"),
            ]
        );
    }

    #[tokio::test]
    async fn non_interactive_without_root_errors_without_spawning() {
        let backend = privilege();
        let terminal = TerminalCapabilities {
            interactive: true,
            color: false,
            width: None,
        };
        let error = super::require_privileged(
            &backend,
            terminal,
            true,
            None,
            &[],
            Vec::new(),
            "Run the same command with sudo or as root",
        )
        .await
        .expect_err("non-interactive escalation should fail");
        let message = error.user_message(false);
        assert!(message.contains("Elevated rights are required"));
        assert!(message.contains("sudo"));
    }

    #[tokio::test]
    async fn root_passes_through_without_escalation() {
        let mut backend = privilege();
        backend.user_id = 0;
        let terminal = TerminalCapabilities {
            interactive: true,
            color: false,
            width: None,
        };
        assert_eq!(
            super::require_privileged(&backend, terminal, false, None, &[], Vec::new(), "hint",)
                .await
                .expect("root should pass through"),
            None
        );
    }

    #[tokio::test]
    async fn missing_backends_report_unavailable() {
        let mut backend = privilege();
        backend.backends.clear();
        let terminal = TerminalCapabilities {
            interactive: true,
            color: false,
            width: None,
        };
        let error =
            super::require_privileged(&backend, terminal, false, None, &[], Vec::new(), "hint")
                .await
                .expect_err("missing backends should fail");
        assert!(
            error
                .user_message(false)
                .to_ascii_lowercase()
                .contains("neither sudo nor pkexec")
        );
    }

    #[test]
    fn sudo_without_home_still_marks_escalation() {
        let arguments = child_arguments(
            EscalationBackend::Sudo,
            PathBuf::from("/usr/bin/microvm"),
            None,
            &[],
            vec![OsString::from("new")],
        );
        assert!(arguments.contains(&OsString::from("TAUMARU_ESCALATED=1")));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.to_string_lossy().starts_with("TAUMARU_HOME="))
        );
    }
}
