# Implementation Plan: CLI `ssh` Command for Connecting to a MicroVM

**Branch**: `010-cli-ssh-command` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/010-cli-ssh-command/spec.md`

## Summary

Add a thin CLI `microvm ssh [NAME] [-- COMMAND...]` command over one minimal
additive SDK read operation, `list_running_microvms()` (name plus stored
`SshConnectionInfo`, ordered by name, live-verified with the start
operation's own `is_vm_live` check). The command resolves one machine
(positional/`--name` flag, or an interactive single-select over running
machines only), reuses the existing `privilege.rs` escalation flow unchanged
(re-executes as a privileged child with `ssh <name> --non-interactive
[-- <remote...>]`, mirroring the `start` pattern with no trusted-values
envelope since the child re-resolves through the SDK), pre-flights the chosen
key file, and hands the terminal to `ssh -i {key} [-p {port}] {user}@{address}
[-- remote...]` with fully inherited stdio, no host-key bypass, and verbatim
exit-status propagation. Success prints nothing, keeping piped scripted use
byte-clean.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (workspace `edition = "2024"`).

**Primary Dependencies**: Existing `clap` 4.6 (derive, `trailing_var_arg` +
`allow_hyphen_values` for remote-command delegation), `inquire` 0.9
(`Select`), `tokio` (`process` feature for the session child); `ssh` binary
located with the existing `backend_path("ssh")` lookup. No new crates. SDK
dependency is the path crate `taumaru-microvm`.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema
migration. The new listing composes two existing repository reads
(`list_microvm_names` for name order, `find_microvm` per `Running` candidate)
plus the existing `is_vm_live` probes (recorded process references the VM
**and** the volume-local socket answers). The CLI never opens the database.

**Testing**: `cargo test` suites: CLI unit tests (`commands::ssh` arg
parsing, argv builder, pre-flight mapping, error triplets), CLI surface tests
(`crates/cli/tests/command_surface.rs`), SDK tests for the new listing using
the existing `TestRuntime` liveness doubles (live, dead-process,
silent-socket, non-running). Real sessions stay out of the default suite
(require rights, a live guest, a terminal); session fidelity is covered by
manual quickstart runs.

**Target Platform**: Linux hosts; session execution requires elevated rights
(existing privilege flow) and the OpenSSH client on `PATH`. Interactive shell
use requires an interactive terminal; scripted use (name plus remote command)
may run without one.

**Project Type**: CLI presentation feature in `crates/cli` plus one additive
SDK read operation in `crates/sdk`. No lifecycle engine in the CLI.

**Performance Goals**: One inventory listing per invocation (N+1 reads over
tens of rows, no registry access, no file hashing for listing). Name
resolution is one listing call on the success path; the miss-labeling
fallback (`list_microvms`) runs only on the failure path. No spinner: the
session handoff is immediate and silent.

**Constraints**: CLI must not implement lifecycle behavior, liveness probing,
process management, network repair, or its own escalation; must not open
SQLite directly; must not print key contents; must not add host-key options
that weaken verification; `--non-interactive` performs zero prompts; success
prints nothing (byte-clean pipes); remote exit status propagates verbatim.

**Scale/Scope**: Multiple independent VMs per host are first-class; the
selector lists running rows ordered by name. Out of scope: `stop`/`list`/
`status`/`inspect` commands (referenced only as pointers), kernel override,
custom volume path, explicit LAN address, connection-hint rendering, and any
SDK operation beyond the one additive running-machines listing.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: one new read op, session spawn is presentation handoff, no orchestration | PASS: `ssh.rs` resolves names, escalates, spawns `ssh`; liveness/repair/launch stay in SDK |
| Public SDK silent, typed, panic-free | PASS: listing returns `Result`, no output | PASS: `list_running_microvms` is a read-only typed query; error/output contracts documented |
| Domain independent from infrastructure | PASS: only one new snapshot type reusing `SshConnectionInfo` | PASS: no domain logic changes; only manager/lib touched for listing |
| SQLite is local source of truth | PASS: selector reads via SDK, not files | PASS: existing reads composed; no migration, no CLI database access |
| Firecracker/firectl stay behind runtime port | PASS: listing reuses existing probes, adds none | PASS: no runtime changes; `is_vm_live` reused, not duplicated |
| Multiple MicroVMs supported | PASS: all running rows listed, per-name execution | PASS: no shared CLI state; per-machine resolution and spawn |
| Public contracts documented | PASS: contract/data-model/quickstart planned | PASS: artifacts below; new SDK items get Rustdoc |
| Calm, accessible CLI; English only | PASS: reuse prompt/error patterns; silent session handoff | PASS: ssh-specific triplets, non-color-safe markers, no success output to corrupt pipes |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/010-cli-ssh-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-ssh.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs                    # Re-export RunningMicroVm
│   │   ├── manager.rs                # list_running_microvms coordinator + is_vm_live reuse
│   │   └── domain/microvm.rs         # RunningMicroVm (additive, reuses SshConnectionInfo)
│   └── tests/ + manager unit tests
│       └── (listing tests: live/empty/dead-process/silent-socket/non-running/ordering)
└── cli/
    ├── src/
    │   ├── cli.rs                    # SshArgs + Command::Ssh
    │   ├── commands/
    │   │   ├── mod.rs                # Route Command::Ssh
    │   │   └── ssh.rs                # Name resolution, selector, escalation, spawn
    │   └── error.rs                  # Ssh-specific triplets (additive constructors)
    └── tests/
        └── command_surface.rs        # ssh help, non-interactive guards
```

**Structure Decision**: Presentation lives in `crates/cli` following the constitution-mandated
layout (`cli.rs` definitions, one `commands/` module per command, `error.rs` triplets; no
`output/` addition since success is silent). The selector's data need is met by one additive SDK
read operation reusing the existing domain/ports/adapters layering. No new top-level modules,
crates, or dependencies.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Running-machines source is a new public SDK `list_running_microvms() ->
   Result<Vec<RunningMicroVm>, SdkError>` (name plus stored `SshConnectionInfo`,
   ordered by name), live-verified with the start operation's own `is_vm_live`
   check; persisted-state filtering alone was rejected (stale `Running` rows),
   as were CLI-side probing (boundary violation) and a full `status` op
   (wider than the minimal-listing mandate).
2. Repository layer needs no trait change: iterate existing
   `list_microvm_names` rows, load each `Running` candidate with existing
   `find_microvm`, keep it only when `is_vm_live` passes. No migration, no
   locks, no mutation or repair.
3. Named-miss classification resolves only against the running listing, then
   calls existing `list_microvms` once on the failure path purely to label the
   error (unknown → `microvm new`; known-but-not-running → state plus
   `microvm start {name}`).
4. Key-material validation split: the listing returns stored paths verbatim
   (no per-VM filesystem validation, so one rotted key cannot deny healthy
   sessions); the CLI pre-flights only the chosen entry before spawn.
   Post-spawn connection failures surface through inherited `ssh` output and
   status, exactly as manual.
5. Command line mirrors `StartArgs` plus `command: Vec<String>` with
   `trailing_var_arg`/`allow_hyphen_values`; everything after `--` is remote
   command, otherwise words after the name are; bare `ssh -- <remote>` runs on
   the interactively selected machine.
6. Elevation mirrors `start` exactly via unchanged `privilege.rs`: interactive
   parents re-exec as `ssh <name> --non-interactive [-- <remote...>]` with
   `TAUMARU_HOME` + `TAUMARU_ESCALATED=1`; no trusted-values envelope (the
   child re-resolves through the SDK, so nothing crosses the boundary).
   Non-interactive calls `require_privileged` with an empty command for
   fail-fast.
7. Session spawn inherits stdio fully, adds no host-key options (default
   OpenSSH verification per clarification Q3), installs no Ctrl-C
   interception around the wait, propagates the exit code verbatim (signal
   death → `130`), prints nothing on success, and maps `ssh`-binary absence
   to a guided triplet via the existing `backend_path` lookup.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): CLI entities, the additive SDK listing type, validation rules,
  and state/flow ownership.
- [contracts/cli-ssh.md](./contracts/cli-ssh.md): command surface, input modes, resolution,
  escalation argv, session spawn, error table, exit codes, and out-of-scope bounds.
- [quickstart.md](./quickstart.md): runnable validation scenarios and quality gates.

### Public SDK boundary (additive only)

Extend the `MicroVmSdk` facade with:

- `list_running_microvms(&self) -> Result<Vec<RunningMicroVm>, SdkError>` returning one entry per
  actually-running machine ordered by name, each with `name: String` and `ssh: SshConnectionInfo`.

Add `RunningMicroVm` to `domain/microvm.rs` (additive struct reusing the existing
`SshConnectionInfo`) and re-export it deliberately from `crates/sdk/src/lib.rs` with Rustdoc
(read-only snapshot of one running machine for selectors and SSH resolution; liveness is
verified at call time with the same probes start uses; key paths only, never contents).
Internally: iterate `list_microvm_names`, skip non-`Running` persisted states, load candidates
with `find_microvm`, keep rows passing the existing private `is_vm_live` (process references
the VM **and** the volume-local socket answers). Return typed errors on unparsable stored
state, never panic. No repository trait change, no schema migration, no locks, no mutation.

### SSH command flow (`commands/ssh.rs`)

Implement in this order, mirroring `commands/start.rs` structure:

1. Validate `--non-interactive` prerequisites first: name present (positional or `--name`,
   via the shared `resolve_name`) else missing-value error with usage; rights required up front
   via `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` so
   missing rights fail before any session with no prompt.
2. Interactive path: resolve the name from flags when present (skip selector), else fetch
   `sdk.list_running_microvms()` and present an `inquire::Select` with the shared
   `prompt_render_config`; empty running set exits with a nothing-running error pointing at
   `microvm start`; cancel maps to the ssh-specific cancellation message (exit `130`).
3. Validate the resolved name with the shared rule (abort, no re-prompt), then resolve it
   against `sdk.list_running_microvms()`; on a miss call `sdk.list_microvms()` once to label
   unknown (point to `microvm new`, no creation shortcut) vs known-but-not-running (persisted
   state plus `microvm start {name}` pointer). Then escalate when not privileged via
   `require_privileged(..., non_interactive=false, home, &[], child_argv, retry_hint)`
   where `child_argv` is `["ssh", name, "--non-interactive", ("--", remote...) ]`; return the
   child's exit code when it runs. The child (privileged, `TAUMARU_ESCALATED=1`) skips
   escalation and resolves directly.
4. Execute: locate `ssh` with `backend_path("ssh")` (guided error when absent), pre-flight
   the chosen private-key path (readable regular file, else calm triplet, no session), then
   spawn `ssh -i {key} [-p {port} when != 22] {user}@{address} [-- remote...]` with inherited
   stdin/stdout/stderr and no host-key options; wait without Ctrl-C interception; exit with
   the child's status verbatim (signal death maps to `130`).

No registry access, no artifact work, no confirmation prompt, no lifecycle mutation, no second
liveness probe in this command. Trailing remote words are appended verbatim; no shell joins
them.

### Output and error design (`error.rs`, silent success)

- On success the command prints nothing: no spinner, no banner, no result report. The selector
  prompt (interactive) and the elevation prompt (privilege flow) are the only pre-session
  output, both pre-existing patterns.
- `error.rs`: additive ssh-specific constructors reusing the triplet format
  (`ssh_not_found` with the `microvm new` pointer, `ssh_not_running` with state plus the
  `microvm start {name}` pointer, `ssh_empty` pointing at start, `ssh_cancelled` with exit
  `130`, key/ssh-binary failures); existing creation/download/start messages are not
  reworded.

### Test implementation

- CLI unit tests: name agreement/mismatch via the shared resolver; trailing-command parsing
  (`--` boundary, words-after-name, bare `--` form); argv builder (port flag conditional on
  `!= 22`, no host-key options, remote words verbatim); pre-flight mapping (missing key →
  triplet, no session); escalated child argv round-trip.
- CLI surface tests: `ssh --help` exposes `[NAME]`, `[COMMAND]...`, `--name`,
  `--non-interactive` and no lifecycle flags; `ssh --non-interactive` without a name fails
  with the missing-name message and no prompt; non-interactive without rights reports
  elevated-rights (skipped when euid is 0, same as the existing privilege test).
- SDK tests: `list_running_microvms` on an empty inventory returns empty; live VMs appear
  ordered by name with stored SSH material; dead-process, silent-socket, stopped, and
  creating rows are all absent; no stdout/stderr/panic on the listing path. Use the existing
  `TestRuntime` liveness doubles, not real launches.
- Manual (quickstart): interactive selector with mixed states, stale-state absence, elevation
  round-trip carrying the remote command, remote-command exit propagation, piped scripted
  use, fidelity comparison against manual `ssh -i`, and host-key first-connect prompt —
  run with rights where required.
