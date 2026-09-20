# Quickstart: CLI `start` Command for Launching a MicroVM

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Root access (or `sudo`/`pkexec`) for the actual launch; the SDK start prerequisites from
  `specs/008-start-microvm/quickstart.md` still apply (KVM, verified boot artifacts, intact VM
  volume and keys).
- At least one created MicroVM in the resolved home (`microvm new` first).
- A writable Taumaru home directory; an interactive terminal for selector mode, or an explicit
  name plus root for automation.

The SDK owns directories, migrations, lifecycle state, network repair, and the background launch
below the resolved home. The CLI only resolves that home, resolves one name, escalates, and
presents the result. This feature adds no migration and no new dependency. See
[data-model.md](./data-model.md) for the entity shapes and
[cli-start.md](./contracts/cli-start.md) for the full surface, output, and exit-status contract.

## Interactive start

Run with an explicit name (skips the selector):

```text
cargo run -p taumaru-microvm-cli -- start web-01
```

Or with no name to pick from every locally created machine:

```text
cargo run -p taumaru-microvm-cli -- start
```

A non-root terminal re-executes itself elevated (sudo, pkexec fallback) as
`start <name> --non-interactive` after the name is known; there is no start confirmation to
approve. While the SDK call runs, a spinner (`Starting MicroVM web-01`) shows progress without
invented values, then clears for the running report:

```text
✓ MicroVM web-01 running

  Connect: microvm ssh web-01
  Direct:  sudo ssh -i ~/.taumaru-microvm/vms/web-01/ssh/id_ed25519 -p 22 root@192.168.127.2
  Stop:    microvm stop web-01
```

LAN-exposed machines add a paragraph between Direct and Stop explaining the key copy to the other
machine plus the remote ssh form against the committed LAN address. Invalid names abort with the
naming rule (no re-prompt); a name/`--name` mismatch aborts before anything starts; Escape cancels
with exit `130`. An unknown name points to `microvm new` without offering creation.

## Explicit automation

Provide the name with no prompts (already root required):

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- start web-01 --non-interactive
```

The command emits no prompts and returns exit code `0` only after the machine is running and the
report is printed. A missing name or missing root fails before any mutation with usage guidance.

## Repeats and the `new` summary

Starting an already-running machine reports running with the identical hint layout and launches no
second process. Creating a machine now ends with the additional summary line:

```text
  Start:   microvm start web-01
```

## Validation scenarios

1. Selector with 3+ created VMs: bare `start` lists all names, keyboard selection starts the
   chosen machine, and the report shows connect/direct/stop hints in order.
2. Explicit name: `start web-01` skips the selector and starts immediately after elevation.
3. Elevation round-trip: non-root interactive run re-executes as root and propagates the child
   status; the direct hint carries the backend prefix used.
4. LAN vs host-only: a LAN-exposed VM shows the key-copy paragraph with the committed LAN
   address; a host-only VM shows none.
5. Repeat: starting a running VM reports running with one machine process afterward.
6. Failures: unknown name (points to `new`), invalid name (rule restated), `Creating` row
   (lifecycle conflict), empty inventory (nothing to start), non-interactive without name or root
   (usage/elevated-rights errors, nothing started).
7. `new` regression: a successful creation still shows all prior rows plus the start line.

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

Unit tests must use deterministic fixtures for parsing, hint ordering, LAN conditionality, prefix
handling, and non-color rendering. SDK listing tests must use a temporary home with seeded rows
(ordered names, empty inventory, persisted states). No test may require KVM, root, a real launch,
or an interactive terminal; manual runs as root cover the live paths above.
