# CLI Contract: `microvm ssh`

## Command surface

```text
microvm ssh [NAME] [-- COMMAND...] [OPTIONS]

Positional:
    [NAME]                  Machine name; skips the machine selector when supplied.
    [COMMAND]...            Remote command to run inside the guest instead of a shell.
                            Everything after `--` is always remote command.

Options:
    --name <NAME>           Equivalent to the positional name; must agree when both given.
    --non-interactive       Disable prompts; the name is required and rights are required up front.
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
selection or session work.

## Input modes

### Interactive mode

When the terminal supports interactive input and no name was supplied, the
command fetches `list_running_microvms()` and shows one single-select machine
list (running names only, keyboard navigation, confirmation, cancellation)
using the shared prompt render configuration. When a name was supplied
(either form), the selector is skipped.

Invalid names abort immediately with the naming rule restated; there is no
re-prompt loop. A positional name and `--name` that disagree abort before
anything opens. Escape or an empty selection cancels with no side effects. An
empty running set exits with a nothing-running error pointing at
`microvm start`. There is no connection confirmation: elevation (when needed)
is the only gate before the session.

### Explicit mode

Valid in a terminal or a non-TTY environment:

```text
microvm ssh web-01 --non-interactive -- uname -a
```

Explicit mode must satisfy all of these before any session:

- a name is present (positional or `--name`) and path-friendly;
- the process already runs with sufficient rights (elevation is never
  prompted here).

The command performs no prompts at all in `--non-interactive` mode and reports
the first missing requirement with usage guidance.

## Machine resolution

An explicit name is resolved **only** against `list_running_microvms()`:

- entry found → that entry's `ssh` material is used to build the session;
- entry missing → `list_microvms()` is consulted once purely to label the
  error: name absent there means unknown (point to `microvm new`, no creation
  shortcut); name present means known-but-not-running (report the persisted
  state, point to `microvm start {name}`).

The all-machines inventory never decides liveness.

## Privilege escalation

Opening a session reads key material below the SDK home, which requires
elevated rights. After name resolution and before the session, the command
checks the effective user ID through the unchanged `privilege.rs` flow:

- already privileged: proceeds with no change;
- interactive without rights: re-executes itself elevated (`sudo` preferred,
  `pkexec` fallback) as `ssh <name> --non-interactive [-- <remote...>]` with
  `TAUMARU_HOME`, a `TAUMARU_ESCALATED=1` guard, and no trusted-values
  envelope (the child re-resolves the name through the SDK listing), then
  exits with the child's status (signal death maps to `130`);
- non-interactive without rights, or no TTY: fails with an elevated-rights
  error and no prompt;
- neither `sudo` nor `pkexec` available: fails with an actionable error;
- the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates.

## Session spawn

After elevation, the command locates the `ssh` binary with the existing
`backend_path("ssh")` lookup (`/usr/bin`, `/usr/sbin`, then `PATH`); absence
is a guided error, never a panic. It pre-flights only the chosen entry (the
private-key path must be a readable regular file) and then spawns:

```text
ssh -i {private_key_path} [-p {port} when != 22] {user}@{address} [-- remote...]
```

No host-key options are ever added: verification keeps default OpenSSH
behavior (first connect prompts, accepted keys are remembered). Standard
input, output, and error are inherited, so typing, echoed and full-screen
output, terminal resizing, and signals behave exactly as in a manual `ssh`
session. The parent installs no Ctrl-C interception around the session:
interrupts inside the session belong to the remote side. The SSH child writes
its own diagnostics to the inherited stderr; the CLI does not re-wrap them.

On success the CLI prints nothing, keeping piped scripted use byte-clean.

## Error contract

Every expected pre-session failure uses the calm what/why/next triplet with a
nonzero status:

| Situation | Message shape |
|---|---|
| Name and `--name` disagree | Mismatch names both values; next is to pass one name. |
| Invalid name | Restates the 1–64 ASCII path-friendly rule; no re-prompt, nothing opened. |
| Missing name (`--non-interactive`) | Missing-value form with `--name <NAME>` and a full example. |
| Missing rights (`--non-interactive`) | Elevated-rights error with the rerun-as-root hint; no prompt. |
| Empty running set (selector) | Nothing running; next is `microvm start`. |
| Unknown machine | Names the machine; next points to `microvm new`; no creation shortcut offered. |
| Known but not running | Names the machine and its persisted state; next is `microvm start {name}`. |
| Chosen key missing/unreadable | Names the affected key path; next is to repair or recreate. |
| `ssh` binary unavailable | Explains the missing program; next is to install OpenSSH client. |
| Selector/elevation cancelled or interrupted | Ssh-specific cancellation; no session opened; exit `130`. |
| Home unavailable | Affected path explained; stops before selection. |

Connection-level failures after spawn (refused, timeout, host-key rejection)
surface through the inherited `ssh` output and exit status, exactly as in a
manual invocation. No SDK debug output, panic, or stack trace is emitted by
the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The session ended with a zero status (shell or remote command). |
| `N` | The session ended with a nonzero remote status `N`; the CLI exits with the same status. |
| `1` | Validation, privilege, lookup, key, `ssh`-binary, or home failure; this invocation opened no session. |
| `130` | The operator cancelled the selector or elevation, an interrupt arrived before the session, or the session died by signal. |

## Out of scope

- the `microvm stop`, `list`, `status`, and `inspect` commands (referenced only as pointers);
- kernel override, custom volume path, explicit LAN address, connection-hint rendering;
- registry selection, resumable transfers, runtime version selection;
- any SDK operation beyond the one additive running-machines listing;
- opening or changing the SDK's SQLite schema from the CLI.
