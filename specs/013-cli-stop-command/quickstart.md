# Quickstart: CLI `stop` Command for Stopping a MicroVM

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Root access (or `sudo`/`pkexec`) for stopping machines; the SDK
  running prerequisites from `specs/012-stop-microvm/quickstart.md` still
  apply (running VM in the explicit SDK home with its volume directory and
  control socket intact).
- At least one running MicroVM in the resolved home (`microvm new`, then
  `microvm start` first).
- A writable Taumaru home directory; an interactive terminal for selector
  mode, or an explicit name plus root access for automation.

The SDK owns directories, migrations, lifecycle state, and the shutdown/wait
mechanics below the resolved home. The CLI only resolves that home, resolves
one machine, escalates, invokes the single existing `stop_microvm` operation,
and renders the stopped report. This feature adds no migration, no new
dependency, and no SDK operation. See [data-model.md](./data-model.md) for
the entity shapes and [cli-stop.md](./contracts/cli-stop.md) for the full
surface, escalation, and exit-status contract.

## Interactive stop

Run with an explicit name (skips the selector):

```text
cargo run -p taumaru-microvm-cli -- stop web-01
```

Or with no name to pick from the running machines only:

```text
cargo run -p taumaru-microvm-cli -- stop
```

A non-privileged terminal re-executes itself elevated (sudo, pkexec
fallback): bare path as `stop`, resolved path as
`stop <name> --non-interactive`; there is no stop confirmation to approve.
While the machine shuts down a live status line is shown (bounded by the SDK
60-second graceful window plus the forced fallback); on success the report
names the stopped machine, states whether the shutdown was graceful or
forced, and shows the `microvm start {name}` next step. Invalid names abort
with the naming rule (no re-prompt); a name/`--name` mismatch aborts before
anything stops; Escape cancels with exit `130`. An unknown name points to
`microvm new` without offering creation. An already-stopped named machine
reports stopped with no forcing used.

## Explicit automation

Provide the name with no prompts (root access already held):

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- stop web-01 --non-interactive
```

The command emits no prompts and exits `0` with the stopped report. A missing
name or missing root access fails before any stop attempt with usage
guidance.

## Validation scenarios

1. Selector with 3+ machines in mixed states: bare `stop` lists only running
   names with ssh-identical behavior; keyboard selection stops the chosen
   machine.
2. Explicit name: `stop web-01` skips the selector and stops after elevation.
3. Elevation round-trip: non-privileged interactive run re-executes as root
   (bare as `stop`, resolved as `stop <name> --non-interactive`) and
   propagates the child status; the SDK call always runs privileged.
4. Already-stopped: naming a stopped machine reports stopped with no forcing
   used and exit `0`.
5. Forced path: a machine that ignores the graceful request reports stopped
   with the forced outcome marked in text, distinguishable from graceful by
   the report alone.
6. Failures: unknown name (points to `new`), invalid name (rule restated),
   name mismatch, empty running set (points to `start`), lifecycle conflict,
   non-interactive without name or rights (usage / elevated-rights errors,
   nothing stopped).
7. Cancellation: selector cancelled, elevation declined, or interrupt during
   selector/elevation reports cancellation with exit `130` and nothing
   stopped.
8. Presentation: spinner focus, stopped report, and failure status remain
   distinguishable through text in non-color and narrow terminals.

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

Unit tests must use deterministic fixtures for name agreement, escalated
child argv round-trip, stop-report rendering (graceful vs forced text,
restart hint), and error-triplet mapping. No test may require KVM, root, a
real shutdown, or an interactive terminal; manual runs with rights cover the
live paths above.
