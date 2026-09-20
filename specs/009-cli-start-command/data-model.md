# Phase 1 Data Model: CLI `start` Command for Launching a MicroVM

## CLI input

### Start request

One machine name to launch, however supplied:

| Source | Rule |
|---|---|
| Positional `[NAME]` | Skips the selector when present. |
| `--name <NAME>` | Equivalent to the positional; both present MUST agree or the command reports the mismatch and starts nothing. |
| Interactive selector | Used only when neither flag is present and the terminal is interactive. |
| `--non-interactive` | Name is required (either form); missing name fails with usage guidance before any mutation. |

Validation (shared with `new`, abort-no-reprompt per clarification Q4): 1–64 ASCII characters,
starts alphanumeric, remaining alphanumeric/`-`/`_`. Invalid names abort with the naming rule
restated before any host mutation.

## SDK listing (additive, selector source)

### `MicroVmSummary` (new public type, re-exported from `lib.rs`)

Read-only inventory snapshot for selectors and future listing surfaces:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable VM identifier, ordered by name. |
| `state` | `MicroVmState` | Last persisted lifecycle state (`Configured`, `Running`, `Creating`). Never live-verified; running truth requires start/status checks. |

Operation: `MicroVmSdk::list_microvms() -> Result<Vec<MicroVmSummary>, SdkError>`. Single read-only
query (`SELECT name, state FROM microvms ORDER BY name`); empty inventory returns an empty vector.
Unparsable stored state is a typed error, never a panic. No locks, no migration, no host mutation.

## Records read, never written by the CLI

The CLI writes nothing to SQLite. The SDK `start_microvm` reads the full `MicroVmRecord`,
`PersistedNetwork`, `PersistedCredential`, and `PersistedRuntime` for the named VM and owns every
transition described in `specs/008-start-microvm/data-model.md` (live verification, network repair,
detached launch, `Running` commit). The CLI never inspects these records directly.

## CLI output

### Start result rendering (from `MicroVmStartResult`)

| Row | Content | Condition |
|---|---|---|
| Title | `✓ MicroVM {name} running` | Always. |
| Connection | `microvm ssh {name}` | Always (documentation hint). |
| Direct | `[prefix]ssh -i {private_key_path} -p {port} {user}@{ssh.address}` | Always; prefix is the elevation backend used (`sudo `/`pkexec `) or empty when already root. Paths only, never key contents. |
| LAN paragraph | Copy `{private_key_path}` to the other machine, then `ssh -i {key} -p {port} {user}@{network.lan_address}` | Only when `network.mode` is LAN-exposed. Host-only results show no LAN paragraph. |
| Stop | `microvm stop {name}` | Always last (documentation hint). |

Already-running results render identically to fresh starts (same rows, same order).

### `new` summary addition

`format_new_result` keeps all existing rows and appends `Start: microvm start {name}` naming the
created machine.

## State and flow ownership

```text
CLI (this feature)                SDK start_microvm (spec 008, existing)
──────────────────                ──────────────────────────────────────
resolve name ──► escalate ──► call once ──► render result / map error
                 (privilege.rs,                       ▲
                  unchanged)                          │ live checks, repair,
                                                      │ launch, Running commit
```

CLI transitions: name unresolved → resolved → elevated (or already root) → spinner → running
report or typed failure. No CLI-owned VM state, no retry loop, no confirmation gate (clarification
Q1: elevation is the only gate). Interrupt during selector/elevation → cancelled, exit `130`, no
side effects.
