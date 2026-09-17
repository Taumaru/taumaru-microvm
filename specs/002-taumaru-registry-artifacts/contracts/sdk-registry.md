# Public SDK Contract: Taumaru Registry Artifacts

This document defines the public boundary planned for the `taumaru-microvm` SDK. It keeps
registry transport, SQLite, and filesystem implementation details private while making list,
download, progress, resolution, and failure behavior explicit.

## Public exports

`crates/sdk/src/lib.rs` will re-export:

- `MicroVmSdk` and `SdkError`;
- local download types: `ArtifactKind`, `DownloadDisposition`, `DownloadProgress`,
  `DownloadPhase`, `DownloadedFile`, `DownloadedKernel`, `DownloadedBinary`,
  `DownloadedDistribution`, and `InstalledBinary`;
- the registry-provided types needed by callers, including `Architecture`, `BinaryFile`,
  `BinaryPackage`, `Distribution`, `DistributionImage`, `Kernel`, and their metadata types.

The registry types come from the exact official file vendored at
`crates/sdk/src/domain/registry.rs`; the SDK does not define a competing copy.

## Construction

```rust
pub struct MicroVmSdk { /* private fields */ }

impl MicroVmSdk {
    pub fn new(home: impl AsRef<std::path::Path>) -> Result<Self, SdkError>;

    pub fn with_registry_base_url(
        home: impl AsRef<std::path::Path>,
        registry_base_url: impl AsRef<str>,
    ) -> Result<Self, SdkError>;
}
```

`new` uses `https://artifacts.taumaru.com/v1/`. The alternate constructor exists for registry
mirrors and deterministic local fixture servers; it does not change the SDK-owned home or schema
migration behavior. Both constructors:

1. normalize and validate the caller-provided home path;
2. create the required `state`, artifact, cache, and temporary directories;
3. open `<home>/state/inventory.db`;
4. configure SQLite foreign keys and contention handling; and
5. apply all pending migrations before returning.

The SDK never reads `HOME`, `TAUMARU_HOME`, or another environment variable to choose its home.
It never creates a Tokio runtime for the caller.

## Listing operations

```rust
impl MicroVmSdk {
    pub async fn list_kernels(&self) -> Result<Vec<Kernel>, SdkError>;
    pub async fn list_binaries(&self) -> Result<Vec<BinaryPackage>, SdkError>;
    pub async fn list_distributions(&self) -> Result<Vec<Distribution>, SdkError>;
}
```

Each operation fetches the current manifest, validates the supported schema version, and returns
the matching collection from the official registry types. Binary package files and distribution
images remain nested in their parent values with their URL, filename, expected size, SHA-256, and
relevant metadata intact. Listing does not silently fall back to stale data when the registry is
unavailable.

## Progress and download result types

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Kernel,
    Binary,
    DistributionImage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadPhase {
    Downloading,
    Verifying,
    Completed,
    AdoptedExisting,
    SkippedExisting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    pub artifact_kind: ArtifactKind,
    pub artifact_id: String,
    pub member_name: Option<String>,
    pub bytes_received: u64,
    pub total_bytes: u64,
    pub aggregate_bytes_received: u64,
    pub aggregate_total_bytes: u64,
    pub phase: DownloadPhase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadDisposition {
    Downloaded,
    AdoptedExisting,
    SkippedExisting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadedFile {
    pub artifact_kind: ArtifactKind,
    pub artifact_id: String,
    pub member_name: Option<String>,
    pub absolute_path: std::path::PathBuf,
    pub relative_path: std::path::PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub disposition: DownloadDisposition,
}
```

For a multi-file package or image set, `aggregate_*` fields are monotonic across members while
the member-level fields reset for each member. A non-cached transfer emits progress as chunks are
received, emits a verification/completion event only after size and SHA-256 checks pass, and
never emits a successful completion for a failed file. A cache hit emits a terminal
`SkippedExisting` event without transferring bytes; an existing correct file missing its database
relationship emits `AdoptedExisting`.

## Download operations

```rust
impl MicroVmSdk {
    pub async fn download_kernel<F>(
        &self,
        kernel_id: &str,
        on_progress: F,
    ) -> Result<DownloadedKernel, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;

    pub async fn download_binary<F>(
        &self,
        binary_id: &str,
        on_progress: F,
    ) -> Result<DownloadedBinary, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;

    pub async fn download_distribution<F>(
        &self,
        distribution_id: &str,
        on_progress: F,
    ) -> Result<DownloadedDistribution, SdkError>
    where
        F: FnMut(DownloadProgress) + Send;
}
```

The result wrappers contain the selected official registry object and the downloaded file
results:

```rust
pub struct DownloadedKernel {
    pub kernel: Kernel,
    pub file: DownloadedFile,
}

pub struct DownloadedBinary {
    pub binary: BinaryPackage,
    pub files: Vec<DownloadedFile>,
}

pub struct DownloadedDistribution {
    pub distribution: Distribution,
    pub images: Vec<DownloadedFile>,
}
```

`download_binary` processes every `BinaryPackage.files` member. `download_distribution` processes
every `Distribution.images` member and records the distribution's ordered boot arguments and
kernel compatibility relationships; it does not implicitly download the default kernel.

Every physical member follows this decision contract:

| Disk state | Database state | Public behavior |
|------------|----------------|-----------------|
| Missing or size/SHA-256 mismatch | `downloads` and content relation may exist | Remove the invalid target and its physical/logical records; for a kernel, also remove its distribution-kernel relationships. Transfer to a temporary sibling, verify, atomically publish, and insert the verified rows. If replacement fails, no stale record remains. Return `Downloaded` only after success. |
| Correct size/SHA-256 | `downloads` or content relation absent/incomplete | Transfer zero bytes, create or repair the complete relationship, and return `AdoptedExisting`. |
| Correct size/SHA-256 | `downloads` and content relation exist with verified current metadata | Transfer zero bytes and return `SkippedExisting`. |

The SDK computes the actual digest and size rather than trusting file existence, HTTP headers, or
the database alone. A wrong existing file is never returned as ready. A valid file that was
placed outside the SDK through another process can be adopted only if it is at the deterministic
target and passes the current registry checks.

## Binary resolution

```rust
pub struct InstalledBinary {
    pub package_id: String,
    pub component_name: String,
    pub version: String,
    pub architecture: Architecture,
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub executable: bool,
}

impl MicroVmSdk {
    pub async fn resolve_binary(
        &self,
        package_id: &str,
        component_name: &str,
    ) -> Result<InstalledBinary, SdkError>;
}
```

Resolution queries the durable inventory, verifies that the referenced path still exists below
the SDK home, and recalculates its size and SHA-256 before returning it. It returns a stale or
integrity error if the file was deleted or replaced. Package, version, architecture, and component
identity are all part of the lookup; no directory scan or process-local cache is authoritative.

## Error contract

`SdkError` is a public, non-panicking error enum with variants covering at least:

- invalid or unavailable home path;
- invalid input, unsafe registry path, invalid URL, malformed digest, or missing metadata;
- registry transport, non-success response, manifest decoding, unsupported schema, or not-found
  artifact;
- filesystem, temporary-file, permission, atomic-publication, or task-join failure;
- SQLite connection, constraint, transaction, or migration failure;
- applied migration name/checksum drift or incompatible pre-existing schema;
- expected-versus-actual size or SHA-256 mismatch;
- stale or missing binary inventory record; and
- an incompatible artifact requested by a future consumer operation.

Every variant carries enough stable context for the caller to decide whether to retry, repair the
cache, choose another artifact, or report the failure. No variant writes diagnostics itself.

## Compatibility and side effects

- The current supported registry schema is version 1.
- `example_message` remains available and unchanged as the temporary bootstrap API.
- All public list/download/resolve methods are asynchronous except construction, which performs
  mandatory migrations synchronously before returning.
- The SDK owns directories and files below the supplied home but does not delete valid artifacts
  automatically.
- The SDK does not print, log, trace, terminate the process, install a global subscriber, or
  read home-selection environment variables.
- The CLI receives no new command in this feature; it may use the SDK in a later feature.
