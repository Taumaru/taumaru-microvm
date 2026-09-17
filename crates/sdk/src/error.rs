use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by public SDK operations.
#[derive(Debug, Error)]
pub enum SdkError {
    /// The caller supplied a home path that could not be normalized or created.
    #[error("invalid SDK home path {path}: {reason}")]
    InvalidHome { path: PathBuf, reason: String },

    /// A registry-controlled value cannot be used safely.
    #[error("invalid registry metadata for {artifact}: {reason}")]
    InvalidMetadata { artifact: String, reason: String },

    /// A registry URL is not an accepted HTTP(S) URL.
    #[error("invalid registry URL {url}")]
    InvalidUrl { url: String },

    /// A requested registry artifact does not exist in the current manifest.
    #[error("{kind} artifact {id} was not found in the registry")]
    NotFound { kind: String, id: String },

    /// A registry request could not be completed.
    #[error("registry request failed: {0}")]
    RegistryTransport(#[source] reqwest::Error),

    /// The registry returned a non-success HTTP response.
    #[error("registry returned HTTP status {status} for {url}")]
    RegistryStatus {
        url: String,
        status: reqwest::StatusCode,
    },

    /// The registry response did not match the official JSON definition.
    #[error("registry response could not be decoded: {0}")]
    RegistryDecode(#[source] serde_json::Error),

    /// The registry advertises a schema not supported by this SDK.
    #[error("unsupported registry schema version {actual}; supported version is {supported}")]
    UnsupportedSchema { actual: u32, supported: u32 },

    /// A filesystem operation failed.
    #[error("filesystem operation {operation} failed for {path}: {source}")]
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A streamed file did not match the registry's declared integrity metadata.
    #[error(
        "integrity mismatch for {artifact}: expected {expected_size} bytes/{expected_sha256}, got {actual_size} bytes/{actual_sha256}"
    )]
    IntegrityMismatch {
        artifact: String,
        expected_size: u64,
        actual_size: u64,
        expected_sha256: String,
        actual_sha256: String,
    },

    /// The SDK could not coordinate concurrent access to one target.
    #[error("could not coordinate access to {target}")]
    Concurrency { target: PathBuf },

    /// SQLite returned an error.
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[source] rusqlite::Error),

    /// A migration file could not be applied or verified.
    #[error("database migration failed: {0}")]
    Migration(String),

    /// A blocking task failed to return its result.
    #[error("blocking SDK task failed: {0}")]
    TaskJoin(#[source] tokio::task::JoinError),

    /// A binary inventory entry is missing or no longer verified on disk.
    #[error("binary {package_id}/{component_name} is stale or missing at {path}")]
    StaleBinary {
        package_id: String,
        component_name: String,
        path: PathBuf,
    },

    /// A registry artifact is valid JSON but cannot be used by the requested operation.
    #[error("artifact {artifact} is incompatible: {reason}")]
    IncompatibleArtifact { artifact: String, reason: String },

    /// A download was cancelled before its current unverified file could be published.
    #[error("download was cancelled")]
    Cancelled,
}

impl SdkError {
    pub(crate) fn filesystem(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: io::Error,
    ) -> Self {
        Self::Filesystem {
            operation,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn invalid_metadata(artifact: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidMetadata {
            artifact: artifact.into(),
            reason: reason.into(),
        }
    }
}

impl From<reqwest::Error> for SdkError {
    fn from(error: reqwest::Error) -> Self {
        Self::RegistryTransport(error)
    }
}

impl From<rusqlite::Error> for SdkError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<tokio::task::JoinError> for SdkError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::TaskJoin(error)
    }
}
