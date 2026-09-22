# CLI Contract: `microvm delete`

## Command surface

```text
microvm delete [NAME] [OPTIONS]

Positional:
    [NAME]                  Machine name; skips the machine selector when supplied.

Options:
    --name <NAME>           Equivalent to the positional name; must agree when both given.
    --non-interactive       Disable prompts; the name is required and root is required up front.
```

No `--image`, `--disk-gb`, `--memory`, `--vcpus`, `--expose-lan`, `--kernel`,
`--volume-path`, or `--lan-address` flags exist. No home-directory flag
exists: home resolution follows the existing policy only.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database directly and does not create
a second artifact directory layout. Home resolution errors are returned before
selection, confirmation, or delete work.

## Input modes

### Interactive mode

When the terminal supports interactive input and no name was supplied, the
command first passes the privilege gate (prompt-driven escalation when not
already privileged, exactly like `stop`/`ssh`), then fetches `list_microvms()` and
shows one single-select machine list (every stored machine in any state, each
labeled `name [state]`, keyboard navigation, confirmation, cancellation)
using the shared prompt render configuration with ssh-identical behavior.
When a name was supplied (either form), the selector is skipped.

Invalid names abort immediately with the naming rule restated; there is no
re-prompt loop. A positional name and `--name` that disagree abort before
anything is deleted. Escape or an empty selection cancels with no side effects. An
empty stored set exits with a nothing-to-delete error pointing at
`microvm new`.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm delete web-01 --non-interactive
```

Explicit mode must satisfy all of these before any deletion attempt:

- a name is present (positional or `--name`) and path-friendly;
- the process already runs with root access (elevation is never
  prompted here).

The command performs no prompts at all in `--non-interactive` mode — no
selector, no confirmation — and reports the first missing requirement with
usage guidance.

## Machine resolution

An explicit name is validated locally and passed **directly** to
`delete_microvm(name)` — never pre-resolved against any listing:

- SDK success → the deleted report naming the machine;
- SDK `NotFound` → unknown-machine error pointing to `microvm new`, no
  creation shortcut;
- SDK running `LifecycleConflict` → stop-first refusal naming the machine
  with the exact `microvm stop {name}` next step, nothing deleted;
- SDK retryable failure → calm `delete_failed` triplet naming the cause with
  the fix-and-retry next step, nothing reported as deleted;
- any other SDK error → calm `delete_failed` triplet naming the operation.

The selector path resolves to a stored name first (in any state), then follows
the same single SDK call with the same mapping — including the running
refusal. No second pre-flight SDK lookup exists on either path.

## Privilege escalation

Deleting a machine removes its volume directory, releases host network state,
and mutates the SDK inventory below the SDK home, which requires root access.
The command checks the effective user ID through the unchanged `privilege.rs`
flow at two gates:

- already privileged: proceeds with no change;
- interactive without rights: re-executes itself elevated (`sudo` preferred,
  `pkexec` fallback) — bare path as `delete`, resolved path as
  `delete <name> --non-interactive` — with `TAUMARU_HOME`, a
  `TAUMARU_ESCALATED=1` guard, and no trusted-values envelope (the child
  re-resolves the name through the SDK), then exits with the child's status
  (signal death maps to `130`);
- non-interactive without rights, or no TTY: fails with an elevated-rights
  error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

The second (pre-call) gate is mandatory: the SDK delete call itself always runs
privileged.

## Delete confirmation

After elevation and before the SDK call, interactive mode shows one explicit
confirmation naming the resolved machine with a permanent-loss warning
(defaulting to decline). Approval is required to proceed; declining or
interrupting cancels with no deletions and a cancelled report (exit `130`).
Non-interactive mode skips the confirmation entirely.

## Delete execution

After confirmation (or directly in non-interactive mode), the command starts a
`DeleteSpinner` (`Deleting MicroVM {name}`; non-interactive stderr line; no
invented progress values), awaits the single `sdk.delete_microvm(name)`,
finishes the spinner, and renders the deleted report: the machine name, a
text-first removed line (record, volume, owned network attachment), a
preserved line (shared kernels and images), and the `microvm new` next step.
No key contents, socket paths, or process identities are printed. The CLI
implements no file removal, network release, or state persistence.

## Error contract

Every expected failure uses the calm what/why/next triplet with a
nonzero status:

| Situation | Message shape |
|---|---|
| Name and `--name` disagree | Mismatch names both values; next is to pass one name. |
| Invalid name | Restates the 1–64 ASCII path-friendly rule; no re-prompt, nothing deleted. |
| Missing name (`--non-interactive`) | Missing-value form with `--name <NAME>` and a full example. |
| Missing rights (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Empty stored set (selector) | Nothing to delete; next is `microvm new`. |
| Unknown machine | Names the machine; next points to `microvm new`; no creation shortcut offered. |
| Running machine | Names the machine, states the stop-first requirement; next is the exact `microvm stop {name}` command; nothing deleted. |
| Retryable delete-operation failure | Names the machine and carries the SDK cause; next is to fix the cause and retry the same delete; nothing reported as deleted. |
| Other delete-operation failure | Names the operation and carries the SDK cause; next is to check the reported cause and retry. |
| Confirmation declined or selector/elevation/confirmation cancelled or interrupted | Delete-specific cancellation; nothing deleted; exit `130`. |
| Home unavailable | Affected path explained; stops before selection. |

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The machine is deleted and reported. |
| `1` | Validation, privilege, lookup, lifecycle, delete-operation, confirmation-decline, or home failure; the machine was not deleted by this invocation. |
| `130` | The operator cancelled the selector, elevation, or confirmation, or an interrupt arrived during selector/elevation/confirmation. |

## Out of scope

- the `microvm ssh`, `list`, `status`, and `inspect` commands (referenced only as pointers);
- kernel override, custom volume path, explicit LAN address, connection-hint rendering;
- registry selection, resumable transfers, runtime version selection;
- any new SDK operation or change to the existing `delete_microvm` contract;
- opening or changing the SDK's SQLite schema from the CLI.
