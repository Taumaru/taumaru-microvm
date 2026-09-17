# Feature Specification: Taumaru Registry Artifact Integration

**Feature Branch**: `002-taumaru-registry-artifacts`

**Created**: 2026-09-17

**Status**: Draft

**Input**: User description: "Add Taumaru Artifacts Registry integration to the SDK so clients can list and download kernels, binary packages, and distribution images with live progress, integrity checks, home-directory organization, and persistent binary metadata."

## Clarifications

### Session 2026-09-17

- Q: When a distribution is downloaded before any of its compatible kernels, should the SDK immediately persist those compatibility relationships from the registry references even when the kernels are not downloaded locally? → A: Yes. Persist the relationships immediately; kernel rows may remain registry references and are considered installed only after their own verified download.
- Q: If a file already recorded in the database is missing or has an incorrect size or SHA-256 digest and the replacement download fails, should the SDK retain the record as stale or remove it until a verified download succeeds? → A: Remove the logical and physical records until a verified download succeeds.
- Q: If a previously downloaded kernel becomes invalid but is still referenced by distribution compatibility relationships, should the SDK remove only its download relation or also remove the kernel reference and every compatibility relationship? → A: Remove everything associated with the invalid downloaded kernel: the file, physical download record, logical kernel row, and all distribution compatibility relationships. A registry-only kernel reference without an invalid local file may remain.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Discover Available Artifacts (Priority: P1)

As an application developer, I want to list the kernels, runtime binary packages, and
distribution images published by Taumaru so that I can present valid choices to the caller
before requesting a download.

**Why this priority**: Discovery is the entry point for every artifact workflow and prevents
callers from hard-coding registry paths, versions, or checksums.

**Independent Test**: Point the SDK's registry boundary at a fixture containing a valid schema
version 1 manifest, call each public listing operation, and verify that all three collections
and their nested downloadable files are returned without direct registry parsing by the caller.

**Acceptance Scenarios**:

1. **Given** a valid registry manifest containing kernels, binary packages, and distributions,
   **When** the caller invokes the corresponding listing operation, **Then** the SDK returns all
   published entries with their stable identifiers, versions, architectures, and descriptive
   metadata.
2. **Given** a binary package with multiple files or a distribution with multiple images,
   **When** the caller lists that collection, **Then** each downloadable member is exposed with
   its source URL, filename, expected byte size, SHA-256 digest, and relevant execution or
   filesystem metadata.
3. **Given** the registry is unavailable, malformed, or advertises an unsupported schema,
   **When** a listing operation is called, **Then** the SDK returns a typed error that identifies
   the failure and produces no unsolicited process output or panic.

---

### User Story 2 - Download Verified Artifacts (Priority: P1)

As an application developer, I want to download a selected kernel, binary package, or
distribution image set into an SDK-managed home directory so that the artifact is ready for
later MicroVM operations and can be trusted to match the registry.

**Why this priority**: A listing has no operational value unless the selected artifact can be
acquired safely and reused by the host-local MicroVM manager.

**Independent Test**: Use a local fixture registry and temporary SDK home, download one item from
each collection, observe progress events, and verify the resulting files against the fixture's
declared size and digest.

**Acceptance Scenarios**:

1. **Given** a listed kernel and a writable SDK home, **When** the caller requests its download,
   **Then** the SDK creates the required managed directories, streams the file into the home,
   reports transfer progress while bytes are received, and returns a verified local path.
2. **Given** a listed binary package containing multiple files, **When** the caller requests its
   download, **Then** every package file is acquired and verified, registry-declared executable
   permissions are preserved, and the package is usable only after all of its files are ready.
3. **Given** a listed distribution containing multiple images, **When** the caller requests its
   download, **Then** every image in that distribution is acquired and verified and the result
   identifies the local path of each image.
4. **Given** a target file already exists and its bytes match both the expected size and
   SHA-256 digest, **When** the caller requests the same download again, **Then** the SDK reuses
   the existing file without transferring it again and reports that the cached artifact was
   reused.
5. **Given** a transfer is interrupted or the downloaded bytes fail the expected size or
   SHA-256 check, **When** the download completes or fails, **Then** the SDK returns a typed
   integrity or acquisition error, does not expose the invalid file as ready, and does not add an
   unverified binary record to the local inventory.

---

### User Story 3 - Resolve Installed Binaries Reliably (Priority: P2)

As a MicroVM lifecycle operation, I want to resolve a verified runtime binary from the SDK's
local inventory so that I can use the correct executable without scanning arbitrary host paths
or relying on process-local memory.

**Why this priority**: Firecracker-related operations need stable, restart-safe mappings from a
registry artifact identity to the exact executable installed on the host.

**Independent Test**: Download a binary package, close and recreate the SDK client with the same
home path, and verify that the binary resolver returns the recorded executable path and metadata.

**Acceptance Scenarios**:

1. **Given** a binary file has completed size and digest verification, **When** a later SDK
   operation resolves it by package and component identity, **Then** the resolver returns the
   recorded local path and the metadata needed to use that file.
2. **Given** the SDK is recreated after the original process exits, **When** a previously
   installed binary is resolved, **Then** the result comes from durable local inventory and does
   not depend on the previous process instance.
3. **Given** a binary path was deleted or replaced outside the SDK, **When** a caller resolves
   that binary, **Then** the SDK detects that the stored artifact is no longer verified and
   returns a typed stale-or-integrity error instead of returning an unsafe path.
4. **Given** two versions or architectures of a package are installed, **When** either one is
   resolved by its stable identity, **Then** the SDK returns the matching file without confusing
   it with the other installation.

### Edge Cases

- The registry endpoint times out, returns a non-success response, returns invalid data, or
  changes to an unsupported schema version; listing and download operations return typed errors
  with useful context.
- The manifest contains duplicate identifiers, missing required metadata, an invalid digest,
  an invalid URL, or a path or filename that would escape the SDK home; the SDK rejects the
  affected entry before writing it.
- The registry declares a size that differs from the response body, including a truncated or
  unexpectedly longer transfer; the target is not marked ready.
- The SDK home cannot be created or written because of missing permissions, a conflicting file,
  or insufficient storage; the failure is returned without partial state being presented as
  usable.
- A valid-looking file already exists at the deterministic target path but has the wrong size or
  digest; the SDK does not reuse it and replaces it only after a new transfer has been verified.
- One member of a multi-file binary package or distribution fails; successfully verified member
  files may remain reusable in the cache, but the package or distribution is not reported as
  fully ready until every member is verified.
- Multiple callers request the same target concurrently; the SDK prevents competing writes
  from exposing a partial file and makes the final inventory state deterministic.
- A caller requests an artifact for a different architecture; the SDK preserves the registry's
  architecture metadata, allows deliberate prefetching, and leaves host-compatibility decisions
  to the operation that will use the artifact.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The SDK MUST use the Taumaru Artifacts Registry at
  `https://artifacts.taumaru.com/v1/` as the default source for the manifest and its published
  artifact metadata.
- **FR-002**: The integration MUST treat the registry's published type definition referenced by
  `types/registry.rs` as the authoritative schema source, include that official definition in
  the SDK integration without manually recreating a competing model, and preserve its source
  provenance for future updates.
- **FR-003**: The SDK MUST expose public operations for listing kernels, binary packages, and
  distributions through the SDK boundary; callers MUST NOT need to assemble registry requests or
  parse the manifest themselves.
- **FR-004**: Listing results MUST preserve the registry identifiers, names, versions,
  architectures, URLs, filenames, expected sizes, SHA-256 digests, and relevant nested metadata
  for each collection and downloadable member.
- **FR-005**: Registry fetch, decoding, schema incompatibility, invalid metadata, and unavailable
  artifact conditions MUST be represented as typed SDK errors with enough context for the caller
  to choose a response.
- **FR-006**: SDK construction MUST receive the host-local base directory as an explicit path
  from the caller; the SDK MUST own the layout below that path and MUST NOT discover or override
  it through environment variables.
- **FR-007**: The SDK MUST expose public download operations for one kernel, one binary package,
  and one distribution. A binary package download MUST cover every file in its `files` collection,
  and a distribution download MUST cover every image in its `images` collection. A distribution
  download MUST also persist each registry-declared supported/default kernel relationship
  immediately, even when the referenced kernel has not been downloaded locally; such a kernel
  remains a registry reference until its own file passes verification.
- **FR-008**: Downloaded files MUST be placed at deterministic paths below the caller-provided
  SDK home, and registry-controlled path components MUST be validated so no artifact can write
  outside that home.
- **FR-009**: Each download operation MUST accept a progress callback that can observe transfer
  progress while bytes are received. Progress events MUST identify the artifact or member,
  report bytes received and expected total bytes, use monotonically increasing byte counts, and
  provide a final event at the expected size for every non-cached file.
- **FR-010**: Before transferring an artifact, the SDK MUST check its deterministic local target.
  A file is reusable only when its current size and SHA-256 digest match the registry metadata;
  a valid cached file MUST NOT be transferred again. If a previously recorded target is missing or
  mismatched and replacement fails, the SDK MUST remove the invalid target and its corresponding
  physical and logical inventory records, leaving no stale record to be mistaken for an installed
  artifact. For an invalid downloaded kernel, removal MUST also delete the kernel's
  distribution-compatibility relationships so no future operation can resolve the invalid
  artifact through persisted metadata.
- **FR-011**: The SDK MUST write downloads through temporary or otherwise atomic state so an
  interrupted transfer cannot appear at the final path as a ready artifact, and concurrent
  requests for the same target MUST be coordinated.
- **FR-012**: Before a downloaded file is reported as ready, the SDK MUST calculate its SHA-256
  digest and compare it with the registry digest, compare its actual size with the registry size,
  and return a typed integrity error on any mismatch.
- **FR-013**: For binary files, the SDK MUST preserve the registry's executable indication and
  declared mode when the host filesystem supports those permissions. A package MUST NOT be
  reported as fully installed when any constituent file is unverified.
- **FR-014**: The SDK MUST persist every verified binary file in the host-local SQLite inventory,
  including its stable package and component identities, version, architecture, registry source,
  local path, expected and verified size, expected and verified SHA-256 digest, executable
  status, mode when present, and verification state.
- **FR-015**: SDK operations that need a runtime binary MUST be able to resolve it from the local
  inventory by stable identity, and MUST revalidate that the recorded path still exists with the
  recorded size and digest before returning it for use.
- **FR-016**: Local paths and inventory records MUST distinguish multiple package versions,
  architectures, and components so installing or resolving one artifact cannot silently select
  another.
- **FR-017**: Public SDK operations MUST return expected filesystem, network, decoding, storage,
  concurrency, and integrity failures as typed results without panicking, terminating the host,
  writing to stdout or stderr, emitting logs or tracing events, or relying on hidden global state.
- **FR-018**: The feature MUST include contract and failure-path tests covering manifest decoding,
  all three listing operations, progress reporting, cache reuse, size and digest validation,
  multi-file downloads, durable binary lookup, architecture/version separation, and the absence
  of unsolicited SDK output or panic.

### Key Entities *(include if feature involves data)*

- **Registry Manifest**: The published registry document containing schema metadata and the
  kernels, binary packages, distributions, and type-definition references available to clients.
- **Kernel Artifact**: A single downloadable kernel with identity, architecture, source location,
  expected size, digest, format, and optional executable metadata.
- **Binary Package**: A versioned, architecture-specific runtime package containing one or more
  named executable files such as a runtime and its helper.
- **Distribution**: A distribution release with boot and compatibility metadata plus one or more
  downloadable filesystem images.
- **Download Progress Event**: A callback payload describing the member being transferred, bytes
  received, expected total bytes, and whether the event represents completion or cache reuse.
- **Cached Artifact**: A locally managed file whose path and contents can be matched to registry
  metadata and safely reused without another transfer.
- **Binary Inventory Record**: Durable metadata linking a verified binary package component to
  its local executable path and integrity information for later SDK operations.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100% of valid-manifest contract tests, callers can list kernels, binary
  packages, and distributions and receive every published entry with its nested downloadable
  members and integrity metadata.
- **SC-002**: In 100% of non-cached download tests, the progress callback receives transfer
  events with monotonically increasing byte counts and a final count equal to the registry's
  declared file size.
- **SC-003**: In 100% of successful download tests, every returned file matches both the
  registry-declared byte size and SHA-256 digest before it is reported as ready.
- **SC-004**: In 100% of repeated-download tests for unchanged files, the SDK reuses the valid
  local file, performs no second transfer, and returns the same verified local path.
- **SC-005**: In 100% of interrupted, truncated, permission, malformed-metadata, and digest-
  mismatch tests, the SDK returns a typed error and exposes neither an invalid ready file nor an
  unverified binary inventory record.
- **SC-006**: In 100% of restart tests, a verified binary installed before the client is closed
  can be resolved afterward by its stable package and component identity from the same SDK home.
- **SC-007**: In 100% of multi-version, multi-architecture, and multi-component tests, each
  requested identity resolves to its own verified file without path or inventory collisions.
- **SC-008**: An application can complete the primary workflow—list a choice, download it with
  progress, and obtain a verified local result—using only public SDK operations and without
  implementing registry parsing, checksum validation, or local binary mapping itself.

## Assumptions

- The registry is publicly readable over HTTPS and does not require Taumaru authentication for
  manifest or artifact downloads in this feature.
- The initial supported registry schema is version 1. A future incompatible schema is surfaced
  as a typed error rather than being silently interpreted.
- The registry's `kernels` entries represent one downloadable file, `binaries` entries represent
  packages whose `files` members are all required for a complete package, and `distributions`
  entries represent image sets whose `images` members are all required for a complete download.
  A distribution download does not implicitly download its referenced default kernel; it does
  persist the registry-declared compatibility and default-kernel relationships, while kernels are
  selected and downloaded through the kernel operation.
- The caller, including the CLI dependency-wiring layer, resolves the base path and passes it to
  the SDK. The SDK owns all subdirectories, cache files, temporary files, locks, and inventory
  data below that path.
- The host-local SQLite inventory is the durable source for verified binary mappings and survives
  SDK client recreation. The feature does not add distributed state or registry write operations.
- The official registry Rust definition is copied from the registry-provided file into the SDK
  integration during implementation; its published content and digest are the source of truth,
  and no hand-maintained duplicate types are required.
- Valid member files from a partially failed package or distribution may remain in the managed
  cache for reuse, but the aggregate download result remains unsuccessful until all members pass
  verification.
- Downloaded artifacts are retained until an explicit future cleanup capability removes them.
  Automatic eviction, upload, resume, cancellation, authentication, and new CLI subcommands are
  outside this feature.
- Architecture metadata is preserved for deliberate prefetching. The SDK operation that uses an
  artifact is responsible for rejecting an incompatible artifact for the current host or VM.
