# Quickstart: CLI `new` Command for Guided MicroVM Creation

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Network access to the Taumaru Artifacts Registry at `https://artifacts.taumaru.com/v1/`
  (or a fixture server for offline verification — see below).
- A writable Taumaru home directory.
- An interactive terminal for guided mode, or a complete explicit flag set
  for automation.

The SDK owns all directories, SQLite migrations, artifact files, integrity
checks, cache decisions, and lifecycle state below the resolved home. The
CLI only resolves that home and presents the operation. This feature adds no
migration and no new dependency. See [data-model.md](./data-model.md) for the
request entity shapes and [cli-new-command.md](./contracts/cli-new-command.md)
for the full input, output, and exit-status contract.

## Guided creation

Run:

```text
cargo run -p taumaru-microvm-cli -- new
```

The command collects values in a fixed order, prompting only for what is
missing. Creation needs root: after confirmation a non-root terminal re-runs
itself elevated (sudo, pkexec fallback) with the collected values; without a
TTY or with `--non-interactive`, run the same command with sudo or as root.

```text
cargo run -p taumaru-microvm-cli -- new web-01 --disk-gb 20 --memory 2GB
```

With the name, disk, and memory supplied above, the command prompts only for
the image, vCPUs, and network mode. A positional name and `--name` are
interchangeable; when both are present they must agree.

The flow loads the current registry collections, then:

1. resolves or prompts for the machine name;
2. shows one single-select image list (entries name their parent
   distribution and carry a `downloaded` / `needs download` text marker);
3. prompts for disk GB, memory `xMB`/`xGB`, vCPU count, and LAN exposure in
   that order;
4. shows a creation summary (chosen values plus prerequisites still to
   fetch) and asks for explicit confirmation;
5. provisions the runtime bundle, the distribution default kernel, and the
   selected image in that order with live per-step progress;
6. creates the VM with live creation progress until it is configured and
   stopped, then prints the result summary.

Invalid input aborts with the rule and minimum shown; there is no re-prompt
loop. Press Escape during selection or decline the confirmation to cancel
before transfer: nothing is downloaded and no VM is created.

Press `Ctrl-C` during provisioning to stop the current transfer. The SDK
removes the partial temporary artifact, keeps prerequisites already
verified, starts no later member, and the CLI exits with code `130`. A
`Ctrl-C` during the creation step itself is reported only after the
operation settles, preserving its outcome and rollback behavior.

## Explicit automation

Provide the complete set with no prompts:

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- new web-01 \
  --non-interactive \
  --image <distribution-id>=<image-id> \
  --disk-gb 20 \
  --memory 2GB \
  --vcpus 2
```

Add `--expose-lan` only for LAN exposure; its absence means host-only.
Exactly one `--image` is required; memory accepts forms like `512MB`,
`512 MB`, `2GB`, and `1.5GB` (case-insensitive). The command emits no
prompts and returns exit code `0` only after the VM is configured and
stopped. Use the IDs shown by the interactive list or the registry manifest.

## Expected output shape

The exact progress refresh is terminal-dependent, but the semantic content
remains stable:

```text
·  Checking artifact registry
✓  Registry ready · 3 distributions · 4 kernels · 3 runtime packages

◆ New MicroVM
────────────────────────────────────────────────

  Name      web-01
  Image     ubuntu-24.04 / ubuntu-24.04-docker (Docker · 4.0 GiB) [downloaded]
  Disk      20 GB
  Memory    2 GB
  vCPUs     2
  Network   host-only

  Needs download: runtime/firecracker-1.14.1-x86_64, kernel/linux-6.8-x86_64

Create this MicroVM? [y/N]

⠋ ━━━━━━━━━━━━━━━━━━━━━━━━━━━ 4.0 GiB/4.0 GiB image/ubuntu-24.04/ubuntu-24.04-docker · Downloading
✓  Prerequisites ready
◆  creation/validation  Finished  1/6
◆  creation/volume_preparation  InProgress  512 MiB / 4.0 GiB
✓  MicroVM web-01 created · host-only 192.168.127.2 · 20 GB · 2 GB · 2 vCPUs
   SSH root@192.168.127.2:22 · key ~/.taumaru-microvm/vms/web-01/ssh/id_ed25519
```

For a valid cache hit the label is `Already available`; for a valid file not
yet recorded by the SDK inventory it is `Adopted`. These states come from
the SDK, never inferred by the CLI. See
[sdk-image-readiness.md](./contracts/sdk-image-readiness.md) for the
readiness query behind the image marker.

## Validation and failure checks

Before confirmation (or before any transfer in non-interactive mode), the
command must reject:

- a missing name, or a positional name and `--name` that disagree;
- a name that is not path-friendly (empty, too long, separators,
  whitespace, or otherwise invalid);
- a missing, malformed, or repeated `--image` value (anything but exactly
  one `distribution=image`);
- an unknown distribution or an image not published by its named
  distribution;
- a distribution, image, or resolved default kernel for another
  architecture;
- an affected distribution whose published default kernel is missing;
- disk input that is non-numeric, zero, negative, overflowing, or below the
  image registry size;
- memory input with a missing or unknown unit, non-numeric amount,
  overflow, or below the distribution minimum;
- vCPU input that is non-integer, zero, or below the distribution minimum;
- runtime packages that do not collectively provide both `firecracker` and
  `firectl`;
- a registry response that the SDK reports as unavailable, malformed, or
  unsupported.

After confirmation, a runtime failure stops dependent work. A kernel or
image failure creates no VM. A name conflict with different settings leaves
the existing VM unchanged; an identical repeat reports the existing VM
without duplicate work. Any failure produces a nonzero result, names the
failed value or prerequisite, retains verified work, and recommends retrying
after the underlying issue is corrected.

## Verification commands for implementation

From the repository root, run the focused suites first:

```text
cargo fmt --all -- --check
cargo test -p taumaru-microvm-cli --all-targets --all-features
cargo test -p taumaru-microvm --all-targets --all-features
```

Then run the workspace gates:

```text
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

SDK tests must use the deterministic fixture server and manifest for the new
readiness query (ready, missing, stale, error propagation). CLI tests must
use deterministic catalog/download/creation fakes for parsing, minimums,
ordering, review, progress, and failure behavior. No test may require the
public registry or a real interactive terminal.
