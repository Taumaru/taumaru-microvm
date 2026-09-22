# Phase 1 Data Model: CLI `delete` Command for Deleting a MicroVM

## CLI input

### Delete request

One machine name to delete, however supplied:

| Source | Rule |
|---|---|
| Positional `[NAME]` | Skips the selector when present. |
| `--name <NAME>` | Equivalent to the positional; both present MUST agree or the command reports the mismatch and deletes nothing. |
| Interactive selector | Used only when no name is present and the terminal is interactive; offers all stored machines in any state with state labels. |
| `--non-interactive` | Name is required (either form); missing name fails with usage guidance before any deletion attempt. No prompts of any kind, including no confirmation. |

Validation (shared with `new`/`start`/`stop`/`ssh`, abort-no-reprompt): 1–64 ASCII
characters, starts alphanumeric, remaining alphanumeric/`-`/`_`. Invalid names
abort with the naming rule restated before any deletion attempt.

## Records read, never written by the CLI

The CLI writes nothing to SQLite and adds no SDK operation. Two existing SDK
reads serve the two paths:

| Path | SDK source | Shape used |
|---|---|---|
| Interactive selector list | `list_microvms()` unfiltered | Names plus verified state for every stored row, ordered by name; each option labeled `name [state]`. Running picks are allowed through to the SDK refusal. |
| Named delete | `delete_microvm(name)` directly | Unknown names surface as typed `NotFound`; running names surface as typed stop-first `LifecycleConflict`; retryable failures keep the record for retry. |

The CLI never inspects inventory records directly. All lifecycle behavior —
socket liveness, network release, whole-volume removal, record deletion with
retry convergence, absent-item convergence — stays inside the existing
`delete_microvm` operation, whose `MicroVmDeleteResult { name }` contract is
unchanged by this feature.

## CLI confirmation

### Deletion confirmation (interactive only)

One `inquire::Confirm` after elevation, before the SDK call:

| Field | Rule |
|---|---|
| Prompt | Names the resolved machine with a permanent-loss warning (`Delete {name}? This permanently removes its disk, credentials, and network attachment`). |
| Default | Decline (`false`); an empty answer cancels. |
| Decline or interrupt | `delete_cancelled`, exit `130`, nothing deleted. |
| Non-interactive | Skipped entirely; the explicit name plus the root gate carry the intent. |

## CLI output

### Delete report (from the returned `MicroVmDeleteResult`)

Rendered by `format_delete_result`, mirroring `format_stop_result`'s
title/rule/detail structure (leading blank line, `✓` check plus bold title,
dim divider, dim row labels):

```text
✓ MicroVM {name} deleted
────────────────────────────────────────────────
  Removed:    record, volume, and owned network attachment
  Preserved:  shared kernels and images
  Create:     microvm new
```

The removed-versus-preserved lines are text-first (never color or symbol
alone) so the outcome survives non-color and narrow terminals. Key contents,
socket paths, and process identities never appear in the report.

### Failure pointers (documentation references, not implemented commands)

| Pointer | Shown when |
|---|---|
| `microvm new` | Named machine is unknown; the stored set is empty; success next step. No creation shortcut is offered. |
| `microvm stop {name}` | The machine is currently running (exact command for that machine). Nothing is deleted. |
| Fix-and-retry | A retryable delete failure (owned network/volume/record); the cause plus retrying the same delete. |

## State and flow ownership

```text
CLI (this feature)                SDK (existing, unchanged)
──────────────────                ─────────────────────────
resolve name ──► escalate ──► confirm ──► spinner ──► delete_microvm(name)
(interactive     (privilege.rs,   (Confirm,         while in        ▲
 selector over    unchanged;       default            flight          │ liveness/network/
 all-stored       double gate:     decline;           │               │ files/record, no
 list, no-name    pre-selector +   non-interactive     └───────────────┘ new operations
 ⇒ list;         pre-call)        skips)             report
 pre-call)
```

CLI transitions: name unresolved → resolved → elevated (or already
privileged) → confirmed (or non-interactive skip) → deleting (spinner) →
deleted report or typed failure. No CLI-owned VM state, no retry loop: a
retryable failure exits nonzero and the operator reruns the same command.
Interrupt during selector/elevation/confirmation → cancelled, exit `130`,
nothing deleted. The terminal never transfers to another program; the CLI
owns its output for the whole invocation.
