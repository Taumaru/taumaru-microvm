# Phase 0 Research: CLI `start` Command for Launching a MicroVM

## Research goals

Resolve how the CLI-only `start` command delivers its spec: where the interactive machine
selector gets its rows, how root elevation reuses the existing privilege flow, how names are
validated, what displays while the SDK start runs, the exact success-output order, how SDK errors
map to calm CLI triplets, and how the `new` summary gains its start line — all without a second
lifecycle engine, new dependencies, or schema changes.

## Findings

### Selector data source: new read-only SDK listing operation

- Decision: add `MicroVmSdk::list_microvms() -> Result<Vec<MicroVmSummary>, SdkError>` returning
  every persisted VM (`name`, `state`) ordered by name from a single
  `SELECT name, state FROM microvms ORDER BY name` query behind the existing repository trait.
- Rationale: the CLI is constitutionally forbidden from opening SQLite itself, and `start_microvm`
  resolves only one exact name (probing names to build a list is impossible). The sibling precedent
  already exists: `list_present_distribution_images` backs the `new` command's downloaded markers
  through the same ports/adapters layering.
- Alternatives considered: CLI-side SQLite read (rejected — duplicates persistence ownership and
  the managed-directory layout); `find_microvm` probing (rejected — no name enumeration possible);
  filesystem directory scan (rejected — source of truth is the database, and stray directories
  would leak into the selector). A `list` CLI command was noted as a natural future consumer of
  the same operation but stays out of scope.

### Elevation reuse: mirror `new`, no trusted-values envelope

- Decision: reuse `crates/cli/src/privilege.rs` unchanged. Interactive parents re-exec as
  `start <name> --non-interactive` with `TAUMARU_HOME` + `TAUMARU_ESCALATED=1` carried inline;
  non-interactive mode calls `require_privileged` with an empty command so missing root fails fast
  with the elevated-rights retry hint and no prompt.
- Rationale: the spec mandates reusing the existing privilege function with no reinvention, and the
  `new` command already proved this exact pattern (parent validates, child executes as root, exit
  code propagates, Ctrl-C maps to `130`). Unlike `new`, no trusted-values environment envelope is
  needed: SDK `start_microvm` takes only the name, so nothing parent-validated must cross the
  privilege boundary.
- Alternatives considered: a start-specific escalation helper (rejected — duplicates backend
  choice, argv building, TTY guards); passing the full new-style envelope anyway (rejected — no
  validated sizes/kernel cross the boundary, so the envelope would be dead weight).

### Name handling: share the `new` validators

- Decision: reuse `commands::new::resolve_name`/`validate_name` (widened to `pub(crate)`) for both
  positional/`--name` agreement and path-friendly validation; abort immediately with the naming
  rule on invalid input, no re-prompt — identical to `new` per the clarification session.
- Rationale: the spec and clarification Q4 demand the same naming rule and abort semantics as
  `new`; a second rule would diverge messages and double the test surface.
- Alternatives considered: a start-local validator (rejected — rule drift risk between commands).

### In-flight indicator: spinner, never invented progress

- Decision: reuse the `CatalogSpinner` indicatif pattern as a `StartSpinner` with message
  `Starting MicroVM {name}`; FR-012 forbids invented values, and SDK start emits no progress
  events, so a zero-progress spinner is the only truthful indicator.
- Rationale: clarification Q2 chose a live spinner over silence or verbose logs, matching the
  immediate-feedback rule inherited from the `new`/download flows with existing, tested spinner
  code.
- Alternatives considered: silent wait (rejected by clarification); verbose step log (rejected —
  SDK exposes no step events for start, so steps would be fabricated).

### Success output order and LAN paragraph

- Decision: render in fixed order — `microvm ssh {name}`, then direct
  `ssh -i {key} -p {port} {user}@{ssh.address}` (elevation prefix when the parent escalated),
  then the LAN paragraph only for LAN-mode results (copy `{private_key_path}` to the other
  machine, then `ssh -i {key} -p {port} {user}@{lan_address}` where the remote address is
  `network.lan_address`), then `microvm stop {name}` last. Key paths only, never contents.
- Rationale: clarification Q5 fixed connect-first/stop-last; the spec fixes the elevation prefix
  and key-copy wording. Remote LAN uses `network.lan_address` (the committed address reachable
  off-host), not the host-local guest address.
- Alternatives considered: stop-first ordering (rejected by clarification); showing key contents
  or fingerprints (rejected — contents are secret, fingerprint adds no connection value).

### Error mapping: start-specific triplets, existing plumbing

- Decision: reuse the `\u{1f}`-joined triplet constructors with start-specific what/why/next text:
  not-found points to `microvm new` with no creation shortcut (clarification Q3); invalid names
  restate the naming rule; lifecycle/prerequisite/network/launch failures carry the SDK message
  plus a retry pointer; cancellation gets a start-specific message because `CliError::cancelled`
  and `Prompt("cancelled")` render creation/download wording today.
- Rationale: constitution requires what/why/next calm failures with nonzero status; the mapping
  table keeps every SDK variant covered without rewording unrelated commands.
- Alternatives considered: passing SDK display text through raw (rejected — loses the next-step
  guidance the spec requires); one generic "start failed" message (rejected — untestable and
  unactionable across distinct failure kinds).

### `new` summary addition: one trailing line

- Decision: append a `Start: microvm start {name}` row to `format_new_result`, keeping every
  existing row byte-identical.
- Rationale: User Story 4 requires the creation summary to name the exact start command; a single
  additive row is the smallest change satisfying SC-004 with no behavior change to creation.

No unresolved technical questions remain for Phase 1 design.
