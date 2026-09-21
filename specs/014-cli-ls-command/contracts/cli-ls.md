# CLI Contract: `microvm ls` / `microvm list`

## Command surface

```text
microvm ls [OPTIONS]
microvm list [OPTIONS]        # visible_alias: identical behavior and output

Options:
    --non-interactive       Disable prompts; root is required up front.
```

No positional `[NAME]`, no `--name`, no `--image`, `--disk-gb`,
`--memory`, `--vcpus`, `--expose-lan`, `--kernel`, `--volume-path`, or
`--lan-address` flags exist. A positional or lifecycle flag surfaces as a
clap error. No home-directory flag exists: home resolution follows the
existing policy only. `ls` and `list` are one `Command::Ls` variant —
the alias cannot diverge by construction — and `list` is advertised in
help via `visible_alias`.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database directly and does not
create a second artifact directory layout. Home resolution errors are
returned before privilege or listing work.

## Input modes

### Interactive mode

The command takes no input: no selector, no prompts beyond the elevation
gate. Once privileged (directly or after the escalated child returns),
it lists and renders. Piped runs (`microvm ls | head`) render the same
table to stdout once privileged — no TTY is required for the listing
itself.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm ls --non-interactive
```

Explicit mode must satisfy this before any listing attempt:

- the process already runs with root access (elevation is never
  prompted here).

The command performs no prompts at all in `--non-interactive` mode and
reports missing rights with usage guidance.

## Privilege escalation

Listing touches host-local runtime state (socket liveness probes) below
the SDK home, which requires root access. The command checks the
effective user ID through the unchanged `privilege.rs` flow at a single
pre-listing gate:

- already privileged: proceeds with no change;
- interactive without rights: re-executes itself elevated (`sudo`
  preferred, `pkexec` fallback) as bare `ls` — with `TAUMARU_HOME`, a
  `TAUMARU_ESCALATED=1` guard, and no trusted-values envelope (the child
  re-lists through the SDK) — then exits with the child's status
  (signal death maps to `130`);
- non-interactive without rights, or no TTY: fails with an
  elevated-rights error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

There is no second gate: unlike `stop`/`start`, there is no machine name
to carry into a re-execution argv.

## Ls execution

After elevation, the command awaits the single `sdk.list_microvms()`
and renders: empty result → the calm stdout empty report (exit `0`);
otherwise the compact table (exit `0`). The CLI implements no socket
probing, process inspection, or state persistence. No key paths, socket
paths, or process identities are printed.

## Table contract

One header plus one row per machine in SDK (name) order; column widths
from content maxima; no truncation; identical bytes on every terminal
shape:

```text
NAME  STATE  VCPUS  MEMORY  DISK  IMAGE  NETWORK
```

Cell rules: `STATE` is the `MicroVmState` display text with semantic
color (running green, stopped dim; text carries the meaning);
`MEMORY` via `format_mb_gb`, `DISK` via `format_gb` (configured values
only); `IMAGE` as `{distribution_id}={image_id}` (the `--image` token
form); `NETWORK` as `host-only {guest}` / `lan {lan} (guest {guest})` /
`lan (guest {guest})` / `-`. Any missing capacity or network value
renders `-`; the degraded row stays in place. SSH user, port, and key
paths never appear.

## Error contract

Every expected failure uses the calm what/why/next triplet with a
nonzero status:

| Situation | Message shape |
|---|---|
| Positional or lifecycle flag supplied | Clap usage error (no such operand). |
| Missing rights (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Missing rights (no TTY) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Listing-operation failure | `ls_failed` triplet naming the operation with the SDK cause; next is to check the reported cause and retry. |
| Elevation declined or interrupted | Cancellation inherited from the privilege flow; nothing listed; exit `130`. |
| Home unavailable | Affected path explained; fails before privilege work. |

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The listing rendered (table) or the inventory is empty (empty report). |
| `1` | Validation, privilege, listing-operation, or home failure; nothing was listed. |
| `130` | The operator declined elevation or an interrupt arrived during elevation. |

## Out of scope

- the `microvm ssh`, `status`, and `inspect` commands (referenced only as pointers);
- per-machine drill-down, filtering, sorting, or output-format flags;
- measured disk usage, guest-OS introspection, registry selection;
- any new SDK operation or change to existing SDK behavior, lifecycle semantics, error contracts, or persistence layout;
- opening or changing the SDK's SQLite schema from the CLI.
