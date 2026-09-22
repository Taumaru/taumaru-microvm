# Quickstart: CLI `delete` Command for Deleting a MicroVM

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Root access (or `sudo`/`pkexec`) for deleting machines; the SDK
  running prerequisites from `specs/017-delete-microvm/quickstart.md` still
  apply (stored VM in the explicit SDK home with its volume directory and
  inventory record intact).
- At least one stored MicroVM in the resolved home (`microvm new` first;
  stopping is not required to list, only to delete a running machine).
- A writable Taumaru home directory; an interactive terminal for selector
  and confirmation modes, or an explicit name plus root access for automation.

The SDK owns directories, migrations, lifecycle state, liveness, file removal,
network release, and record deletion below the resolved home. The CLI only
resolves that home, resolves one machine, escalates, confirms, invokes the
single existing `delete_microvm` operation, and renders the deleted report.
This feature adds no migration, no new dependency, and no SDK operation. See
[data-model.md](./data-model.md) for the entity shapes and
[cli-delete.md](./contracts/cli-delete.md) for the full surface, escalation,
confirmation, and exit-status contract.

## Interactive delete

Run with an explicit name (skips the selector):

```text
cargo run -p taumaru-microvm-cli -- delete web-01
```

Or with no name to pick from every stored machine with state labels:

```text
cargo run -p taumaru-microvm-cli -- delete
```

A non-privileged terminal re-executes itself elevated (sudo, pkexec
fallback): bare path as `delete`, resolved path as
`delete <name> --non-interactive`. After elevation one confirmation names the
machine with a permanent-loss warning (declining cancels with exit `130`);
while the machine is deleted a live status line is shown (bounded by the SDK
delete work, under 2 minutes); on success the report names the deleted
machine, confirms owned traces are gone, notes shared artifacts are preserved,
and shows the `microvm new` next step. Invalid names abort with the naming
rule (no re-prompt); a name/`--name` mismatch aborts before anything is
deleted; Escape cancels with exit `130`. An unknown name points to
`microvm new` without offering creation. A running machine reports the
stop-first refusal with `microvm stop {name}` and deletes nothing.

## Explicit automation

Provide the name with no prompts (root access already held):

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- delete web-01 --non-interactive
```

The command emits no prompts (no selector, no confirmation) and exits `0`
with the deleted report. A missing name or missing root access fails before
any deletion attempt with usage guidance.

## Validation scenarios

1. Selector with 3+ machines in mixed states (running, stopped,
   never-started): bare `delete` lists every stored name with state labels
   and ssh-identical behavior; keyboard selection plus confirmation deletes
   the chosen machine.
2. Explicit name: `delete web-01` skips the selector, confirms once, and
   deletes after elevation.
3. Elevation round-trip: non-privileged interactive run re-executes as root
   (bare as `delete`, resolved as `delete <name> --non-interactive`) and
   propagates the child status; the SDK call always runs privileged.
4. Confirmation: declining or interrupting the prompt deletes nothing with
   exit `130`; non-interactive skips the prompt entirely.
5. Running refusal: deleting a running machine deletes nothing and reports
   the stop-first step with the exact stop command; stopping then deleting
   succeeds.
6. Repeat delete: deleting the same name again reports not found, not success.
7. Retryable failure: an unreleasable owned item reports the cause with the
   fix-and-retry next step and exit `1`; fixing and rerunning deletes.
8. Failures: unknown name (points to `new`), invalid name (rule restated),
   name mismatch, empty stored set (points to `new`), retryable failure,
   non-interactive without name or rights (usage / elevated-rights errors,
   nothing deleted).
9. Cancellation: selector cancelled, elevation declined, confirmation
   declined, or interrupt during selector/elevation/confirmation reports
   cancellation with exit `130` and nothing deleted.
10. Presentation: spinner focus, confirmation prompt, deleted report, and
    failure status remain distinguishable through text in non-color and
    narrow terminals.

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
child argv round-trip, confirmation default-decline, delete-report rendering
(removed/preserved lines, creation hint), and error-triplet mapping. No test
may require KVM, root, a real deletion, or an interactive terminal; manual
runs with rights cover the live paths above.
