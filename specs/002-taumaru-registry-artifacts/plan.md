# Implementation Plan: Taumaru Registry Artifact Integration

**Branch**: `002-taumaru-registry-artifacts` | **Date**: 2026-09-17 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-taumaru-registry-artifacts/spec.md`

**Note**: This template is filled in by the `$speckit-plan` command; its definition describes the execution workflow.

## Summary

Add a public, asynchronous artifact-management surface to the SDK for the Taumaru Artifacts
Registry. The SDK will fetch the schema-versioned registry manifest, expose typed listing
operations for kernels, binary packages, and distributions, and stream selected files into a
caller-provided host directory with progress callbacks, size validation, SHA-256 validation, and
atomic publication.

The implementation will separate registry transport, domain types, download coordination, and
SQLite persistence. A synchronous `MicroVmSdk::new` entrypoint will create the SDK-owned directory
layout and run pending migrations before returning a usable client. Public list and download
operations will be asynchronous and will use Tokio for network and file I/O. Synchronous
`rusqlite` work will be isolated in blocking tasks once an asynchronous operation is running.

The cache decision is explicit: a correct file with both its `downloads` row and its content row
is skipped; a correct file without the required database relationship is adopted into the
inventory without another transfer; a missing or mismatched file is replaced only after a new
transfer has passed both integrity checks. If that replacement fails, the invalid file and its
physical and logical inventory records are removed. For an invalid downloaded kernel, its
distribution-kernel relationships are removed as well. The normalized inventory keeps one
physical download record per file and separate logical kernel, binary-package, distribution,
image, and distribution-kernel relationships.

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for the project. The structure here is presented in advisory capacity to guide
  the iteration process.
-->

**Language/Version**: Rust edition 2024; repository toolchain is Rust 1.98.1. No new MSRV is
declared by this feature.

**Primary Dependencies**: `tokio` 1.x for async runtime, filesystem, task bridging, and test
support; `reqwest` 0.13.x for asynchronous HTTPS requests and streamed response chunks;
`rusqlite` 0.40.x with the bundled SQLite build for local persistence; `serde` and `serde_json`
for the registry's official Rust types and manifest decoding; `sha2` 0.11.x for incremental
SHA-256; `thiserror` 2.x for the public typed error surface. Production Tokio features remain
minimal; test-only macros are enabled only where needed.

**Storage**: SQLite at `<sdk-home>/state/inventory.db`, with the migration ledger and normalized
artifact inventory described in [data-model.md](./data-model.md). Downloaded files live below
the same caller-owned home in deterministic category paths; temporary files live below its `tmp`
directory.

**Testing**: `cargo fmt`, `cargo check`, `cargo clippy -D warnings`, and `cargo test`; async
contract tests use Tokio and a local fixture HTTP server built with `tokio::net::TcpListener`,
while persistence tests inspect temporary SQLite databases directly with `rusqlite`.

**Target Platform**: Linux host, matching the existing MicroVM project. The SDK boundary remains
independent of CLI environment lookup and terminal output.

**Project Type**: Publishable Rust SDK library; the CLI is not changed in this feature.

**Performance Goals**: Stream files in bounded buffers without loading an artifact into memory;
emit progress at each received response chunk; keep database transactions short and never hold a
SQLite transaction across network awaits; valid cache hits perform no second transfer.

**Constraints**: The SDK must not read `HOME` or `TAUMARU_HOME`, print, log, trace, panic, or
terminate the process. The caller supplies the base path. Only verified files become final targets
or verified inventory records. Migrations are separate SQL files, idempotent, checksum-tracked,
and run before the SDK constructor returns. Registry-controlled paths must remain under the SDK
home. Existing bootstrap API behavior must be preserved.

**Scale/Scope**: Multiple kernel versions and architectures, binary packages with multiple
components, and distributions with multiple images may coexist in one home. The first migration
owns the artifact inventory only; upload, deletion, resumable transfer, cancellation, registry
authentication, and new CLI commands are out of scope.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Phase 0 Gate

| Principle | Status | Design response |
|-----------|--------|-----------------|
| I. SDK-First Shared Core | PASS | All registry and download behavior stays in the SDK; no CLI lifecycle or registry engine is introduced. |
| II. Panic-Free, Silent SDK Boundary | PASS | Public operations return typed errors, use no output or logging, and keep callback notification separate from SDK diagnostics. |
| III. Explicit Local State and Lifecycle | PASS | The caller-supplied home owns durable SQLite inventory, verified artifact paths, cache state, and restart-safe binary resolution. |
| IV. Closed for Modification, Open for Extension | PASS | Registry and persistence are replaceable ports/adapters; official types, migrations, and download coordination have separate ownership. |
| V. Calm, Accessible CLI and Intentional Documentation | PASS | No CLI surface changes are required; public SDK behavior and failure contracts will receive Rustdoc and English tests/docs. |

No constitution violations require a complexity exception.

### Post-Phase 1 Gate

| Check | Status | Evidence |
|-------|--------|----------|
| Dependency direction | PASS | `crates/sdk` owns the registry feature; `crates/cli` remains a consumer and is untouched. |
| Local-state ownership | PASS | `MicroVmSdk::new` receives the path, creates `<home>/state`, runs migrations, and keeps all managed files below the path. |
| Persistence integrity | PASS | The design uses normalized foreign keys, uniqueness constraints, verification states, short transactions, and a migration checksum ledger. |
| Async boundary | PASS | HTTP and file streaming use Tokio-compatible async APIs; `rusqlite` calls are isolated from async worker threads. |
| Download safety | PASS | Cache checks require physical size/hash plus the complete content relationship; writes publish only after verification through an atomic rename. |
| Public contract | PASS | Listing, download, progress, resolution, and typed error contracts are defined in [contracts/sdk-registry.md](./contracts/sdk-registry.md). |

No post-design gate is violated.

## Project Structure

### Documentation (this feature)

```text
specs/002-taumaru-registry-artifacts/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/
│   └── sdk-registry.md  # Public SDK contract
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the placeholder tree below with the concrete layout
  for this feature. Delete unused options and expand the chosen structure with
  real paths (e.g., apps/admin, packages/something). The delivered plan must
  not include Option labels.
-->

```text
Cargo.toml                         # add shared dependency versions
Cargo.lock                         # resolve the SDK dependency graph
crates/sdk/
├── Cargo.toml                     # SDK runtime dependencies
├── migrations/
│   └── 0001_artifact_inventory.sql
├── src/
│   ├── lib.rs                     # public facade and deliberate re-exports
│   ├── error.rs                   # public typed errors
│   ├── manager.rs                 # SDK construction and artifact orchestration
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── artifact.rs            # local artifact, progress, and result types
│   │   └── registry.rs            # exact official registry.rs definition
│   ├── ports/
│   │   ├── mod.rs
│   │   ├── artifacts.rs           # replaceable registry/source boundary
│   │   └── repository.rs          # replaceable inventory boundary
│   └── adapters/
│       ├── mod.rs
│       ├── persistence/
│       │   ├── mod.rs
│       │   ├── migrations.rs      # migration discovery and ledger
│       │   └── sqlite.rs          # rusqlite repository implementation
│       └── registry/
│           ├── mod.rs
│           └── taumaru.rs          # canonical manifest and artifact transport
└── tests/
    ├── public_api.rs              # public re-exports and Rustdoc examples
    ├── registry_contract.rs       # manifest/listing and registry failures
    ├── download_flow.rs           # streaming, cache, progress, integrity
    └── sqlite_persistence.rs      # migrations, relationships, restart lookup
```

**Structure Decision**: Extend the existing two-crate workspace only inside `crates/sdk`.
`domain/registry.rs` contains the registry-provided Rust definitions as data contracts without
network or filesystem code. `adapters/registry/taumaru.rs` owns HTTP and manifest validation;
`adapters/persistence/sqlite.rs` owns all SQL; `manager.rs` coordinates the two boundaries and
the cache state machine. The CLI remains unchanged because no presentation behavior is required
to satisfy this feature.

## Design Decisions

### SDK entrypoint and asynchronous boundary

`MicroVmSdk::new(home)` is synchronous and performs the one-time setup that must happen before
the client can be used: normalize the supplied path, create the required directories, open the
SQLite database, configure foreign-key enforcement and a bounded busy timeout, and run all pending
migrations. It never consults environment variables and never constructs a Tokio runtime.

Listing and download methods are `async`. The registry adapter keeps one reusable asynchronous
HTTP client. File transfers use bounded response chunks, `tokio::fs::File`, and incremental hashing.
Each SQLite operation opens or borrows a connection only inside `tokio::task::spawn_blocking`,
performs its short transaction, and returns owned data before the future continues. No database
transaction remains open while the network is being awaited.

### Official registry types

The exact file advertised by the registry at `types/registry.rs` is copied into
`crates/sdk/src/domain/registry.rs` during implementation. It is not retyped or duplicated. The
manifest parser uses its `TaumaruRegistry`, `Kernel`, `BinaryPackage`, `BinaryFile`, `Distribution`,
and nested types; `lib.rs` re-exports only the stable types needed by the public SDK contract.

### Download and cache state machine

Every physical file has a deterministic artifact key and target path. The coordinator resolves the
latest metadata from the manifest, validates the expected digest/path, reads the local target, and
checks the database relationship before selecting one of these paths:

| Physical target | Complete content relationship | Action |
|-----------------|-------------------------------|--------|
| Missing or wrong size/SHA-256 | Present or absent | Remove the invalid target and its existing physical/logical records; for a kernel, also remove its distribution-kernel relationships. Stream to a unique temporary file, verify, atomically rename, then insert the verified records. If replacement fails, no stale record remains. |
| Correct size/SHA-256 | Absent or incomplete | Do not transfer; adopt the existing file by inserting/upserting `downloads` and the related content row in one transaction. |
| Correct size/SHA-256 | Present and verified | Do not transfer; return the recorded target as `SkippedExisting`. |

For a binary package or distribution, the coordinator applies this decision independently to
each member, but only reports the aggregate result as complete when every member is verified. A
valid partial member can stay cached and be reused on a later retry; a failed member has no stale
inventory record. The target file is never the download destination directly; a temporary sibling
is flushed, synced, verified, and atomically renamed only on success. Executable mode is applied
before the file is published or before its verified record is committed.

### Database and migration strategy

The first migration creates the migration ledger, physical download table, logical artifact tables,
relationship tables, nested metadata tables, indexes, and foreign-key constraints. The migration
runner has a compile-time ordered list of separate SQL files, computes each file's checksum, and
records `(version, name, checksum, applied_at)` after a successful transaction. An already applied
version is skipped only when its name and checksum still match. Re-running the constructor is safe:
`CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` protect first boot, while the ledger
prevents duplicate application. A pre-existing object with an incompatible shape returns a typed
migration conflict instead of being dropped or silently accepted.

The normalized shape keeps the physical file facts in `downloads` and the logical facts in
`kernels`, `binary_packages`/`binary_files`, `distributions`/`distribution_images`, and
`distribution_kernels`. The distribution relation uses one download row per image because the
registry's `images` collection is plural; a direct distribution-to-download column would lose
that relationship. A kernel row may exist as a registry reference with a null download relation
so distribution compatibility can be recorded before that kernel is downloaded. It is considered
installed only when its download relation is present and verified.

### Verification and error handling

The integrity helper reads a file incrementally and returns actual byte length plus lowercase
SHA-256. It is used both for cache inspection and post-transfer verification. Registry metadata is
validated before writes: schema version, IDs, required URL/path components, digest shape, and
non-negative sizes. `SdkError` maps network status, decode, path, filesystem, migration, SQLite,
integrity, stale-record, and not-found failures without leaking presentation policy into the SDK.

## Implementation Sequence

1. Add the dependency declarations and preserve the existing bootstrap API.
2. Vendor the exact official registry type file and add domain/progress/result types plus typed
   errors.
3. Add the initial SQL migration and migration runner; make `MicroVmSdk::new` create directories
   and run it before returning.
4. Implement the registry adapter and internal registry/repository ports with fixture-friendly
   boundaries.
5. Implement incremental hash/cache inspection, temporary-file streaming, progress callbacks,
   atomic publication, executable mode handling, and normalized persistence.
6. Add public re-exports and binary resolution; document all public items with Rustdoc.
7. Add contract, cache-decision, migration, relationship, restart, and failure-path tests.
8. Run the full workspace quality gates and use [quickstart.md](./quickstart.md) as the final
   acceptance checklist.
