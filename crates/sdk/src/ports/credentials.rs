use std::path::{Path, PathBuf};

use crate::error::SdkError;

/// Public metadata returned by the credential adapter. It contains no private-key value.
#[derive(Clone, Debug)]
pub(crate) struct GeneratedCredential {
    pub private_key_path: PathBuf,
    pub public_key_path: PathBuf,
    pub public_key: String,
    pub fingerprint: String,
    pub file_mode: String,
}

/// Replaceable boundary for per-VM SSH credential generation.
pub(crate) trait CredentialStore: Send + Sync {
    fn generate(&self, volume_path: &Path) -> Result<GeneratedCredential, SdkError>;
}
