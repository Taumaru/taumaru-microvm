# Phase 1 Data Model: CLI `ssh` Command for Connecting to a MicroVM

## CLI input

### SSH request

One machine name to connect to, however supplied, plus zero or more trailing
remote-command words:

| Source | Rule |
|---|---|
| Positional `[NAME]` | Skips the selector when present. |
| `--name <NAME>` | Equivalent to the positional; both present MUST agree or the command reports the mismatch and opens nothing. |
| Trailing `[COMMAND]...` | Remote words to run inside the guest instead of a shell. Everything after `--` is always remote command; without `--`, words after the name are remote command. Trailing words with no name (after `--`) run on the interactively selected machine. |
| Interactive selector | Used only when no name is present and the terminal is interactive. |
| `--non-interactive` | Name is required (either form); missing name fails with usage guidance before any session. No prompts of any kind. |

Validation (shared with `new`/`start`, abort-no-reprompt): 1–64 ASCII
characters, starts alphanumeric, remaining alphanumeric/`-`/`_`. Invalid names
abort with the naming rule restated before any session.

## SDK listing (additive, selector and resolution source)

### `RunningMicroVm` (new public type, re-exported from `lib.rs`)

Read-only snapshot of one actually-running machine:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable VM identifier, ordered by name. |
| `ssh` | `SshConnectionInfo` | Stored SSH connection material (guest `user`, `port`, guest `address`, host `private_key_path`, `public_key_path`), verbatim from the inventory. Paths only, never key contents. |

Operation:
`MicroVmSdk::list_running_microvms() -> Result<Vec<RunningMicroVm>, SdkError>`.
For each persisted row (name order), in order: skip rows whose persisted state
is not `Running`; load the full record with the existing `find_microvm`; keep
the row only when the start operation's own `is_vm_live` check passes
(recorded process still references the VM **and** the volume-local control
socket answers). Empty running set returns an empty vector. No filesystem
validation of key material inside the listing; no mutation, repair, or lock;
no schema change. Unparsable stored state is a typed error, never a panic.

## Records read, never written by the CLI

The CLI writes nothing to SQLite. The new SDK operation reads the full
`MicroVmRecord`, `PersistedNetwork`, `PersistedCredential`, and
`PersistedRuntime` for each `Running` candidate exactly as `start_microvm`
does, and reuses its `is_vm_live` liveness check. The CLI never inspects these
records directly. No lifecycle transition, network repair, launch, or state
write occurs anywhere in this feature.

## CLI output

### Session handoff (from the chosen `RunningMicroVm`)

No success-path output. The command hands the terminal to the child:

```text
ssh -i {private_key_path} [-p {port} when != 22] {user}@{address} [-- remote...]
```

No host-key options are added: verification keeps default OpenSSH behavior
(prompts on first connect, remembers accepted keys). Key paths only, never
contents. The child's exit code becomes the CLI's exit code (signal death maps
to `130`, matching the existing convention).

### Failure pointers (documentation references, not implemented commands)

| Pointer | Shown when |
|---|---|
| `microvm new` | Named machine is unknown. No creation shortcut is offered. |
| `microvm start {name}` | Named machine is known but not running (with its persisted state); or the running set is empty. |

## State and flow ownership

```text
CLI (this feature)                SDK (existing + one additive read)
──────────────────                ──────────────────────────────────
resolve name ──► escalate ──► pre-flight key ──► spawn ssh (inherited)
(interactive     (privilege.rs,    chosen entry     ▲
 selector over    unchanged;        only)           │ is_vm_live probes,
 running list,                       │              │ no mutation/repair
 no-name ⇒ list)                      └──────────────┘
```

CLI transitions: name unresolved → resolved → elevated (or already
privileged) → pre-flight → session (terminal owned by `ssh`) or typed
failure. No CLI-owned VM state, no retry loop, no confirmation gate:
elevation is the only gate before the session. Interrupt during
selector/elevation/establishment → cancelled, exit `130`, no session opened.
Interrupts inside an established session belong to the remote side.
