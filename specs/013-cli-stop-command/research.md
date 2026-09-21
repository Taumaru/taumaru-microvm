# Phase 0 Research: CLI `stop` Command for Stopping a MicroVM

## Research goals

Resolve how the `stop` command delivers its spec: which SDK source feeds the
running-only selector, how the named path reaches the SDK without duplicating
resolution, how elevation reuses the existing privilege flow while guaranteeing
the stop call always runs privileged, and how progress and the
graceful-versus-forced outcome are presented — all without a second lifecycle
engine, new dependencies, schema changes, or new SDK operations.

## Findings

### Selector source: `list_microvms()` filtered to `Running`, exactly like `ssh`

- Decision: the interactive selector calls the existing
  `MicroVmSdk::list_microvms()` and keeps only entries whose call-time verified
  state is `MicroVmState::Running`, presenting them in an `inquire::Select`
  with the shared `prompt_render_config` — the same call, filter, and prompt
  shape the `ssh` selector uses (`commands/ssh.rs` resolves its selector list
  this way, not through the running-machines listing).
- Rationale: `list_microvms` already returns call-time verified state
  (`Running` iff the volume-local socket answers), so the filter is live truth,
  not a stale snapshot. Stop needs names only; the `RunningMicroVm` SSH
  material would be dead weight. Reusing the `ssh` selector's exact source
  satisfies FR-002's "behaving exactly like" requirement by construction.
- Alternatives considered: `list_running_microvms()` (rejected — carries
  unneeded SSH connection material; even the `ssh` selector itself does not
  use it for listing); a new SDK listing (rejected — no new SDK surface is
  needed for a CLI-only feature, and the spec mandates reusing the SDK stop
  operation unchanged).

### Named path: validate locally, call `stop_microvm` directly, no pre-resolution

- Decision: an explicitly named machine is validated with the shared
  `resolve_name`/`validate_name` rule and passed straight to the existing
  `sdk.stop_microvm(name)`. There is no running-listing lookup first.
  `SdkError::NotFound` maps to the `stop_not_found` triplet (points to
  `microvm new`); any other SDK error maps through `stop_failed`.
- Rationale: unlike `ssh`, stop must accept already-stopped names (idempotent
  success), so resolving against a running-only source first would wrongly
  reject them. The SDK already distinguishes unknown (typed `NotFound`) from
  stopped (success), so a pre-resolution listing would be a second call that
  adds a race without adding information.
- Alternatives considered: ssh-style resolve-against-running-then-label
  (rejected — contradicts FR-006 and doubles SDK calls on the success path).

### Elevation: mirror `start`, child carries the resolved name

- Decision: reuse `privilege.rs` unchanged with the `start`/`ssh`
  parent-validates/child-executes pattern. Non-interactive mode calls
  `require_privileged(..., non_interactive=true, ..., Vec::new(), retry_hint)`
  so missing rights fail fast with no prompt. The bare interactive path
  escalates first with child argv `["stop"]` (so listing and selection happen
  privileged), and after resolution every path passes the gate again with
  `escalated_child_command(name)` = `["stop", name, "--non-interactive"]`
  before the SDK call. The escalated child (`TAUMARU_ESCALATED=1`) skips
  escalation and runs the stop directly.
- Rationale: the double gate (pre-selector plus pre-call) is what guarantees
  FR-004 — the SDK call itself always runs privileged — while keeping the
  single existing escalation mechanism. No trusted-values envelope crosses the
  boundary: the child re-resolves the validated name through the SDK, exactly
  like `ssh` (which likewise passes only the name, not key material).
- Alternatives considered: a single pre-call gate only (rejected — the bare
  path would list and prompt unprivileged, diverging from the `ssh` behavior
  the spec mandates); passing stop outcome hints over the boundary (rejected —
  nothing needs to cross; the child observes the SDK result itself).

### Progress and outcome: `StopSpinner` plus a graceful/forced report

- Decision: add a `StopSpinner` in `output/human.rs` mirroring `StartSpinner`
  (spinner message `Stopping MicroVM {name}`, non-interactive stderr line, no
  invented progress values). On success render `format_stop_result` with the
  stopped title, an explicit shutdown line distinguishing graceful (guest
  exited on its own) from forced (SIGKILL was delivered) in text alone, and a
  `microvm start {name}` next step — mirroring `format_start_result`'s
  title/rule/detail structure.
- Rationale: the SDK wait can last up to 60 seconds plus the forced re-verify,
  so a silent stop looks hung; the `start` flow already proves the accepted
  spinner pattern. The forcing outcome must survive non-color terminals and
  pipes, hence a text line rather than color or symbol alone.
- Alternatives considered: reusing `StartSpinner` verbatim (rejected — the
  message names the wrong operation); silent success like `ssh` (rejected —
  `ssh` hands off to a live session that speaks for itself, while stop's only
  evidence is its report).

### Errors: additive `stop_*` constructors, cancellation exits `130`

- Decision: add `stop_not_found` (names the machine, points to `microvm new`),
  `stop_empty` (nothing running, points to `microvm start`),
  `stop_cancelled` (selector/elevation cancelled, exits `130` via a new
  `exit_code` prefix arm mirroring the start/connection arms), and
  `stop_failed(what, &SdkError)` mirroring `start_failed`. Missing-name and
  missing-rights non-interactive failures reuse the existing `missing_value`
  and `privileged` shapes; the non-interactive-no-TTY case reuses the generic
  `creation` triplet with the `microvm stop <NAME> --non-interactive` pointer.
- Rationale: FR-010/F R-012 require calm triplets without rewording existing
  messages; the additive-constructor pattern is exactly how `start` and `ssh`
  extended `error.rs`. The `130` arm keeps Ctrl-C/cancel semantics consistent
  across commands.
- Alternatives considered: reusing `ssh_*` constructors (rejected — they name
  the wrong operation and point at the wrong recovery commands).

No unresolved technical questions remain for Phase 1 design.
