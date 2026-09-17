# Quickstart: CLI Artifact Download

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Network access to the Taumaru Artifacts Registry at `https://artifacts.taumaru.com/v1/`.
- A writable Taumaru home directory.
- An interactive terminal for selector mode, or explicit IDs for automation mode.

The SDK owns all directories, SQLite migrations, artifact files, integrity checks, and cache
decisions below the resolved home. The CLI only resolves that home and presents the operation.

## Interactive download

Run:

```text
cargo run -p taumaru-microvm-cli -- download
```

The command loads the current registry collections, then:

1. select one or more distributions with the keyboard;
2. choose one compatible kernel for each distribution;
3. review the runtime package, distribution/kernel pairs, image count, and estimated size;
4. confirm `Download`;
5. observe runtime, kernel, and distribution progress until the verified result.

Press Escape during selection or review to cancel before transfer. A cancellation does not alter
the artifact inventory.

Press `Ctrl-C` during a transfer to stop the current operation. The SDK removes the partial
temporary artifact, keeps groups already verified, does not start later groups, and the CLI exits
with code `130` after summarizing completed, failed, and cancelled groups.

## Explicit automation

Provide one repeatable distribution and one mapping per distribution:

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- download \
  --non-interactive \
  --distribution <distribution-id> \
  --kernel <distribution-id>=<kernel-id>
```

For more than one distribution, repeat both options. The command emits no selectors and returns
exit code `0` only after every required artifact has been verified. Use the same IDs shown by the
interactive catalog or the registry manifest.

## Expected output shape

The exact progress refresh is terminal-dependent, but the semantic content remains stable:

```text
Download plan
  Runtime: Firecracker 1.14.1 (x86_64), 2 files
  alpine-3.20 -> linux-6.8-x86_64 (default), 1 image
  Estimated: 128 MiB

Downloading runtime/firecracker  8.0 MiB / 8.0 MiB  Downloaded
Downloading kernel/linux-6.8-x86_64  16.0 MiB / 16.0 MiB  Already available
Verifying distro/alpine-3.20/image  104 MiB / 104 MiB  Downloaded

Download complete: 3 groups verified, 128 MiB available
```

For a valid cache hit, the terminal label is `Already available`; for a valid file that was not
yet recorded by the SDK inventory, it is `Adopted`. These states are not inferred by the CLI.

## Validation and failure checks

Before confirmation, the command must reject:

- an empty distribution selection;
- an unknown distribution or kernel ID;
- a kernel not published as compatible with its distribution;
- a duplicate or incomplete explicit mapping;
- a distribution or kernel for another architecture;
- a runtime package without both `firecracker` and `firectl`;
- a registry response that the SDK reports as unavailable, malformed, or unsupported.

After confirmation, a runtime failure stops dependent downloads. A kernel failure skips the
images of its associated distribution while unrelated groups continue. Any failed or skipped
group produces a nonzero result, names the group, retains verified work, and recommends retrying
after the underlying issue is corrected. A transfer cancellation returns `130`, publishes no
partial artifact, and identifies completed, failed, and cancelled groups.

## Verification commands for implementation

From the repository root, run the focused CLI checks first:

```text
cargo fmt --all -- --check
cargo test -p taumaru-microvm-cli --all-targets --all-features
```

Then run the workspace gates:

```text
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The CLI tests must use deterministic catalog/download fakes for planning, ordering, output, and
failure behavior. They must not require the public registry or a real interactive terminal.
