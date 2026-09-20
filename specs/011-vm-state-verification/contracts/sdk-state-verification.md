# SDK + CLI Contract: VM State Verification

This contract describes the public SDK surface, the migration, and the CLI
presentation for the feature. It contains no command lines, SQL beyond the
migration body, key material, or process-handle internals.

## Public types

`MicroVmState` keeps its name and shrinks to two variants:

```rust
pub enum MicroVmState {
    Running,
    Stopped,
}
```

Removed: `Creating`, `Configured`. `Display` renders `running`/`stopped`.
`MicroVmSummary { name, state }` keeps its shape; `state` is now the
call-time verified value. `RunningMicroVm { name, ssh }` is unchanged.
`MicroVmStartResult.state` is always `Running`; `MicroVmCreationResult.state`
is always `Stopped`. All types stay re-exported from
`crates/sdk/src/lib.rs` with Rustdoc stating the new invariants. No new
public type is introduced.

## Public operations

### List all MicroVMs (verified)

```rust
impl MicroVmSdk {
    pub async fn list_microvms(&self) -> Result<Vec<MicroVmSummary>, SdkError>;
}
```

Returns one entry per stored VM ordered by name. Each entry carries the
live-verified state: `Running` iff the VM's volume-local control socket
answers (bounded probe, CPU-scaled fan-out); `Stopped` otherwise, including
probe failures and timeouts. Performs no writes. Inventory access failures
remain typed errors.

### List running MicroVMs (verified)

```rust
impl MicroVmSdk {
    pub async fn list_running_microvms(&self) -> Result<Vec<RunningMicroVm>, SdkError>;
}
```

Returns one entry per socket-answering VM ordered by name, with stored SSH
metadata verbatim (paths only). Silent, failed, or timed-out probes are
omitted, never errors. Performs no writes.

### Start (socket-governed, foreign-live refusal)

`start_microvm(name)` behavior:

1. Unknown name → `NotFound`. Incomplete record → `LifecycleConflict`.
2. Socket answers and recorded process references the VM → idempotent return,
   nothing launched.
3. Socket answers but the recorded PID is missing or foreign →
   `TemporaryRuntime` (`stopped: false`, "already running outside SDK
   management"). No launch, no adoption, no signalling.
4. Socket silent → stale recovery (clear refs, remove owned socket only after
   liveness proves no listener) and fresh launch as today.

### Create and configure (completeness gates)

- `create_microvm` repeat against a complete record runs the existing
  immutable-field comparison; against an incomplete record it fails with
  `LifecycleConflict`. Success returns `state: Stopped`.
- `configure_network` requires a complete record and a silent socket; a live
  socket fails with `TemporaryRuntime` (`stopped: false`).

## Error contract

No new public error variant. Reuse:

| Situation | Variant |
|---|---|
| Unknown VM name | `NotFound` for kind `MicroVM` |
| Incomplete record for create/configure/start | `LifecycleConflict` |
| Live socket blocks configure | `TemporaryRuntime` (`stopped: false`) |
| Socket live but unmanaged blocks start | `TemporaryRuntime` (`stopped: false`) |
| Inventory access fails on a read path | `Sqlite` / `Migration`, never a per-VM verdict |
| Launch/readiness/network/host failures | Existing variants unchanged |

Per-VM probe failures on read paths resolve to `Stopped`, never to errors.

## Side-effect contract

- Read paths (`list_microvms`, `list_running_microvms`) perform zero
  persistence writes, launch nothing, take no locks, emit no output/logs.
- The SDK gains no dependency; `tokio` `sync`/`rt`/`time`/`macros` features
  already enabled cover semaphore, `spawn_blocking`, `JoinSet`, `timeout`.
- Bulk verification bounds: `(available_parallelism * 4).clamp(8, 32)`
  permits, 12s per-probe timeout, ~100 VMs in well under SC-002's 10s for
  typical mixed inventories.

## Migration contract

Version 4, `crates/sdk/migrations/0004_drop_microvm_state.sql`:

```sql
DROP INDEX IF EXISTS microvms_state;
ALTER TABLE microvms DROP COLUMN state;
```

Applied once inside the runner's per-migration transaction with automatic
checksum/version gating. Existing databases lose the column including stale
`running` markers; fresh databases never have it. No downgrade path exists;
downgrading requires restoring a pre-migration backup. A migration that
merely ignores the column does not satisfy this feature.

## CLI presentation contract

- `start` selector lists all VMs as `name [state]` with verified states; no
  flow change.
- `ssh` keeps its `Running` pre-filter (now truthful) and
  `resolve_running_entry` flow; error labels use the verified state string.
- `new` drops the `Configured` post-check; creation `Ok` implies
  completeness.
- State keeps current calm labels; a verified-stopped machine is never
  presented as running. No color-only signaling, no new copy beyond
  `running`/`stopped`.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

for vm in sdk.list_microvms().await? {
    println!("{} [{}]", vm.name, vm.state);
}

let running = sdk.list_running_microvms().await?;
// Every entry's socket answered at call time; silent VMs are absent.

let started = sdk.start_microvm("web-01").await?;
assert_eq!(started.state, MicroVmState::Running);
```
