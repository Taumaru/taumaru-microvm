# Quickstart: Taumaru Registry Artifact Integration

This guide validates the SDK feature after implementation. It uses small local registry fixtures
for deterministic tests and avoids downloading the large production root filesystem images.

## Prerequisites

- Linux host with the repository toolchain available.
- A Tokio runtime for asynchronous SDK calls.
- `cargo` and, optionally, `sqlite3` for inspecting a test database.
- No Firecracker process, KVM device, CLI configuration, or Taumaru authentication is required.

## Run the focused validation

From the repository root:

```bash
cargo fmt --all -- --check
cargo test -p taumaru-microvm --test registry_contract
cargo test -p taumaru-microvm --test download_flow
cargo test -p taumaru-microvm --test failure_paths
cargo test -p taumaru-microvm --test sqlite_persistence
cargo test -p taumaru-microvm --test public_api
```

The focused tests should prove the following:

1. A schema-version 1 fixture is decoded through the official registry types and all kernel,
   binary-package, and distribution listings preserve nested member metadata.
2. The SDK constructor creates the requested home layout, creates the SQLite state database, and
   runs the migration ledger before returning. Constructing a second client for the same home
   does not duplicate tables or destroy rows.
3. A non-cached file streams through bounded chunks, reports monotonic progress, passes both
   expected-size and SHA-256 checks, and is atomically published.
4. A correct file with no complete database relationship is adopted without a second transfer.
5. A correct file with both `downloads` and its content row is skipped without a second transfer.
6. A missing or wrong-digest file is removed/replaced, and an existing database record is updated
   only after the replacement passes verification.
7. Binary packages persist one physical `downloads` row per component, preserve executable mode,
   and resolve correctly after the SDK client is recreated.
8. Distribution images and ordered kernel compatibility rows are persisted without confusing a
   distribution image with its referenced kernel.
9. Interrupted, truncated, malformed, unsafe-path, permission, migration-drift, database, and
   integrity failures return typed errors without SDK output or panic; invalid kernel cleanup
   removes its physical, logical, and compatibility rows.

## Run the complete quality gates

```bash
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

All checks must pass without changing the existing CLI bootstrap behavior. The SDK tests should
use a local fixture server and temporary homes; they must not make production artifact downloads
part of the default test suite.

## Optional live manifest smoke check

The production manifest can be inspected without downloading an artifact:

```bash
curl --fail --location https://artifacts.taumaru.com/v1/
```

The response should advertise schema version 1 and the `kernels`, `binaries`, `distributions`,
and `types` collections. The official Rust type source is
[types/registry.rs](https://artifacts.taumaru.com/v1/types/registry.rs).

## Database review checklist

For a test home retained by a focused test or a small manual harness, inspect the state database:

```bash
sqlite3 <sdk-home>/state/inventory.db '.tables'
sqlite3 <sdk-home>/state/inventory.db 'PRAGMA foreign_key_check;'
sqlite3 <sdk-home>/state/inventory.db 'SELECT artifact_key, artifact_type, relative_path, absolute_path, verification_status FROM downloads;'
```

Expected results:

- The migration ledger and normalized inventory tables exist exactly once.
- `PRAGMA foreign_key_check` returns no rows.
- Every verified binary has a `downloads` row and a related `binary_files` row.
- Absolute paths remain below `<sdk-home>` and relative paths contain no leading separator.
- Re-running the same download does not add duplicate rows or transfer the file again.
- A failed replacement leaves no invalid final file, temporary part file, or unverified physical
  inventory row; a previously downloaded invalid kernel also loses its compatibility links.
