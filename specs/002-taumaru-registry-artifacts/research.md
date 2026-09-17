# Research: Taumaru Registry Artifact Integration

**Date**: 2026-09-17

## Decision 1: Use the published manifest and official Rust definitions as the registry contract

**Decision**: Treat `https://artifacts.taumaru.com/v1/` as the default manifest endpoint and use
the registry-provided `types/registry.rs` as the source for manifest and artifact models. Vendor
the exact Rust file in the SDK so builds do not depend on a live registry request.

**Rationale**: The live manifest inspected during planning reports `schema_version: 1` and
separate `kernels`, `binaries`, `distributions`, and `types` collections. Kernel entries are
single downloadable files; binary entries contain a `files` array; distribution entries contain
an `images` array. Each physical member supplies a URL, filename, expected byte size, SHA-256
digest, MIME type, and modification timestamp. The `types` collection points to both TypeScript
and Rust definitions, with the Rust file at the path required by the feature.

The checked-in definition is an input contract, not a second hand-maintained domain model. The
registry source and its manifest digest should be recorded when the file is updated, and the
implementation should fail with a typed incompatibility error for an unsupported manifest schema.

**Alternatives considered**:

- Hand-writing SDK structs: rejected because it can drift from the registry and contradicts the
  request to use the published Rust typing.
- Downloading the type file at runtime: rejected because SDK builds and normal operation should
  not depend on a type-definition endpoint being reachable.
- Treating binary packages or distributions as a single physical file: rejected because the live
  contract contains plural nested members and each member has its own integrity metadata.

**Sources**:

- [Taumaru Artifacts Registry manifest](https://artifacts.taumaru.com/v1/)
- [Official Rust registry definitions](https://artifacts.taumaru.com/v1/types/registry.rs)

## Decision 2: Use an asynchronous HTTP client with streamed response chunks

**Decision**: Use one reusable asynchronous `reqwest::Client` in the registry adapter. Validate
HTTP status before reading a response, use response chunks for file downloads, and keep the
transport behind an internal source boundary so tests can use a local fixture server.

**Rationale**: The `reqwest` documentation describes `Client` as asynchronous and recommends
reusing it for multiple requests to benefit from connection pooling. `Response::error_for_status`
provides a direct non-success response check, and `Response::chunk` supports incremental body
consumption without requiring the optional stream/futures layer. The SDK needs incremental bytes
both for bounded memory use and for real-time progress callbacks.

The default TLS configuration must support the HTTPS registry. Use the minimal feature set needed
by the selected `reqwest` version; a Rustls-backed TLS feature is preferred for a self-contained
Linux SDK build. The client must not turn registry errors into output or retries hidden from the
caller.

**Alternatives considered**:

- `reqwest::blocking`: rejected because the feature explicitly requires asynchronous operations and
  blocking HTTP would complicate progress and concurrency.
- `Response::bytes()`: rejected because it buffers the complete artifact and cannot provide
  bounded-memory streaming.
- A new HTTP client abstraction exposed publicly: rejected because HTTP mechanics are an adapter
  concern and would unnecessarily couple SDK consumers to a transport library.

**Sources**:

- [reqwest crate documentation](https://docs.rs/reqwest/latest/reqwest/)
- [reqwest `Response` documentation](https://docs.rs/reqwest/latest/reqwest/struct.Response.html)

## Decision 3: Use Tokio for async I/O and bridge synchronous SQLite work explicitly

**Decision**: Make listing and download operations asynchronous. Use Tokio filesystem and I/O
traits for directory/file operations and use `tokio::task::spawn_blocking` for bounded
`rusqlite` queries and transactions. Keep the public constructor synchronous so migrations finish
before it returns and do not require the SDK to construct a runtime.

**Rationale**: Tokio's library guidance recommends enabling only the features a library needs.
The SDK needs `fs`, `io-util`, `rt`, and `sync`; test-only runtime macros can be enabled for tests.
Tokio documents that ordinary filesystem operations use blocking system calls behind its blocking
pool, and that `spawn_blocking` is the bridge for synchronous work that would otherwise block an
async worker. SQLite is inherently synchronous through `rusqlite`, so no connection or transaction
will be held across a network await. Each repository operation will execute its short database
unit in a blocking task and return owned values.

`MicroVmSdk::new` performs path normalization, directory creation, connection setup, and
migrations synchronously. That makes the migration timing explicit: no public SDK method can run
before the schema is ready. Async methods require the caller to execute them inside a Tokio
runtime, as does the selected async HTTP client.

**Alternatives considered**:

- Enabling `tokio = { features = ["full"] }`: rejected because a publishable library should not
  pull in unrelated runtime features.
- Running `rusqlite` directly inside async methods: rejected because database calls can block the
  executor.
- Keeping one SQLite connection behind an async mutex: rejected for the first implementation
  because it would serialize unrelated work and make connection/guard lifetimes span async
  boundaries. Short-lived blocking operations give a simpler ownership boundary while SQLite's
  busy timeout handles ordinary contention.

**Sources**:

- [Tokio crate feature guidance](https://docs.rs/tokio/latest/tokio/#feature-flags)
- [Tokio filesystem documentation](https://docs.rs/tokio/latest/tokio/fs/index.html)
- [Tokio `spawn_blocking` documentation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

## Decision 4: Use `rusqlite` with a migration ledger and normalized relational inventory

**Decision**: Store the inventory in `<sdk-home>/state/inventory.db` using `rusqlite`. Keep one
physical-file row per downloaded member in `downloads`, then relate it to logical tables for
kernels, binary package files, and distribution images. Record distribution/kernel compatibility
in a foreign-keyed many-to-many table.

**Rationale**: `rusqlite` provides the required `Connection`, prepared statements, transactions,
and `execute_batch` APIs. A local relational store matches the requested durable mapping and
allows uniqueness constraints to prevent a version, architecture, component, or path collision.
Separating physical file facts from logical metadata avoids duplicating paths and digests for
multi-file packages and makes the cache decision auditable.

The database schema uses foreign keys, explicit verification status, uniqueness constraints, and
indexes for stable IDs and paths. Nested registry metadata that has list semantics gets child
tables (boot arguments, filesystem features, capabilities, ELF libraries); scalar registry
metadata stays on the owning logical table. A distribution does not get a single download ID
because one registry distribution can contain multiple images; each image has its own download
row.

The migration runner compiles an ordered list of separate SQL files, creates `schema_migrations`
with `IF NOT EXISTS`, applies each missing migration inside a transaction, and records the
migration's checksum after commit. Reopening an existing SDK skips matching applied versions. A
modified applied migration or an existing object with an incompatible shape returns a typed
migration conflict; no migration drops user data or silently recreates an object.

**Alternatives considered**:

- One JSON inventory file: rejected because it makes relationships, atomic updates, and binary
  resolution less reliable.
- One denormalized table per artifact category: rejected because binary packages and
  distributions contain multiple physical files and would duplicate or obscure download state.
- A migration that always reruns `CREATE TABLE IF NOT EXISTS`: rejected because it would not detect
  migration drift or incomplete pre-existing schemas.

**Source**:

- [rusqlite crate documentation](https://docs.rs/rusqlite/latest/rusqlite/)

## Decision 5: Make the cache decision from both disk integrity and complete database relationships

**Decision**: Revalidate the deterministic target's actual size and SHA-256 before every download
decision, then inspect the required logical/content relationship and `downloads` row. Use three
outcomes: skip a correct file already represented completely, adopt a correct file missing its
database relation, or replace a missing/mismatched file through a verified temporary transfer.

**Rationale**: File existence alone cannot distinguish a complete download from a corrupt or
manually copied file. The requested behavior explicitly distinguishes these cases. Requiring both
physical verification and the correct content relationship makes restart behavior deterministic
and lets later SDK operations resolve binaries through the database rather than directory scans.

The hash helper reads bounded chunks and updates `Sha256` incrementally. The same helper is used
for cache inspection and after transfer, so the decision and the recorded verification facts use
the same algorithm. The `sha2` documentation exposes the incremental `Digest::update` and
`finalize` API needed for this flow.

For a missing or mismatched file, any existing database record and invalid target are removed only
after path ownership has been validated, and the new response is written to a unique sibling
temporary path. For an invalid downloaded kernel, the SDK also removes its logical kernel row and
distribution compatibility relationships before retrying. The SDK flushes and syncs the temporary
file, checks actual size and SHA-256, applies executable mode where required, atomically renames it
into place, and then commits the inventory insert. A failed transfer never becomes a final ready
file or leaves a stale inventory record; a kernel reference with no invalid local file may remain.

**Alternatives considered**:

- Existence-only cache hit: rejected because it accepts corruption and violates the requested
  database relationship rule.
- Hash-only validation: rejected because a complete but unexpectedly long response could still
  require an explicit size mismatch error and the registry publishes both facts.
- Writing directly to the final path: rejected because interruptions would leave a path that looks
  ready to later SDK operations.
- Recording a `downloading` row as the source of truth: rejected because a process crash could
  leave a partial file marked as installed; temporary files remain outside the ready inventory.

**Source**:

- [sha2 crate documentation](https://docs.rs/sha2/latest/sha2/)

## Decision 6: Preserve compatibility relationships independently from download completion

**Decision**: Allow `kernels` rows to exist as registry references with a nullable physical
download relation. A kernel counts as installed only when its `download_id` points to a verified
`downloads` row. Store every distribution's supported/default kernel relationship in
`distribution_kernels`, even when the referenced kernel has not yet been downloaded.

**Rationale**: The registry's distribution metadata references kernel IDs, while the client may
download a distribution image before selecting a kernel. Keeping logical compatibility separate
from physical acquisition preserves the complete registry relationship without inventing a fake
file. The cache rule remains strict because a logical kernel reference alone is not an installed
kernel.

**Alternatives considered**:

- Only insert compatibility rows after a kernel download: rejected because it loses registry
  compatibility information when a distribution is acquired first.
- Store kernel IDs as unconstrained text in the join table: rejected because foreign keys and
  uniqueness constraints provide stronger relationship integrity.
