use std::io;

use taumaru_microvm::SdkError;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum CliError {
    Home(String),
    Validation(String),
    Prompt(String),
    Creation(String),
    MissingValue(String),
    Provisioning(String),
    Conflict(String),
    Cancelled,
    Sdk(Box<SdkError>),
    Io(io::Error),
}

impl From<SdkError> for CliError {
    fn from(error: SdkError) -> Self {
        Self::Sdk(Box::new(error))
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl CliError {
    pub(crate) fn creation(
        what: impl Into<String>,
        why: impl Into<String>,
        next: impl Into<String>,
    ) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{}\u{1f}{}",
            what.into(),
            why.into(),
            next.into()
        ))
    }

    pub(crate) fn missing_value(
        field: impl Into<String>,
        flag: impl Into<String>,
        example: impl Into<String>,
    ) -> Self {
        Self::MissingValue(format!(
            "Missing {}\u{1f}non-interactive mode requires {}\u{1f}{}",
            field.into(),
            flag.into(),
            example.into()
        ))
    }

    pub(crate) fn provisioning(
        what: impl Into<String>,
        why: impl Into<String>,
        next: impl Into<String>,
    ) -> Self {
        Self::Provisioning(format!(
            "{}\u{1f}{}\u{1f}{}",
            what.into(),
            why.into(),
            next.into()
        ))
    }

    pub(crate) fn conflict(
        what: impl Into<String>,
        why: impl Into<String>,
        next: impl Into<String>,
    ) -> Self {
        Self::Conflict(format!(
            "{}\u{1f}{}\u{1f}{}",
            what.into(),
            why.into(),
            next.into()
        ))
    }

    pub(crate) fn cancelled() -> Self {
        Self::Cancelled
    }

    pub(crate) fn privileged(retry_hint: impl Into<String>) -> Self {
        Self::Creation(format!(
            "Elevated rights are required\u{1f}MicroVM creation configures host networking and storage\u{1f}{}",
            retry_hint.into()
        ))
    }

    pub(crate) fn start_not_found(name: &str) -> Self {
        Self::Creation(format!(
            "MicroVM {name:?} was not found\u{1f}no created machine named {name:?} exists in this home\u{1f}Run `microvm new` to create it, then try again"
        ))
    }

    pub(crate) fn start_empty_inventory() -> Self {
        Self::Creation(
            "No MicroVMs to start\u{1f}no created machines exist in this home\u{1f}Run `microvm new` to create one, then try again"
                .to_string(),
        )
    }

    pub(crate) fn start_cancelled() -> Self {
        Self::Creation(
            "MicroVM start cancelled\u{1f}no machine was started\u{1f}Run `microvm start` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn start_failed(what: impl Into<String>, error: &SdkError) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{error}\u{1f}Check the reported cause, then retry the start",
            what.into()
        ))
    }

    pub(crate) fn ssh_not_found(name: &str) -> Self {
        Self::Creation(format!(
            "MicroVM {name:?} was not found\u{1f}no running machine named {name:?} exists in this home\u{1f}Run `microvm new` to create it, then start it before connecting"
        ))
    }

    pub(crate) fn ssh_not_running(name: &str, state: impl Into<String>) -> Self {
        let state = state.into();
        Self::Creation(format!(
            "MicroVM {name:?} is not running\u{1f}machine {name:?} is currently {state}\u{1f}Run `microvm start {name}` and try again"
        ))
    }

    pub(crate) fn ssh_empty() -> Self {
        Self::Creation(
            "No running MicroVMs to connect to\u{1f}no machine is currently running in this home\u{1f}Run `microvm start` to start one, then try again"
                .to_string(),
        )
    }
    pub(crate) fn ssh_cancelled() -> Self {
        Self::Creation(
            "MicroVM connection cancelled\u{1f}no session was opened\u{1f}Run `microvm ssh` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn ssh_client_missing() -> Self {
        Self::Creation(
            "SSH client is unavailable\u{1f}no `ssh` program was found on this host\u{1f}Install the OpenSSH client, then try again"
                .to_string(),
        )
    }

    pub(crate) fn stop_not_found(name: &str) -> Self {
        Self::Creation(format!(
            "MicroVM {name:?} was not found\u{1f}no created machine named {name:?} exists in this home\u{1f}Run `microvm new` to create it, then try again"
        ))
    }

    pub(crate) fn stop_empty() -> Self {
        Self::Creation(
            "No running MicroVMs to stop\u{1f}no machine is currently running in this home\u{1f}Run `microvm start` to start one, then try again"
                .to_string(),
        )
    }

    pub(crate) fn stop_cancelled() -> Self {
        Self::Creation(
            "MicroVM stop cancelled\u{1f}no machine was stopped\u{1f}Run `microvm stop` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn stop_failed(what: impl Into<String>, error: &SdkError) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{error}\u{1f}Check the reported cause, then retry the stop",
            what.into()
        ))
    }

    pub(crate) fn snapshot_cancelled() -> Self {
        Self::Creation(
            "MicroVM snapshot cancelled\u{1f}the running VM was left active and the incomplete archive was removed\u{1f}Run `microvm snapshot` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn delete_not_found(name: &str) -> Self {
        Self::Creation(format!(
            "MicroVM {name:?} was not found\u{1f}no created machine named {name:?} exists in this home\u{1f}Run `microvm new` to create it, then try again"
        ))
    }

    pub(crate) fn delete_empty() -> Self {
        Self::Creation(
            "No MicroVMs to delete\u{1f}no created machines exist in this home\u{1f}Run `microvm new` to create one, then try again"
                .to_string(),
        )
    }

    pub(crate) fn delete_cancelled() -> Self {
        Self::Creation(
            "MicroVM delete cancelled\u{1f}no machine was deleted\u{1f}Run `microvm delete` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn delete_running(name: &str) -> Self {
        Self::Creation(format!(
            "MicroVM {name:?} is running\u{1f}machine {name:?} must be stopped before it can be deleted\u{1f}Run `microvm stop {name}`, then run `microvm delete {name}` again"
        ))
    }

    pub(crate) fn delete_failed(what: impl Into<String>, error: &SdkError) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{error}\u{1f}Check the reported cause, fix it, then retry the delete",
            what.into()
        ))
    }

    pub(crate) fn ls_failed(what: impl Into<String>, error: &SdkError) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{error}\u{1f}Check the reported cause, then retry the listing",
            what.into()
        ))
    }

    pub(crate) fn prune_failed(what: impl Into<String>, error: &SdkError) -> Self {
        Self::Creation(format!(
            "{}\u{1f}{error}\u{1f}Check the reported cause, then retry the prune",
            what.into()
        ))
    }

    pub(crate) fn prune_cancelled() -> Self {
        Self::Creation(
            "MicroVM prune cancelled\u{1f}no artifact was deleted\u{1f}Run `microvm artifacts prune` again when you are ready"
                .to_string(),
        )
    }

    pub(crate) fn ssh_key_unreadable(path: &std::path::Path) -> Self {
        Self::Creation(format!(
            "SSH key is missing or unreadable\u{1f}the private key at {} is not a readable file\u{1f}Repair the machine volume or recreate the machine, then try again",
            path.display()
        ))
    }

    pub(crate) fn escalation_unavailable() -> Self {
        Self::Creation(
            "Elevated rights are required\u{1f}neither sudo nor pkexec is available on this host\u{1f}Install sudo or polkit, or run the command as root"
                .to_string(),
        )
    }

    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::Prompt(message) if message == "cancelled" => 130,
            Self::Sdk(error) if matches!(error.as_ref(), SdkError::Cancelled) => 130,
            Self::Cancelled => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM start cancelled") => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM connection cancelled") => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM stop cancelled") => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM delete cancelled") => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM prune cancelled") => 130,
            Self::Creation(payload) if payload.starts_with("MicroVM snapshot cancelled") => 130,
            _ => 1,
        }
    }

    pub(crate) fn user_message(&self, color: bool) -> String {
        let (what, why, next) = match self {
            Self::Home(message) => (
                "Taumaru home is unavailable".to_owned(),
                message.clone(),
                "Set TAUMARU_HOME to a writable directory, then try again".to_owned(),
            ),
            Self::Validation(message) => (
                "Download selection is invalid".to_owned(),
                message.clone(),
                "Choose compatible registry image IDs as DISTRIBUTION_ID=IMAGE_ID and retry"
                    .to_owned(),
            ),
            Self::Prompt(message) if message == "cancelled" => (
                "Download cancelled".to_owned(),
                "No artifact transfer was started".to_owned(),
                "Run `microvm artifacts download` again when you are ready".to_owned(),
            ),
            Self::Prompt(message) => (
                "Download selection could not be completed".to_owned(),
                message.clone(),
                "Check terminal input and try again".to_owned(),
            ),
            Self::Sdk(error) if matches!(error.as_ref(), SdkError::Cancelled) => (
                "Download cancelled".to_owned(),
                "The SDK stopped before publishing an unverified partial artifact".to_owned(),
                "Retry `microvm artifacts download` to acquire the remaining groups".to_owned(),
            ),
            Self::Sdk(error) => (
                "Artifact preparation failed".to_owned(),
                error.to_string(),
                "Check registry access and local storage, then retry; verified artifacts will be reused"
                    .to_owned(),
            ),
            Self::Io(error) => (
                "CLI output failed".to_owned(),
                error.to_string(),
                "Check terminal permissions and try again".to_owned(),
            ),
            Self::Creation(payload)
            | Self::MissingValue(payload)
            | Self::Provisioning(payload)
            | Self::Conflict(payload) => {
                let mut parts = payload.split("\u{1f}");
                (
                    parts.next().unwrap_or_default().to_owned(),
                    parts.next().unwrap_or_default().to_owned(),
                    parts.next().unwrap_or_default().to_owned(),
                )
            }
            Self::Cancelled => (
                "MicroVM creation cancelled".to_owned(),
                "No MicroVM was created".to_owned(),
                "Run `microvm new` again when you are ready".to_owned(),
            ),
        };
        let marker = match self {
            Self::Prompt(message) if message == "cancelled" => paint("!", ANSI_YELLOW, color),
            Self::Sdk(error) if matches!(error.as_ref(), SdkError::Cancelled) => {
                paint("!", ANSI_YELLOW, color)
            }
            Self::Cancelled => paint("!", ANSI_YELLOW, color),
            Self::Validation(_) | Self::MissingValue(_) => paint("!", ANSI_YELLOW, color),
            _ => paint("×", ANSI_RED, color),
        };
        format!(
            "\n{} {}\n\n  {} {}\n\n  {} {}\n",
            marker,
            paint(&what, ANSI_BOLD, color),
            paint("Why:", ANSI_DIM, color),
            sentence(&why),
            paint("Next:", ANSI_BOLD, color),
            sentence(&next),
        )
    }
}

const ANSI_BOLD: &str = "\u{1b}[1m";
const ANSI_DIM: &str = "\u{1b}[2m";
const ANSI_YELLOW: &str = "\u{1b}[33m";
const ANSI_RED: &str = "\u{1b}[31m";
const ANSI_RESET: &str = "\u{1b}[0m";

fn paint(text: impl AsRef<str>, ansi: &str, color: bool) -> String {
    let text = text.as_ref();
    if color {
        format!("{ansi}{text}{ANSI_RESET}")
    } else {
        text.to_owned()
    }
}

fn sentence(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "No additional details were provided.".to_owned();
    }

    let mut result = String::new();
    if let Some(first) = trimmed.chars().next() {
        result.extend(first.to_uppercase());
        result.push_str(&trimmed[first.len_utf8()..]);
    }
    if !result.ends_with(['.', '!', '?']) {
        result.push('.');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::CliError;

    #[test]
    fn error_message_has_a_compact_actionable_hierarchy() {
        let message =
            CliError::Validation("at least one image is required".to_owned()).user_message(false);

        assert!(message.starts_with("\n! Download selection is invalid"));
        assert!(message.contains("Why: At least one image is required."));
        assert!(message.contains("Next: Choose compatible registry image IDs"));
        assert!(!message.contains("\u{1b}["));
    }
}
