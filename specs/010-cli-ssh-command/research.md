# Phase 0 Research: CLI `ssh` Command for Connecting to a MicroVM

## Research goals

Resolve how the `ssh` command delivers its spec: where the running-only selector gets
its rows (and whether the SDK already has such a source), how the minimal additive
SDK listing reuses the start operation's liveness notion, how the command line
carries an optional remote command, how elevation reuses the existing privilege
flow while carrying that remote command, and how the privileged child hands the
terminal to `ssh` with native fidelity — all without a second lifecycle engine,
new dependencies, schema changes, or weakened host-key verification.

## Findings

### Running-machines source: no live-verified listing exists, one additive op needed

- Decision: add `MicroVmSdk::list_running_microvms() -> Result<Vec<RunningMicroVm>, SdkError>`
  returning one entry per actually-running machine (`name` plus the stored
  `SshConnectionInfo`), ordered by name. Liveness reuses the private
  `is_vm_live` check the start operation uses (recorded process still references
  the VM **and** the volume-local control socket answers); the persisted
  `MicroVmSummary.state` snapshot alone is never consulted as running truth.
- Rationale: the only existing listing, `list_microvms`, returns the last
  persisted state and is documented as never live-verified, so it cannot feed a
  running-only selector. The runtime port already exposes exactly the two probes
  start trusts (`process_references_vm`, `socket_answers`).
- Alternatives considered: filtering `list_microvms` by persisted `Running`
  (rejected — a crashed VM keeps a stale `Running` row with a dead process and
  would be offered, then fail at connect); a CLI-side liveness probe
  (rejected — the CLI must not touch process tables, sockets, or SQLite; it is
  constitutionally a presentation layer); a full `status` operation
  (rejected — wider than the spec's minimal-listing mandate).

### Repository layer: no trait change, reuse two existing reads

- Decision: implement the new operation in `manager.rs` only, by iterating the
  existing `list_microvm_names` rows (name order preserved) and loading each
  candidate with the existing `find_microvm`. Skip candidates whose persisted
  state is not `Running` before probing, then apply `is_vm_live`; collect the
  survivors. No new repository method, no schema migration, no locks (single
  read statements through `run_repository`), no mutation or repair.
- Rationale: FR-004 demands a strictly additive, minimal SDK change. The N+1
  read pattern is proportionate — hosts hold tens of VMs, and the selector runs
  once per invocation. Adding a bulk full-record query would widen the internal
  port for no measurable gain.
- Alternatives considered: new `list_stored_microvms` repository method
  (rejected — larger internal surface for the same result); per-VM locking
  (rejected — read-only liveness probes take no locks in start either).

### Named-miss classification: running listing decides, all-machines listing labels

- Decision: resolve an explicit name **only** against the running listing. On a
  miss, call the existing `list_microvms` once purely to label the error:
  name absent there means unknown (point to `microvm new`); name present means
  known-but-not-running (report its persisted state, point to
  `microvm start {name}`). The all-machines inventory never decides liveness.
- Rationale: FR-005 requires resolution through the running source, but the
  edge cases require distinguishing unknown from stopped with the current
  state. The fallback runs only on the failure path, so the success path stays
  at one SDK call and the liveness guarantee is uncompromised.
- Alternatives considered: returning non-running rows from the new listing
  (rejected — widens the contract beyond "running machines" and duplicates
  `list_microvms`); probing `start_microvm` for classification
  (rejected — start mutates and repairs; a lookup must never launch).

### Key-material validation split: SDK returns stored paths, CLI pre-flights the chosen one

- Decision: the listing performs **no** key-file filesystem validation beyond
  liveness; it returns the stored `SshConnectionInfo` paths verbatim. The CLI
  pre-flights only the chosen entry before spawn (private-key path is a
  readable regular file); failures map to calm triplets and open no session.
  Connection-level failures after spawn (refused, timeout, guest sshd absent)
  surface through the inherited `ssh` stderr and exit status, exactly as in a
  manual invocation — they are not re-wrapped.
- Rationale: failing the whole listing because one VM's key rotted would deny
  the selector for healthy machines; silently skipping the rotted VM would
  misreport it as not-running. Per-choice pre-flight gives the precise error
  for the machine the operator actually asked about. Full contract validation
  (modes, metadata triple) stays owned by creation/start, which already enforce
  it.
- Alternatives considered: per-VM validation inside the listing with skip
  (rejected — mislabels broken as stopped); failing the listing on first broken
  entry (rejected — one bad VM blocks all sessions).

### Command line: positional name plus trailing remote command

- Decision: `SshArgs { name: Option<String>, explicit_name: Option<String>,
  non_interactive: bool, command: Vec<String> }` with
  `#[arg(trailing_var_arg = true, allow_hyphen_values = true)]` on `command`,
  mirroring `StartArgs` for the first three fields. Everything after `--` is
  always remote command; without `--`, words after the name are remote command.
  Bare `microvm ssh -- ls` (no name) runs `ls` on the interactively selected
  machine.
- Rationale: clarification Q1 (remote command) and Q4 (mirror start flags
  exactly) compose into this shape. `trailing_var_arg` is the established clap
  idiom for `ssh`-like delegation and keeps `--non-interactive`/`--name`
  parseable before the delegation boundary.
- Alternatives considered: `--command <STRING>` single flag (rejected — splits
  words through a shell-quoting hazard and diverges from manual `ssh` usage);
  no remote command at all (rejected by clarification Q1).

### Elevation reuse: mirror `start`, carry name plus remote words into the child

- Decision: reuse `privilege.rs` unchanged. Interactive parents re-exec as
  `ssh <name> --non-interactive [-- <remote...>]` with `TAUMARU_HOME` plus the
  `TAUMARU_ESCALATED=1` guard carried inline by the existing `child_arguments`
  helper; non-interactive mode calls `require_privileged` with an empty command
  so missing rights fail fast with the rerun-as-root hint and no prompt. The
  escalated child never re-escalates.
- Rationale: the spec mandates the existing privilege flow with no reinvention,
  and `start.rs` already proves the parent-validates/child-executes pattern
  with exit-code propagation and Ctrl-C mapping. Unlike `new`, no
  trusted-values envelope crosses the boundary: the child re-resolves the name
  through the SDK listing, so nothing parent-validated must be smuggled.
- Alternatives considered: an ssh-specific escalation helper (rejected —
  duplicates backend choice, argv building, TTY guards); passing the resolved
  key path over the boundary (rejected — the privileged child must read the
  inventory itself; a passed path would be a trust bypass and go stale).

### Session spawn: inherit everything, intercept nothing, weaken nothing

- Decision: after elevation, resolve the `ssh` binary with the existing
  `backend_path("ssh")` lookup (checks `/usr/bin`, `/usr/sbin`, then `PATH`);
  absence is a guided triplet, not a panic. Build
  `ssh -i {private_key_path} [-p {port} when != 22] {user}@{address}
  [-- remote...]` with **no** host-key options added, so verification keeps
  default OpenSSH behavior per clarification Q3. Spawn with
  `tokio::process::Command` with stdin/stdout/stderr all inherited and
  `child.wait().await` with **no** `select!` on Ctrl-C, so interrupts and
  window-size changes belong to the session exactly as in a manual invocation.
  Propagate the child's exit code verbatim (signal death maps to `130`,
  matching the existing convention).
- Rationale: FR-008 fidelity is the acceptance bar. Inheriting stdio plus
  refusing to intercept signals is what makes editors, pagers, and Ctrl-C
  behave natively. The CLI's `tokio` dependency already enables
  `process`/`signal` features, so no new crate is needed. Omitting `-p` when
  the port is 22 reproduces the requested `ssh -i {key} root@{ip}` form
  byte-for-byte for today's fixed contract while staying correct if the
  contract ever varies.
- Alternatives considered: `spawn_elevated`-style Ctrl-C interception around
  the session (rejected — the parent would steal SIGINT from the remote side
  and kill editors); adding `StrictHostKeyChecking=no` for convenience
  (rejected — bypasses verification, contradicts clarification Q3);
  piping stdio through the parent (rejected — breaks full-screen programs,
  resize propagation, and binary transparency).

### Presentation: silent handoff, ssh-specific triplets

- Decision: on success the command prints **nothing** — no spinner, no banner,
  no result report — so piped scripted use (`microvm ssh web-01 -- cat file |
  ...`) is byte-clean. All pre-session failures use additive `CliError`
  triplet constructors (`ssh_not_found` pointing at `microvm new`,
  `ssh_not_running` pointing at `microvm start {name}`, `ssh_empty` pointing
  at start, `ssh_cancelled` with exit `130`, key/ssh-binary/home failures);
  existing creation/download/start messages are not reworded.
- Rationale: any success-path output would corrupt remote-command pipelines.
  The selector prompt (interactive) and the elevation prompt (privilege flow)
  are the only pre-session output, both pre-existing patterns.
- Alternatives considered: a "Connecting to …" banner (rejected — pollutes
  pipes and adds no value the session itself does not show); a spinner while
  resolving (rejected — resolution is one fast local query, not a launch).

No unresolved technical questions remain for Phase 1 design.
