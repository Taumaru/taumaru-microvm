# CLI Contract: `microvm start`

## Command surface

```text
microvm start [NAME] [OPTIONS]

Positional:
    [NAME]                  Machine name; skips the machine selector when supplied.

Options:
    --name <NAME>           Equivalent to the positional name; must agree when both given.
    --non-interactive       Disable prompts; the name is required and root is required up front.
```

No `--image`, `--disk-gb`, `--memory`, `--vcpus`, `--expose-lan`, `--kernel`, `--volume-path`,
or `--lan-address` flags exist. No home-directory flag exists: home resolution follows the
existing policy only.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database directly and does not create a second artifact
directory layout. Home resolution errors are returned before selection or start.

## Input modes

### Interactive mode

When the terminal supports interactive input and no name was supplied, the command fetches
`list_microvms()` and shows one single-select machine list (names in persisted order, keyboard
navigation, confirmation, cancellation) using the shared prompt render configuration. When a name
was supplied (either form), the selector is skipped.

Invalid names abort immediately with the naming rule restated; there is no re-prompt loop. A
positional name and `--name` that disagree abort before anything starts. Escape or an empty
selection cancels with no side effects. An empty local inventory exits with a nothing-to-start
error pointing at `microvm new`. There is no start confirmation: elevation (when needed) is the
only gate before the SDK call.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm start web-01 --non-interactive
```

Explicit mode must satisfy all of these before any mutation:

- a name is present (positional or `--name`) and path-friendly;
- the process already runs with root access (elevation is never prompted here).

The command performs no prompts at all in `--non-interactive` mode and reports the first missing
requirement with usage guidance.

## Privilege escalation

Starting a machine configures host networking and launches the machine process, which requires
root. After name resolution and before the SDK call, the command checks the effective user ID
through the unchanged `privilege.rs` flow:

- already root: proceeds with no change;
- interactive without root: re-executes itself elevated (`sudo` preferred, `pkexec` fallback) as
  `start <name> --non-interactive` with `TAUMARU_HOME`, a `TAUMARU_ESCALATED=1` guard, and no
  trusted-values envelope (the SDK start takes only the name), then exits with the child's status
  (signal death maps to `130`);
- non-interactive without root, or no TTY: fails with an elevated-rights error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

The direct-ssh hint carries the elevation prefix actually used (`sudo `, `pkexec `, or empty when
already root) so the copied command works as shown.

## SDK call order

After elevation, the command makes exactly one SDK call:

1. `list_microvms()` once — only on the interactive no-name path, to populate the selector;
2. `start_microvm(name)` once — with the resolved name only.

The CLI performs no registry access, artifact work, checksum, cache, SQLite, process, or network
handling. Launch, repair, idempotency, and persistence are the SDK's existing behavior.

## Progress and output

While the SDK call is in flight the command shows a spinner (`Starting MicroVM {name}`) reusing
the catalog-spinner pattern; non-interactive terminals get the `·`-prefixed stderr line. The
spinner carries no progress values and is cleared before the result. The final running report goes
to stdout:

```text
✓ MicroVM web-01 running

  Connect: microvm ssh web-01
  Direct:  sudo ssh -i ~/.taumaru-microvm/vms/web-01/ssh/id_ed25519 -p 22 root@192.168.127.2
  LAN:     copy ~/.taumaru-microvm/vms/web-01/ssh/id_ed25519 to the other machine, then
           ssh -i id_ed25519 -p 22 root@192.168.10.30
  Stop:    microvm stop web-01
```

The LAN paragraph appears only for LAN-exposed results and names the committed
`network.lan_address` as the remote address. Host-only results omit it. Key paths are shown;
key contents are never printed. Already-running results render the identical layout. Non-TTY
output is line-oriented and deterministic; color loss never removes text markers or hint order
(connect hints first, `microvm stop` last). The `microvm ssh` and `microvm stop` references are
documentation-only hints.

The `new` command's summary keeps all existing rows and appends:

```text
  Start:   microvm start web-01
```

## Error contract

Every expected failure uses the calm what/why/next triplet with a nonzero status:

| Situation | Message shape |
|---|---|
| Name and `--name` disagree | Mismatch names both values; next is to pass one name. |
| Invalid name | Restates the 1–64 ASCII path-friendly rule; no re-prompt, nothing started. |
| Missing name (`--non-interactive`) | Missing-value form with `--name <NAME>` and a full example. |
| Missing root (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Empty inventory (selector) | Nothing to start; next is `microvm new`. |
| Unknown machine | Names the machine; next points to `microvm new`; no creation shortcut offered. |
| Lifecycle conflict (`Creating` row) | Names the machine and state; next is to wait or recreate. |
| Missing prerequisites / unrepairable network / launch failure | SDK message as the cause; next is the concrete retry or repair action; the VM stays stopped. |
| Selector/elevation cancelled or interrupted | Start-specific cancellation; no machine started; exit `130`. |
| Home unavailable | Affected path explained; stops before selection. |

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The machine is running (freshly started or already running) and the report was printed. |
| `1` | Validation, privilege, lookup, prerequisite, network, or launch failure; this invocation started nothing new. |
| `130` | The operator cancelled the selector or elevation, or an interrupt arrived first. |

## Out of scope

- the `microvm ssh` and `microvm stop` commands (hints only);
- kernel override, custom volume path, explicit LAN address;
- a general `list` command (the new SDK listing serves the selector for now);
- opening or changing the SDK's SQLite schema from the CLI;
- registry selection, resumable transfers, runtime version selection.
