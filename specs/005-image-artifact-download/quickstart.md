# Quickstart: Image Artifact Download

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Network access to the Taumaru Artifacts Registry at `https://artifacts.taumaru.com/v1/`
  (or a fixture server for offline verification — see below).
- A writable Taumaru home directory.
- An interactive terminal for selector mode, or explicit `--image` IDs for automation.

The SDK owns all directories, SQLite migrations, artifact files, integrity checks, and
cache decisions below the resolved home. The CLI only resolves that home and presents
the operation. This feature adds no migration and no new dependency.

## Interactive download

Run:

```text
cargo run -p taumaru-microvm-cli -- artifacts download
```

The command loads the current registry collections, then:

1. select one or more images from the single sorted image list (entries name their
   parent distribution; several images of one distribution are allowed);
2. review the selected images, resolved default kernels, runtime packages, and the
   estimated size of selected artifacts only;
3. confirm `Start download`;
4. observe runtime, default-kernel, and per-image progress until the verified result.

Press Escape during selection or review to cancel before transfer. A cancellation
does not alter the artifact inventory. A repeated pick of the same image collapses to
one entry.

Press `Ctrl-C` during a transfer to stop the current operation. The SDK removes the
partial temporary artifact, keeps groups already verified, does not start later
groups, and the CLI exits with code `130` after summarizing completed, failed, and
cancelled groups.

## Explicit automation

Provide repeatable scoped image selections:

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- artifacts download \
  --non-interactive \
  --image <distribution-id>=<image-id> \
  --image <distribution-id>=<other-image-id>
```

Repeat `--image` for more images, including several of one distribution. The command
emits no selectors and returns exit code `0` only after every required artifact has
been verified. Use the IDs shown by the interactive list or the registry manifest.
The old `--distribution` / `--kernel` flags are gone; their use is rejected with
guidance toward `--image`.

Migration from the previous command:

```text
# before
microvm download --non-interactive \
  --distribution <distribution-id> --kernel <distribution-id>=<kernel-id>
# after: pick the images explicitly (kernels resolve automatically)
microvm artifacts download --non-interactive \
  --image <distribution-id>=<image-id>
```

## Expected output shape

The exact progress refresh is terminal-dependent, but the semantic content remains
stable. See [cli-artifacts-download.md](./contracts/cli-artifacts-download.md) for
the label, ordering, and exit-status contract:

```text
·  Checking artifact registry
✓  Registry ready · 3 distributions · 4 kernels · 3 runtime packages

◆ Download plan
────────────────────────────────────────────────

Runtime
  •  Firecracker 1.14.1 (firecracker-1.14.1-x86_64) · 2 files · x86_64
     3 files · 8.0 MiB expected

Targets
  •  alpine-3.20 / alpine-3.20-minimal (Alpine Minimal)
     ↳  linux-6.8-x86_64 (Linux 6.8) · default · 120 MiB

Transfer
  145 MiB · 4 planned groups · 2 images

Review the plan above. The download starts after confirmation.

✓  kernel/linux-6.8-x86_64  Already available  16.0 MiB / 16.0 MiB  ·  plan 25.0 MiB / 145 MiB
↓  distribution/alpine-3.20/alpine-3.20-minimal  Downloading  104 MiB / 104 MiB  ·  plan 129 MiB / 145 MiB

✓ Download complete
────────────────────────────────────────────────

  4/4 groups ready · 5 files verified
```

For a valid cache hit the label is `Already available`; for a valid file not yet
recorded by the SDK inventory it is `Adopted`. These states come from the SDK, never
inferred by the CLI. See [sdk-image-download.md](./contracts/sdk-image-download.md)
for the single-image operation contract and [data-model.md](./data-model.md) for the
plan entity shapes behind the review.

## Validation and failure checks

Before confirmation, the command must reject:

- an empty image selection;
- an unknown distribution or an image not published by its named distribution;
- a malformed `--image` value (anything but `distribution=image`);
- a distribution, image, or resolved default kernel for another architecture;
- an affected distribution whose published default kernel is missing;
- runtime packages that do not collectively provide both `firecracker` and `firectl`;
- a registry response that the SDK reports as unavailable, malformed, or
  unsupported.

After confirmation, a runtime failure stops dependent downloads. A default-kernel
failure skips only its dependent images while unrelated images continue. Any failed
or skipped group produces a nonzero result, names the group, retains verified work,
and recommends retrying after the underlying issue is corrected. A transfer
cancellation returns `130`, publishes no partial artifact, and identifies completed,
failed, and cancelled groups.

## Verification commands for implementation

From the repository root, run the focused suites first:

```text
cargo fmt --all -- --check
cargo test -p taumaru-microvm --all-targets --all-features
cargo test -p taumaru-microvm-cli --all-targets --all-features
```

Then run the workspace gates:

```text
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

SDK tests must use the deterministic fixture server and manifest for the new
single-image operation (store-only-requested, whole-distribution preserved,
not-found, incompatible, cancelled). CLI tests must use deterministic
catalog/download fakes for selection, dedup, ordering, review, progress, and
failure behavior. No test may require the public registry or a real interactive
terminal. Offline fixture verification for the flow:

```text
# SDK single-image path against the local fixture server happens inside
# `cargo test -p taumaru-microvm`; no manual server setup is required.
# CLI planning/output behavior is covered by unit tests with the fake client.
```
