use crate::domain::microvm::MicroVmStartResult;
use crate::error::SdkError;

/// Smallest accepted number of start attempts for one autostart run.
pub const MIN_AUTOSTART_ATTEMPTS: u32 = 1;
/// Largest accepted number of start attempts for one autostart run.
pub const MAX_AUTOSTART_ATTEMPTS: u32 = 10;
/// Start attempts used by [`AutostartSettings::default`].
pub const DEFAULT_AUTOSTART_ATTEMPTS: u32 = 3;

/// Persisted host-local policy that marks a MicroVM for automatic start.
///
/// The policy only records intent. Triggering [`crate::MicroVmSdk::start_autostart_microvms`]
/// at host boot is the caller's responsibility, so the SDK stays independent of any service
/// manager or executable. The policy is removed together with its MicroVM and is never
/// included in snapshot archives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutostartPolicy {
    /// Name of the MicroVM this policy belongs to.
    pub name: String,
    /// `false` keeps the policy stored but skips the machine during autostart runs.
    pub enabled: bool,
    /// Start attempts per autostart run, between [`MIN_AUTOSTART_ATTEMPTS`] and
    /// [`MAX_AUTOSTART_ATTEMPTS`].
    pub max_start_attempts: u32,
}

/// Complete settings for a new autostart policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutostartSettings {
    /// Whether autostart runs start the machine.
    pub enabled: bool,
    /// Start attempts per autostart run.
    pub max_start_attempts: u32,
}

impl Default for AutostartSettings {
    /// Enabled with [`DEFAULT_AUTOSTART_ATTEMPTS`] attempts.
    fn default() -> Self {
        Self {
            enabled: true,
            max_start_attempts: DEFAULT_AUTOSTART_ATTEMPTS,
        }
    }
}

/// Partial change to an existing autostart policy. `None` keeps the stored value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AutostartPolicyUpdate {
    /// New enabled flag.
    pub enabled: Option<bool>,
    /// New number of start attempts per autostart run.
    pub max_start_attempts: Option<u32>,
}

impl AutostartPolicyUpdate {
    /// Returns `true` when the update would not change any field.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.max_start_attempts.is_none()
    }
}

/// Result of removing an autostart policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutostartDeleteResult {
    /// Name of the MicroVM whose policy was targeted.
    pub name: String,
    /// `false` when no policy existed, which makes removal idempotent.
    pub removed: bool,
}

/// Per-machine result of one autostart run.
#[derive(Debug)]
pub struct AutostartOutcome {
    /// Name of the MicroVM.
    pub name: String,
    /// Start attempts consumed. `0` for paused policies.
    pub attempts: u32,
    /// Final result for this machine.
    pub result: AutostartResult,
}

/// Final result for one machine in an autostart run.
#[derive(Debug)]
pub enum AutostartResult {
    /// The machine is running, either freshly started or already live.
    Started(Box<MicroVmStartResult>),
    /// The policy is paused, so no start was attempted.
    Paused,
    /// Every attempt failed. Holds the error of the last attempt.
    Failed(SdkError),
}

/// Aggregated result of [`crate::MicroVmSdk::start_autostart_microvms`], ordered by name.
#[derive(Debug, Default)]
pub struct AutostartRunReport {
    /// One entry per stored policy.
    pub outcomes: Vec<AutostartOutcome>,
}

impl AutostartRunReport {
    /// Returns `true` when at least one enabled machine could not be started.
    pub fn has_failures(&self) -> bool {
        self.outcomes
            .iter()
            .any(|outcome| matches!(outcome.result, AutostartResult::Failed(_)))
    }
}

pub(crate) fn validate_max_start_attempts(attempts: u32) -> Result<(), SdkError> {
    if (MIN_AUTOSTART_ATTEMPTS..=MAX_AUTOSTART_ATTEMPTS).contains(&attempts) {
        return Ok(());
    }
    Err(SdkError::InvalidRequest {
        field: "max_start_attempts".to_owned(),
        reason: format!(
            "must be between {MIN_AUTOSTART_ATTEMPTS} and {MAX_AUTOSTART_ATTEMPTS}, got {attempts}"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempts_outside_the_supported_range_are_rejected() {
        assert!(validate_max_start_attempts(0).is_err());
        assert!(validate_max_start_attempts(1).is_ok());
        assert!(validate_max_start_attempts(10).is_ok());
        assert!(validate_max_start_attempts(11).is_err());
    }

    #[test]
    fn default_settings_are_enabled_with_default_attempts() {
        let settings = AutostartSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.max_start_attempts, DEFAULT_AUTOSTART_ATTEMPTS);
    }
}
