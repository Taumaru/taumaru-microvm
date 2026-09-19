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
    Sdk(SdkError),
    Io(io::Error),
}

impl From<SdkError> for CliError {
    fn from(error: SdkError) -> Self {
        Self::Sdk(error)
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

    pub(crate) fn escalation_unavailable() -> Self {
        Self::Creation(
            "Elevated rights are required\u{1f}neither sudo nor pkexec is available on this host\u{1f}Install sudo or polkit, or run the command as root"
                .to_string(),
        )
    }

    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::Prompt(message) if message == "cancelled" => 130,
            Self::Sdk(SdkError::Cancelled) => 130,
            Self::Cancelled => 130,
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
            Self::Sdk(SdkError::Cancelled) => (
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
            Self::Sdk(SdkError::Cancelled) => paint("!", ANSI_YELLOW, color),
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
