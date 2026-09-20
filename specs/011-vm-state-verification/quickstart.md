# Quickstart: VM State Verification

State is derived at call time now. There is no state column to set, seed, or
inspect — only sockets to answer and processes to reference VMs.

## Host prerequisites

Linux with the existing SDK home layout (`state/inventory.db`, VM volumes).
No KVM needed for read-path verification. The SDK home is passed explicitly;
the SDK never reads `HOME` or `TAUMARU_HOME`.

## 1. Killed machine reports stopped

```bash
microvm start web-01
kill -KILL <firecracker-pid>   # outside the CLI
microvm list                   # web-01 shows stopped
microvm start web-01           # fresh launch, no "already running" refusal
```

SDK equivalent: `list_microvms()` returns `Stopped` for the killed VM;
`start_microvm()` boots fresh.

## 2. Stale socket file never counts

Leave (or plant) a non-listening file at
`<volume>/firecracker.sock`, then list: the VM reports stopped. Only an
answering control connection reports running.

## 3. Recycled PID never counts

Run any long-lived process, record its PID as the VM's `process_id` while
the socket stays silent: the VM reports stopped. The process probe
(`/proc/<pid>/cmdline` referencing the VM socket/binary) is the guard.

## 4. Foreign-live machine is running but unadoptable

With an answering socket (e.g. VM started manually) and a missing/foreign
recorded PID: listings report running; `start_microvm` returns
`TemporaryRuntime` instead of launching a duplicate or claiming the foreign
process.

## 5. Bulk listing stays fast

Seed ~100 VMs in mixed states and list: every entry verified, total well
under 10s on a typical host. One hung probe cannot stall the listing — each
probe is bounded at 12s and failures resolve to stopped.

## 6. Migration removes the column

```bash
# On a pre-feature database:
sqlite3 <home>/state/inventory.db "SELECT name FROM pragma_table_info('microvms');"
# After running any SDK operation with the new binary: no `state` row.
# `SELECT state FROM microvms;` fails with "no such column".
```

Databases that already migrated a subset of versions pick up only the
missing ones; checksums of applied migrations are drift-gated.

## Automated validation

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Targeted suites:

```bash
cargo test -p taumaru-microvm --test microvm_listing
cargo test -p taumaru-microvm --test public_api
cargo test -p taumaru-microvm --test sqlite_persistence
cargo test -p taumaru-microvm manager::
```

The suite covers: socket-governs verdicts in all four probe combinations,
recycled-PID safety, stale-file safety, foreign-live start refusal, bulk
mixed-state correctness with injected probe failures, migration column
removal on a seeded v3 database, completeness gates for create/configure/
start, and SDK silence under failure injection. Real Firecracker/KVM
integration stays capability-gated out of the default suite.
