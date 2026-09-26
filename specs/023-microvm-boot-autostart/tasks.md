# Tasks: MicroVM Boot Autostart

**Input**: [spec.md](./spec.md), [plan.md](./plan.md)

## SDK

- [x] T001 Add migration `0006_microvm_autostart.sql` and register it with the required-table check
- [x] T002 Add autostart domain types and attempt validation in `crates/sdk/src/domain/autostart.rs`
- [x] T003 Extend `MicroVmRepository` and the SQLite adapter with autostart persistence
- [x] T004 Implement autostart operations in `crates/sdk/src/manager/autostart.rs`
- [x] T005 Re-export the public types from `lib.rs` with Rustdoc
- [x] T006 Add `crates/sdk/tests/autostart.rs` (CRUD, idempotence, conflict, typed errors, cascade)
- [x] T007 Add manager unit tests for run success, paused skip, and retry exhaustion

## CLI

- [x] T008 Add `autostart` clap definitions with `add`, `edit`, `rm`, `ls`, and hidden `run`
- [x] T009 Implement `crates/cli/src/boot.rs` systemd integration with unit tests
- [x] T010 Implement `crates/cli/src/commands/autostart.rs` with selector and non-interactive paths
- [x] T011 Add human output and error constructors in the three-part style
- [x] T012 Add parser tests and command-surface tests

## Verification

- [x] T013 Run `cargo fmt`, `check`, `clippy -D warnings`, and `test` for all targets
- [ ] T014 Manual boot check on a systemd host
