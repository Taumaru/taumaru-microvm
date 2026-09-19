use std::path::Path;

use crate::error::SdkError;

/// Replaceable process boundary for Firecracker host validation and state checks.
pub(crate) trait RuntimeController: Send + Sync {
    fn validate_host(&self) -> Result<(), SdkError>;

    fn verify_stopped(&self, socket_path: &Path) -> Result<(), SdkError>;
}
