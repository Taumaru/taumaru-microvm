use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

use taumaru_microvm::MicroVmSdk;

use crate::error::CliError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalCapabilities {
    pub(crate) interactive: bool,
    pub(crate) color: bool,
    pub(crate) width: Option<usize>,
}

impl TerminalCapabilities {
    pub(crate) fn detect() -> Self {
        let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        let color =
            interactive && std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();
        let width = env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0);
        Self {
            interactive,
            color,
            width,
        }
    }
}

pub(crate) struct CliContext {
    pub(crate) sdk: MicroVmSdk,
    pub(crate) terminal: TerminalCapabilities,
}

impl CliContext {
    pub(crate) fn new() -> Result<Self, CliError> {
        let home = resolve_home()?;
        let sdk = MicroVmSdk::new(&home).map_err(CliError::from)?;
        Ok(Self {
            sdk,
            terminal: TerminalCapabilities::detect(),
        })
    }
}

pub(crate) fn resolve_home() -> Result<PathBuf, CliError> {
    resolve_home_from(
        env::var_os("TAUMARU_HOME"),
        env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_home_from(
    taumaru_home: Option<std::ffi::OsString>,
    user_home: Option<PathBuf>,
) -> Result<PathBuf, CliError> {
    if let Some(home) = taumaru_home {
        if home.is_empty() {
            return Err(CliError::Home(
                "TAUMARU_HOME is set but empty; provide a writable directory".to_owned(),
            ));
        }
        return Ok(PathBuf::from(home));
    }

    user_home
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.join(".taumaru-microvm"))
        .ok_or_else(|| CliError::Home("could not determine the user's home directory".to_owned()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::resolve_home_from;

    #[test]
    fn custom_home_wins_over_default_home() {
        let result = resolve_home_from(
            Some(OsString::from("/var/lib/taumaru")),
            Some(PathBuf::from("/home/user")),
        );
        assert_eq!(result.ok(), Some(PathBuf::from("/var/lib/taumaru")));
    }

    #[test]
    fn default_home_is_used_when_custom_home_is_unset() {
        let result = resolve_home_from(None, Some(PathBuf::from("/home/user")));
        assert_eq!(
            result.ok(),
            Some(PathBuf::from("/home/user/.taumaru-microvm"))
        );
    }
}
