# CLI Contract: `microvm stop`

## Command surface

```text
microvm stop [NAME] [OPTIONS]

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
selection or stop work.

## Input modes

### Interactive mode

When the terminal supports interactive input and no name was supplied, the
command first passes the privilege gate (prompt-driven escalation when not
already privileged, exactly like `ssh`), then fetches `list_microvms()` and
shows one single-select machine list (running names only, keyboard
navigation, confirmation, cancellation) using the shared prompt render
configuration with ssh-identical behavior. When a name was supplied (either
form), the selector is skipped.

Invalid names abort immediately with the naming rule restated; there is no
re-prompt loop. A positional name and `--name` that disagree abort before
anything stops. Escape or an empty selection cancels with no side effects. An
empty running set exits with a nothing-running error pointing at
`microvm start`. There is no stop confirmation: elevation (when needed) is
the only gate before the stop.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm stop web-01 --non-interactive
```

Explicit mode must satisfy all of these before any stop attempt:

- a name is present (positional or `--name`) and path-friendly;
- the process already runs with root access (elevation is never
  prompted here).

The command performs no prompts at all in `--non-interactive` mode and reports
the first missing requirement with usage guidance.

## Machine resolution

An explicit name is validated locally and passed **directly** to
`stop_microvm(name)` — never pre-resolved against a running listing:

- SDK success → the stopped report with the returned forcing outcome;
- SDK `NotFound` → unknown-machine error pointing to `microvm new`, no
  creation shortcut;
- SDK success on an already-stopped machine → stopped report with no forcing
  used (idempotent success, never a not-running error);
- any other SDK error → calm `stop_failed` triplet naming the operation.

The selector path resolves to a running name first, then follows the same
single SDK call. No second pre-flight SDK lookup exists on either path.

## Privilege escalation

Stopping a machine touches host networking state and the SDK inventory below
the SDK home, which requires root access. The command checks the effective
user ID through the unchanged `privilege.rs` flow at two gates:

- already privileged: proceeds with no change;
- interactive without rights: re-executes itself elevated (`sudo` preferred,
  `pkexec` fallback) — bare path as `stop`, resolved path as
  `stop <name> --non-interactive` — with `TAUMARU_HOME`, a
  `TAUMARU_ESCALATED=1` guard, and no trusted-values envelope (the child
  re-resolves the name through the SDK), then exits with the child's status
  (signal death maps to `130`);
- non-interactive without rights, or no TTY: fails with an elevated-rights
  error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

The second (pre-call) gate is mandatory: the SDK stop call itself always runs
privileged.

## Stop execution

After elevation, the command starts a `StopSpinner` (`Stopping MicroVM
{name}`; non-interactive stderr line; no invented progress values), awaits
the single `sdk.stop_microvm(name)`, finishes the spinner, and renders the
stopped report: the machine name, a text-first graceful/forced shutdown line,
and the `microvm start {name}` next step. No key paths, socket paths, or
process identities are printed. The CLI implements no socket messaging,
process signaling, or state persistence.

## Error contract

Every expected failure uses the calm what/why/next triplet with a
nonzero status:

| Situation | Message shape |
|---|---|
| Name and `--name` disagree | Mismatch names both values; next is to pass one name. |
| Invalid name | Restates the 1–64 ASCII path-friendly rule; no re-prompt, nothing stopped. |
| Missing name (`--non-interactive`) | Missing-value form with `--name <NAME>` and a full example. |
| Missing rights (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Empty running set (selector) | Nothing running; next is `microvm start`. |
| Unknown machine | Names the machine; next points to `microvm new`; no creation shortcut offered. |
| Lifecycle conflict (e.g. incomplete creation) | Names the machine and its state; next is the repair action from the SDK error. |
| Stop-operation failure | Names the operation and carries the SDK cause; next is to check the reported cause and retry. |
| Selector/elevation cancelled or interrupted | Stop-specific cancellation; nothing stopped; exit `130`. |
| Home unavailable | Affected path explained; stops before selection. |

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The machine is stopped (graceful, forced, or already-stopped idempotent success). |
| `1` | Validation, privilege, lookup, lifecycle, stop-operation, or home failure; the machine was not stopped by this invocation. |
| `130` | The operator cancelled the selector or elevation, or an interrupt arrived during selector/elevation. |

## Out of scope

- the `microvm ssh`, `list`, `status`, and `inspect` commands (referenced only as pointers);
- kernel override, custom volume path, explicit LAN address, connection-hint rendering;
- registry selection, resumable transfers, runtime version selection;
- any new SDK operation or change to the existing `stop_microvm` contract;
- opening or changing the SDK's SQLite schema from the CLI.
