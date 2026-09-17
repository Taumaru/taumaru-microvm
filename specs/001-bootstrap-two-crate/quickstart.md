# Quickstart: Bootstrap Two-Crate Workspace

This guide validates the project foundation after implementation. It does not require
Firecracker, KVM, a registry connection, a local database, or an existing MicroVM.

## Prerequisites

- Linux host
- Rust toolchain with Rust 2024 edition support
- Network access only if dependencies are not already cached

## 1. Verify workspace members

From the repository root, run:

```bash
cargo metadata --no-deps --format-version 1
```

Expected result: the metadata contains two packages named `taumaru-microvm` and
`taumaru-microvm-cli`.

## 2. Run workspace quality checks

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Expected result: every command exits successfully without source changes.

## 3. Validate the SDK contract

```bash
cargo test -p taumaru-microvm --test public_api
```

Expected result: the external-consumer test imports `example_message`, receives
`taumaru-microvm SDK is ready`, and observes no unsolicited SDK output.

See [SDK Contract](contracts/sdk.md) and [Data Model](data-model.md) for the exact boundary.

## 4. Validate the CLI contract

```bash
cargo run -p taumaru-microvm-cli -- --help
cargo run -p taumaru-microvm-cli -- --version
cargo run -p taumaru-microvm-cli --
cargo run -p taumaru-microvm-cli -- unknown
```

Expected results:

- `--help` exits successfully and shows usage for `microvm`.
- `--version` exits successfully and shows the CLI version.
- No arguments show concise help guidance without requiring runtime resources.
- An unknown command shows a clear parse error and exits unsuccessfully without a panic.

The executable-level checks are also covered by:

```bash
cargo test -p taumaru-microvm-cli --test command_surface
```

See [CLI Contract](contracts/cli.md) for the exact command expectations.

## 5. Review dependency boundaries

```bash
cargo tree -p taumaru-microvm-cli
cargo package -p taumaru-microvm --allow-dirty --no-verify
```

Expected result: the CLI depends on the SDK and Clap; no logging, async, table, progress, or
JSON dependency is added without a tested baseline use. The SDK package can be packaged for
future publication. Actual publication and binary release distribution are separate work.
