//! Reusable host-local MicroVM management capabilities.
//!
//! The SDK is intentionally silent and side-effect free until callers invoke an explicit
//! operation through its public API.

/// Returns a deterministic readiness message for the bootstrap SDK.
///
/// This is a temporary demonstration API. It may be replaced before the first stable release
/// and does not represent the future MicroVM manager API.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     taumaru_microvm::example_message(),
///     "taumaru-microvm SDK is ready"
/// );
/// ```
#[must_use]
pub fn example_message() -> &'static str {
    "taumaru-microvm SDK is ready"
}
