# Phase 1 Data Model: CLI `stop` Command for Stopping a MicroVM

## CLI input

### Stop request

One machine name to stop, however supplied:

| Source | Rule |
|---|---|
| Positional `[NAME]` | Skips the selector when present. |
| `--name <NAME>` | Equivalent to the positional; both present MUST agree or the command reports the mismatch and stops nothing. |
| Interactive selector | Used only when no name is present and the terminal is interactive; offers running machines only. |
| `--non-interactive` | Name is required (either form); missing name fails with usage guidance before any stop attempt. No prompts of any kind. |

Validation (shared with `new`/`start`/`ssh`, abort-no-reprompt): 1–64 ASCII
characters, starts alphanumeric, remaining alphanumeric/`-`/`_`. Invalid names
abort with the naming rule restated before any stop attempt.

## Records read, never written by the CLI

The CLI writes nothing to SQLite and adds no SDK operation. Two existing SDK
reads serve the two paths:

| Path | SDK source | Shape used |
|---|---|---|
| Interactive selector list | `list_microvms()` filtered to `state == MicroVmState::Running` | Names plus verified state, ordered by name; same call and filter the `ssh` selector uses. |
| Named stop | `stop_microvm(name)` directly | Unknown names surface as typed `NotFound`; already-stopped names succeed idempotently. |

The CLI never inspects inventory records directly. All lifecycle behavior —
socket liveness, graceful request, 60-second wait, SIGKILL escalation,
stopped-state persistence — stays inside the existing `stop_microvm`
operation, whose `MicroVmStopResult { name, state, socket_path, forced }`
contract is unchanged by this feature.

## CLI output

### Stop report (from the returned `MicroVmStopResult`)

Rendered by `format_stop_result`, mirroring `format_start_result`'s
title/rule/detail structure (leading blank line, `✓` check plus bold title,
dim divider, dim row labels):

```text
✓ MicroVM {name} stopped
────────────────────────────────────────────────
  Shutdown:  graceful — the guest exited on its own | forced — the guest did not exit and was force-terminated
  Start:     microvm start {name}
```

The shutdown line is text-first (never color or symbol alone) so the
graceful/forced distinction survives non-color and narrow terminals. Key
paths, socket paths, and process identities never appear in the report.

### Failure pointers (documentation references, not implemented commands)

| Pointer | Shown when |
|---|---|
| `microvm new` | Named machine is unknown. No creation shortcut is offered. |
| `microvm start` | The running set is empty (bare path); the stopped machine should run again (success next step). |

## State and flow ownership

```text
CLI (this feature)                SDK (existing, unchanged)
──────────────────                ─────────────────────────
resolve name ──► escalate ──► spinner ──► stop_microvm(name)
(interactive     (privilege.rs,    while in        ▲
 selector over    unchanged;        flight          │ socket/shutdown/wait/
 running list,    double gate:      │               │ SIGKILL/persist,
 no-name ⇒ list; pre-selector +     └───────────────┘ no new operations
 pre-call)       report
```

CLI transitions: name unresolved → resolved → elevated (or already
privileged) → stopping (spinner) → stopped report or typed failure. No
CLI-owned VM state, no retry loop, no confirmation gate: elevation is the only
gate before the stop. Interrupt during selector/elevation → cancelled, exit
`130`, nothing stopped. The terminal never transfers to another program; the
CLI owns its output for the whole invocation.
