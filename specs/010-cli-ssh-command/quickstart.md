# Quickstart: CLI `ssh` Command for Connecting to a MicroVM

## Prerequisites

- A Linux host with the workspace built through Cargo.
- Elevated rights (or `sudo`/`pkexec`) for reading key material; the SDK
  running prerequisites from `specs/008-start-microvm/quickstart.md` still
  apply (verified boot artifacts, intact VM volume and keys).
- At least one running MicroVM in the resolved home (`microvm new`, then
  `microvm start` first).
- The OpenSSH client installed on the host.
- A writable Taumaru home directory; an interactive terminal for selector
  mode, or an explicit name plus rights for automation.

The SDK owns directories, migrations, lifecycle state, and the running
liveness checks below the resolved home. The CLI only resolves that home,
resolves one running machine, escalates, and hands the terminal to `ssh`.
This feature adds no migration and no new dependency. See
[data-model.md](./data-model.md) for the entity shapes and
[cli-ssh.md](./contracts/cli-ssh.md) for the full surface, session, and
exit-status contract.

## Interactive shell

Run with an explicit name (skips the selector):

```text
cargo run -p taumaru-microvm-cli -- ssh web-01
```

Or with no name to pick from the running machines only:

```text
cargo run -p taumaru-microvm-cli -- ssh
```

A non-privileged terminal re-executes itself elevated (sudo, pkexec
fallback) as `ssh <name> --non-interactive` after the machine is known;
there is no connection confirmation to approve. The command prints nothing on
success: the guest shell takes over the terminal exactly as if `ssh` had been
run by hand. First connect prompts to accept the host key and remembers it.
Invalid names abort with the naming rule (no re-prompt); a name/`--name`
mismatch aborts before anything opens; Escape cancels with exit `130`. An
unknown name points to `microvm new` without offering creation; a stopped
machine reports its state and points to `microvm start {name}`.

## Remote command

Run one guest command instead of a shell (words after `--` are always remote):

```text
cargo run -p taumaru-microvm-cli -- ssh web-01 -- uname -a
cargo run -p taumaru-microvm-cli -- ssh -- uname -a
```

The second form selects the machine interactively, then runs the command on
it. The remote exit status becomes the CLI exit status, and piped scripted
use stays byte-clean because the CLI prints nothing on success.

## Explicit automation

Provide the name with no prompts (rights already held):

```text
TAUMARU_HOME=/var/lib/taumaru-microvm \
cargo run -p taumaru-microvm-cli -- ssh web-01 --non-interactive -- uname -a
```

The command emits no prompts and returns the remote exit status. A missing
name or missing rights fails before any session with usage guidance.

## Validation scenarios

1. Selector with 3+ machines in mixed states: bare `ssh` lists only running
   names, keyboard selection opens the chosen machine's shell.
2. Explicit name: `ssh web-01` skips the selector and opens the session after
   elevation.
3. Elevation round-trip: non-privileged interactive run re-executes as root
   (carrying the remote command when present) and propagates the child
   status.
4. Remote command: trailing words run in the guest with no shell, and the
   remote exit status is returned verbatim.
5. Stale state: a machine with a dead process or silent socket is absent from
   the selector; naming it reports not-running with the persisted state.
6. Failures: unknown name (points to `new`), invalid name (rule restated),
   name mismatch, empty running set (points to `start`), missing key file,
   missing `ssh` binary, non-interactive without name or rights (usage /
   elevated-rights errors, nothing opened).
7. Fidelity: interactive typing, full-screen output, terminal resizing,
   interrupt handling, and remote exit status match the equivalent manual
   `ssh -i {key} {user}@{address}` invocation exactly.

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

Unit tests must use deterministic fixtures for name agreement, trailing
command parsing, argv building (port flag conditional, no host-key options),
and pre-flight mapping. SDK listing tests must use a temporary home with
seeded rows plus liveness doubles (live, dead-process, silent-socket,
non-running) through the existing test doubles. No test may require KVM,
root, a real launch, an interactive terminal, or a live guest; manual runs
with rights cover the live paths above.
