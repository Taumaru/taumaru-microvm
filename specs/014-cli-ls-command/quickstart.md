# Quickstart: CLI `ls` Command for Listing MicroVMs

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Root access (or `sudo`/`pkexec`) for the listing; the socket liveness
  probes touch host-local runtime state below the SDK home.
- At least two MicroVMs in the resolved home for the mixed-state scenario
  (`microvm new`, then `microvm start` for one of them); an empty home
  for the empty-report scenario.
- A writable Taumaru home directory; an interactive terminal for the
  elevation prompt, or root access already held for automation and pipes.

The SDK owns directories, migrations, verification, and the inventory
below the resolved home. The CLI only resolves that home, escalates once,
invokes the single existing `list_microvms` operation (with additive
already-persisted fields), and renders the table or the empty report.
This feature adds no migration, no new dependency, and no SDK operation.
See [data-model.md](./data-model.md) for the entity shapes and
[cli-ls.md](./contracts/cli-ls.md) for the full surface, escalation, and
exit-status contract.

## Interactive listing

Run with either spelling (identical output):

```text
cargo run -p taumaru-microvm-cli -- ls
cargo run -p taumaru-microvm-cli -- list
```

A non-privileged terminal re-executes itself elevated as bare `ls`;
there is no selector to approve and no confirmation. On success the
single compact table shows every machine in name order with state,
vCPUs, memory, disk, image token, and network; degraded rows keep their
place with dashes for missing values. An empty inventory prints the calm
no-machines report pointing to `microvm new` with exit `0`. Piped runs
(`... ls | head`) render the same table once privileged. Invalid
operands (a positional name, lifecycle flags) abort as clap usage
errors; Escape during elevation cancels with exit `130`.

## Explicit automation

No prompts (root access already held):

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- ls --non-interactive
```

The command emits no prompts and exits `0` with the table (or the empty
report). Missing root access fails before any listing attempt with usage
guidance.

## Validation scenarios

1. Mixed states: `ls` with running plus stopped machines shows every
   row once, in name order, with correct states and matching configured
   capacities, image tokens, and network strings.
2. Alias parity: `list` output is byte-identical to `ls` for the same
   inventory and privilege context.
3. Elevation round-trip: non-privileged interactive run re-executes as
   bare `ls` and propagates the child status; the listing always runs
   privileged.
4. Empty inventory: calm no-machines report pointing to `microvm new`,
   exit `0`, no table and no error.
5. Degraded row: a machine with an incomplete record (or a timed-out
   probe) keeps its row with dashes for missing values while every
   readable machine still lists.
6. Failures: stray positional/flag operands (usage error), incomplete
   creation conflicts via `ls_failed`, non-interactive without rights
   (elevated-rights error, nothing listed), broken inventory (nonzero
   status with cause and next step).
7. Cancellation: elevation declined or interrupt during elevation
   reports cancellation with exit `130` and no listing.
8. Presentation: table, empty report, and failure status remain
   distinguishable through text in non-color and narrow terminals;
   piped output matches terminal output.

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

Unit tests must use deterministic fixtures for child argv round-trip,
table rendering (column presence, name order, configured sizes, image
token, network strings, dash-filled degraded row, color-off text
distinction), empty-report content, and error-triplet mapping. No test
may require KVM, root, a real guest, or an interactive terminal; manual
runs with rights cover the live paths above.
