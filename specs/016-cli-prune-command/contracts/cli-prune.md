# CLI Contract: `microvm artifacts prune`

## Command surface

```text
microvm artifacts prune [OPTIONS]

Options:
    --non-interactive       Disable prompts; root is required up front.
```

No positional arguments and no selection flags exist. No home-directory
flag exists: home resolution follows the existing policy only
(`TAUMARU_HOME`, else `~/.taumaru-microvm`), passed explicitly to the SDK.
Unknown flags and positionals are rejected by the parser with the standard
usage error.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database directly and does not
create a second artifact directory layout. Home resolution errors are
returned before elevation or preview work.

## Input modes

### Interactive mode

```text
microvm artifacts prune
```

When already privileged, the command derives the preview directly. When
not, it first passes the privilege gate (prompt-driven escalation when the
terminal supports it, exactly like the sibling commands), then derives the
preview from read-only SDK queries: downloaded kernel/image identities
minus the identities referenced by `list_microvms()` rows. When the preview
is empty, the command reports nothing to prune with success status and
shows no confirmation. Otherwise it shows the preview — every kernel and
image identity plus the estimated freed space — and asks one explicit
confirmation (defaulting to no, same copy pattern as the sibling
review-then-confirm steps). Declining or interrupting cancels with no
deletions. On confirmation the command makes the single
`prune_unused_artifacts()` call and renders the authoritative result.

The preview is an estimate: inventory may change between preview and
deletion, and the rendered `PruneSummary` is always authoritative.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm artifacts prune --non-interactive
```

Explicit mode deletes without preview or confirmation after the root gate.
The process must already run with root access (elevation is never prompted
here). The command performs no prompts at all in `--non-interactive` mode
and reports the first missing requirement with usage guidance.

## Privilege escalation

Pruning deletes artifact files and mutates the SDK inventory below the SDK
home, which requires root access. The command checks the effective user ID
through the unchanged `privilege.rs` flow with one gate:

- already privileged: proceeds with no change;
- interactive without rights: re-executes itself elevated (`sudo`
  preferred, `pkexec` fallback) as `artifacts prune --non-interactive`
  with `TAUMARU_HOME`, a `TAUMARU_ESCALATED=1` guard, then exits with the
  child's status (signal death maps to `130`);
- non-interactive without rights, or no TTY: fails with an elevated-rights
  error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

The SDK prune call itself always runs privileged.

## Prune execution

After elevation (and interactive confirmation when candidates exist), the
command awaits the single `sdk.prune_unused_artifacts()` and renders:

- on success: the removed kernels and images with counts per kind, the
  freed space per kind and in total (shared binary-unit helper), and the
  skipped group when non-empty; nothing-to-prune renders the calm report
  instead of an empty removal list;
- on `PruneIncomplete`: the removed set with freed space so far, the
  skipped group, each failed artifact key with its cause, and a
  repair-and-retry next step.

No key paths, socket paths, digests, or process identities are printed.
The CLI implements no candidacy, reference, deletion, or accounting logic.

## Error contract

Every expected failure uses the calm what/why/next triplet with a nonzero
status, except cancellations which use the cancelled shape:

| Situation | Message shape |
|---|---|
| Missing rights (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Declined preview or interrupted confirmation | Prune-specific cancellation; nothing deleted; exit `130`. |
| Declined elevation or interrupted escalation | Cancellation through the existing flow; nothing deleted; exit `130`. |
| Nothing to prune | Calm report, not an error; success status. |
| Partial failure (`PruneIncomplete`) | Removed set with freed space so far, skipped group, each failed key with cause; next is the repair-and-retry step; exit `1`. |
| Prune-operation failure (other SDK errors) | Names the operation and carries the SDK cause; next is to check the reported cause and retry; exit `1`. |
| Home unavailable | Affected path explained; stops before elevation. |

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | Prune succeeded (including nothing-to-prune). |
| `1` | Validation, privilege, home, prune-operation, or partial failure; some or all candidates were not pruned by this invocation. |
| `130` | The operator cancelled elevation or the deletion confirmation, or an interrupt arrived during either. |

## Out of scope

- the `microvm artifacts download` flow (referenced only as a group sibling);
- per-artifact selection, dry-run previews without deletion, and filtering or sorting flags;
- creation, start, stop, `ssh`, `ls`, `status`, and `inspect` commands (referenced only as pointers);
- any new SDK operation or change to the existing prune contract;
- opening or changing the SDK's SQLite schema from the CLI.
