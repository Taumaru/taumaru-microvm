# Phase 0 Research: CLI `ls` Command for Listing MicroVMs

## Research goals

Resolve how the `ls` command delivers its spec: which SDK surface feeds the
overview given that `MicroVmSummary` today carries only name plus verified
state, how elevation reuses the existing privilege flow for a command that
takes no machine name, and how the single compact table (capacities, image
reference, network details, no SSH material, dashes for degraded rows) is
presented — all while keeping the SDK touch strictly minimal and additive,
with no second lifecycle engine, no new dependencies, and no schema changes.

## Findings

### Data source: extend `MicroVmSummary` with already-persisted fields, no new operation

- Decision: extend the existing `MicroVmSummary` struct in
  `crates/sdk/src/domain/microvm.rs` with additive public fields populated
  from data `list_stored_microvms()` already loads, and populate them in the
  existing `MicroVmSdk::list_microvms()` in `crates/sdk/src/manager.rs`:
  `vcpu_count: u32`, `memory_bytes: u64`, `disk_size_bytes: u64`,
  `distribution_id: String`, `image_id: String` (all from `MicroVmRecord`,
  always present when the row exists), plus `network_mode:
  Option<NetworkMode>`, `guest_address: Option<IpAddr>`, `lan_address:
  Option<IpAddr>` (from the optional `PersistedNetwork`; `None` when the
  record is incomplete after interrupted creation). The call-time verified
  `state` semantics, name ordering, and probe-failure-resolves-to-`Stopped`
  behavior stay exactly as they are.
- Rationale: FR-003 names the existing listing operation as the data source
  with extra already-persisted fields on its result. Every field is already
  read by `list_stored_microvms()` (the `microvms` row plus the `vm_networks`
  row via `load_network_record`), so the extension adds zero new queries,
  zero schema migration, and zero registry or filesystem probing. One listing
  path keeps the verify semantics identical for selectors and the overview —
  no drift between two parallel listings. The stale struct-level doc comment
  (which claims the state is never live-verified) is corrected as part of
  the same edit since the new fields force the doc to describe the snapshot
  honestly.
- Alternatives considered: a new `list_microvm_details()` returning a new
  struct while keeping `MicroVmSummary` frozen (rejected — a second verify
  path that can drift from the selector path, two listings to maintain, for
  no behavioral gain; the in-repo exhaustive literals are only two CLI test
  sites plus docs, all updated in the same change); a per-VM getter called in
  a loop (rejected — N+1 queries plus a state race between the listing and
  the detail reads); live-measured disk usage via filesystem probing
  (rejected — the clarify session decided configured values only, and
  probing adds failure modes plus latency).

### Elevation: single pre-listing gate, bare `["ls"]` child, no second gate

- Decision: reuse `privilege.rs` unchanged with one gate before any listing
  attempt, mirroring the `stop` bare path minus its second gate (there is no
  machine name to carry). Non-interactive mode calls
  `require_privileged(..., non_interactive=true, ..., Vec::new(),
  retry_hint)` so missing rights fail fast with no prompt. The interactive
  path calls `require_privileged(..., non_interactive=false, home, &[],
  vec![OsString::from("ls")], retry_hint)`: unprivileged terminals re-execute
  as `microvm ls` elevated (`sudo` preferred, `pkexec` fallback, with
  `TAUMARU_HOME` and the `TAUMARU_ESCALATED=1` guard) and exit with the
  child's status; the privileged child (or an already-root invocation) sees
  euid 0 and proceeds straight to the listing. No interactive terminal is
  required beyond the gate — piped runs (`microvm ls | head`) render the
  same table to stdout, since the command takes no input and shows no
  selector.
- Rationale: `ls` has no selector and no per-machine resolution, so the
  stop/start double gate (pre-selector plus pre-call with the resolved name)
  collapses to one gate with nothing to carry. Elevation decline or
  interrupt propagates exactly as it does for `stop` (child exit status,
  signal death mapping to `130`) with no new cancellation constructor.
- Alternatives considered: a pre-call second gate (rejected — nothing to
  carry, the name slot does not exist for `ls`); gating after the listing
  (rejected — spec mandates elevation before any listing attempt, and the
  socket probes themselves may need rights).

### Rendering: one compact table, established size formatters, text-first state

- Decision: add `format_ls_table(&[MicroVmSummary],
  TerminalCapabilities) -> String` plus `write_ls_table` in
  `crates/cli/src/output/human.rs`, following the existing format/write
  pair convention. Columns: `NAME STATE VCPUS MEMORY DISK IMAGE NETWORK`,
  one row per machine in the SDK's name order, widths computed from content,
  dim header, no truncation — the same column set on every terminal for
  deterministic scripted output. Sizes reuse the established helpers from
  `commands::new`: `format_gb` for disk, `format_mb_gb` for memory (already
  imported by `human.rs`). Image renders as `{distribution_id}={image_id}`,
  the same token form as the `--image DISTRIBUTION_ID=IMAGE_ID` flag, so the
  value is copy-pasteable. Network renders as `host-only {guest}` for
  host-only machines, `lan {lan} (guest {guest})` when a LAN address is
  committed, `lan (guest {guest})` when LAN-exposed without one, and `-` for
  any missing network value (incomplete record). State renders as the
  `MicroVmState` display text (`running`/`stopped`) with semantic color
  (running green, stopped dim, gated by `capabilities.color`) — text labels
  carry the meaning, so non-color output is lossless. SSH user, port, and key
  paths never appear.
- Rationale: the clarify session fixed a single compact table, configured
  values only, network included, SSH excluded, dashes for degraded rows.
  Reusing the `new`-command size formatters keeps units identical between
  creation review and listing. No spinner: the listing is one SDK call with
  bounded parallel probes, and deterministic table bytes matter more for a
  read-only overview than a transient indicator.
- Alternatives considered: detailed blocks per machine (rejected in
  clarification — long output for tens of machines); hiding IMAGE/NETWORK on
  narrow terminals (rejected — hides the point of the command and breaks
  output determinism; wrapping is left to the terminal); a `--json` or
  `--format` flag (rejected — FR-012 puts output-format flags out of scope).

### Empty inventory: stdout report with exit 0, not an error

- Decision: when the SDK returns an empty listing, render
  `format_ls_empty`/`write_ls_empty` (calm "no machines" report pointing to
  `microvm new`) and exit `0`. No `CliError` constructor is involved, so no
  exit-code arm is needed.
- Rationale: FR-008 mandates success status for the empty case; routing it
  through the error type would force an exit-code exception. The existing
  `start_empty_inventory`/`stop_empty` triplets stay untouched (they remain
  errors for commands that need a machine to act on).
- Alternatives considered: reusing `stop_empty` (rejected — wrong operation
  name, wrong recovery pointer, and it exits `1`).

### Errors: one additive `ls_failed` constructor, existing arms cover the rest

- Decision: add `CliError::ls_failed(what, &SdkError)` mirroring
  `stop_failed`/`start_failed` for listing-operation failures (unreadable
  inventory, database or base-directory failure surface as typed `SdkError`
  through the single SDK call). Non-interactive missing-rights reuses the
  existing `privileged` shape; home-resolution failures reuse the existing
  `Home` path. No new `exit_code` arm: elevation decline/interrupt behavior
  is inherited from the privilege flow unchanged.
- Rationale: FR-009 requires the calm what/why/next triplet with nonzero
  status; the additive-constructor pattern is exactly how `start` and `ssh`
  extended `error.rs`, and existing messages are not reworded.
- Alternatives considered: reusing `stop_failed` (rejected — names the wrong
  operation and points at the wrong retry).

### Command surface: `Ls` variant with `visible_alias = "list"`, flag-only args

- Decision: add `Command::Ls(LsArgs)` with doc `List all MicroVMs with state
  and configured capacities.` and `#[command(visible_alias = "list")]`, plus
  `LsArgs { non_interactive: bool }` mirroring the other commands' flag. No
  positional name, no per-machine flags (rejected positionals surface as a
  clap error, covered by a parser test). Route the arm in `commands/mod.rs`
  to a new `commands/ls.rs`.
- Rationale: `visible_alias` (not `alias`) advertises `list` in help output,
  and a single variant guarantees both spellings share one code path by
  construction (FR-001). The `--non-interactive` flag keeps parity with the
  established privilege gating contract (FR-007).
- Alternatives considered: two variants sharing a function (rejected — two
  paths that can diverge, violating the pure-alias requirement); a hidden
  `alias` (rejected — discoverability matters for a primary listing
  command).

No unresolved technical questions remain for Phase 1 design.
