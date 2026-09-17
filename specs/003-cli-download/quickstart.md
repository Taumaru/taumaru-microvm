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
4. confirm `Start download`;
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
·  Checking artifact registry
✓  Registry ready · 3 distributions · 4 kernels · 1 runtime packages

◆ Download plan
────────────────────────────────────────────────

Runtime
  •  Firecracker 1.14.1 (firecracker-1.14.1-x86_64) · 2 files · x86_64
     8.0 MiB expected

Targets
  •  alpine-3.20 (Alpine 3.20)
     ↳  linux-6.8-x86_64 (Linux 6.8) · default · 1 image · 120 MiB

Transfer
  128 MiB · 3 planned groups · 1 images

Review the plan above. The download starts after confirmation.

↓  runtime/firecracker  Downloading  8.0 MiB / 8.0 MiB  ·  plan 8.0 MiB / 128 MiB
✓  runtime/firecracker  Downloaded  8.0 MiB / 8.0 MiB  ·  plan 8.0 MiB / 128 MiB
✓  kernel/linux-6.8-x86_64  Already available  16.0 MiB / 16.0 MiB  ·  plan 24.0 MiB / 128 MiB
↓  distribution/alpine-3.20/image  Verifying  104 MiB / 104 MiB  ·  plan 128 MiB / 128 MiB

✓ Download complete
────────────────────────────────────────────────

  3/3 groups ready · 4 files verified
  128 MiB available of 128 MiB planned · 128 MiB verified

Artifacts
  ✓  runtime/firecracker  Downloaded
  ✓  kernel/linux-6.8-x86_64  Already available
  ✓  distribution/alpine-3.20  Downloaded
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
