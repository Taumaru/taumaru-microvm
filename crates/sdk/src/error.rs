use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by public SDK operations.
#[derive(Debug, Error)]
pub enum SdkError {
    /// The caller supplied a value that cannot be used for a MicroVM operation.
    #[error("invalid VM request field {field}: {reason}")]
    InvalidRequest { field: String, reason: String },

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

    /// A required local artifact is absent or cannot be used for creation.
    #[error("{kind} artifact {id} is not ready at {path}: {reason}")]
    ArtifactPrerequisite {
        kind: String,
        id: String,
        path: PathBuf,
        reason: String,
    },

    /// The requested root disk is smaller than a safe physical or registry floor.
    #[error(
        "disk size for image {image_id} is too small: requested {requested_size_bytes} bytes, minimum {minimum_size_bytes:?}, source {source_size_bytes} bytes"
    )]
    DiskSizeTooSmall {
        image_id: String,
        requested_size_bytes: u64,
        minimum_size_bytes: Option<u64>,
        source_size_bytes: u64,
    },

    /// A runtime component cannot be used with the selected package or host.
    #[error(
        "runtime component {component} from package {package_id} ({version}, {architecture}) is incompatible: {reason}"
    )]
    RuntimeIncompatible {
        component: String,
        package_id: String,
        version: String,
        architecture: String,
        reason: String,
    },

    /// A VM name or volume is already owned by another record or caller.
    #[error("volume {volume_path} for VM {vm_name} conflicts with {owner}: {reason}")]
    StorageConflict {
        vm_name: String,
        volume_path: PathBuf,
        owner: String,
        reason: String,
    },

    /// An operation conflicts with a durable VM lifecycle state.
    #[error("VM {name} is in state {state} and cannot perform {operation}")]
    LifecycleConflict {
        name: String,
        state: String,
        operation: String,
    },

    /// A repeated creation request conflicts with an immutable VM setting.
    #[error(
        "VM {name} configuration conflicts for {field}: existing {existing}, requested {requested}"
    )]
    ConfigurationConflict {
        name: String,
        field: String,
        existing: String,
        requested: String,
    },

    /// A network resource could not be inspected, created, or repaired.
    #[error("{mode} network operation {operation} failed for {resource}: {reason}")]
    Network {
        mode: String,
        operation: String,
        resource: String,
        reason: String,
    },

    /// A guest filesystem operation failed without exposing guest secrets.
    #[error("guest filesystem operation {operation} failed for {path}: {reason}")]
    GuestFilesystem {
        operation: String,
        path: PathBuf,
        reason: String,
    },

    /// A credential operation failed without exposing key material.
    #[error("credential operation {operation} failed for {path}: {reason}")]
    Credential {
        operation: String,
        path: PathBuf,
        reason: String,
    },

    /// A temporary runtime could not start, become ready, or stop cleanly.
    #[error("temporary runtime {component} failed: {reason}; stopped={stopped}")]
    TemporaryRuntime {
        component: String,
        reason: String,
        stopped: bool,
    },

    /// The primary operation failed and one or more owned cleanup actions also failed.
    #[error("operation failed: {primary}; cleanup failures: {failures:?}")]
    Cleanup {
        primary: String,
        failures: Vec<String>,
    },

    /// A local process command could not be executed safely.
    #[error("host command {program} failed: {reason}")]
    HostCommand { program: String, reason: String },

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
