# Phase 1 Data Model: CLI `ls` Command for Listing MicroVMs

## CLI input

### Ls request

The command takes no operands — only the shared automation flag:

| Source | Rule |
|---|---|
| `--non-interactive` | No prompts of any kind; root required up front, missing rights fail before any listing attempt. Absent the flag, unprivileged terminals escalate through the prompt-driven flow. |
| Positional name / per-machine flags | None exist. A positional or lifecycle flag surfaces as a clap error (covered by a parser test). |

## Records read, never written by the CLI

The CLI writes nothing to SQLite and adds no SDK operation. One existing
SDK read serves the whole command:

| Path | SDK source | Shape used |
|---|---|---|
| Full overview | `list_microvms()` | Every stored row ordered by name with call-time verified state plus the additive persisted fields below. |

The CLI never inspects inventory records directly. All lifecycle behavior —
socket liveness, bounded parallel probes, probe-failure-resolves-to-`Stopped`
— stays inside the existing `list_microvms` operation, whose verification
semantics are unchanged by this feature.

## Extended listing shape (`MicroVmSummary`, additive)

Existing fields (`name`, `state`) keep their meaning: `state` is the
call-time verified state (`Running` iff the volume-local control socket
answers, else `Stopped`). The extension adds only already-persisted
creation data:

| Field | Type | Source | Absent when |
|---|---|---|---|
| `vcpu_count` | `u32` | `microvms.vcpu_count` | Never (row column). |
| `memory_bytes` | `u64` | `microvms.memory_requested_bytes` | Never (row column). Rendered with `format_mb_gb`. |
| `disk_size_bytes` | `u64` | `microvms.disk_size_bytes` | Never (row column). Rendered with `format_gb`. |
| `distribution_id` | `String` | `microvms.distribution_id` | Never (row column). |
| `image_id` | `String` | `microvms.image_id` | Never (row column). Rendered as `{distribution}={image}`. |
| `network_mode` | `Option<NetworkMode>` | `vm_networks` row via `load_network_record` | `None` for incomplete records (interrupted creation). |
| `guest_address` | `Option<IpAddr>` | Stored `config.guest_address` | `None` for incomplete records. |
| `lan_address` | `Option<IpAddr>` | Stored `config.lan_address` | `None` for host-only machines and incomplete records. |

All sizes are the configured values chosen at creation, never
live-measured usage. SSH user, port, and key paths are readable from the
same stored rows but are deliberately not added to the struct — the
clarify session excluded them and they must not cross into the listing.

## Network rendering rules

The table's `NETWORK` cell is a pure function of the triple:

| Stored triple | Rendered cell |
|---|---|
| `Some(HostOnly)`, guest `Some` | `host-only {guest}` |
| `Some(Lan)`, guest `Some`, LAN `Some` | `lan {lan} (guest {guest})` |
| `Some(Lan)`, guest `Some`, LAN `None` | `lan (guest {guest})` |
| Anything missing (incomplete record) | `-` |

## CLI output

### Ls table (from the returned `Vec<MicroVmSummary>`)

Rendered by `format_ls_table`, following the format/write pair
convention. One header plus one row per machine in SDK (name) order,
column widths from content maxima, no truncation — identical bytes on
every terminal shape for deterministic scripted output:

```text
NAME    STATE    VCPUS  MEMORY  DISK   IMAGE              NETWORK
db-01   stopped  2      2 GB    20 GB  ubuntu-24.04=base  host-only 10.200.8.2
web-01  running  4      4 GB    40 GB  ubuntu-24.04=base  lan 192.168.1.50 (guest 10.200.8.3)
```

State is text-first (`running`/`stopped` display text with semantic
color: running green, stopped dim); any missing capacity or network
value renders `-` so degraded rows stay in place. Key paths, socket
paths, and process identities never appear.

### Empty report (zero rows, stdout, exit `0`)

Rendered by `format_ls_empty` — a calm no-machines report pointing to
`microvm new`, not a table and not an error. It bypasses `CliError`
entirely, so no exit-code arm is involved.

### Failure pointers (documentation references, not implemented commands)

| Pointer | Shown when |
|---|---|
| `microvm new` | The inventory is empty (report, success) or a listing can't be produced and the home looks fresh. |
| Rerun with root | Non-interactive (or non-TTY) run without rights. |

## State and flow ownership

```text
CLI (this feature)                SDK (existing op, extended result)
──────────────────                ────────────────────────────────
gate privilege ──► list ──► render  list_microvms() (unchanged
(single pre-listing   │     table /     verify: bounded probes,
gate; bare ["ls"]     │     empty       failure ⇒ Stopped) + stored
child; no second      └──── report      row fields (no new query,
gate, no selector)                      no migration, no probing)
```

CLI transitions: ungated → elevated (or already privileged) → listed →
table or empty report, or typed failure. No CLI-owned VM state, no retry
loop, no confirmation gate: elevation is the only gate before the list.
Interrupt during elevation reports cancellation with exit `130` and no
listing. The command is strictly read-only: it never creates, starts,
stops, deletes, or mutates any machine or host state.
