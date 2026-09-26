# Implementation Plan: MicroVM Boot Autostart

**Branch**: `023-microvm-boot-autostart` | **Date**: 2026-09-26 | **Spec**: [spec.md](./spec.md)

## Summary

The SDK stores an autostart policy per MicroVM and can start every enabled machine; the CLI
manages policies and installs a systemd template unit that runs `microvm autostart run` at boot.

## SDK

- Migration `0006_microvm_autostart.sql`: table `vm_autostart` keyed by `microvm_id` with
  `ON DELETE CASCADE`, `enabled`, `max_start_attempts` (CHECK 1..10), timestamps.
- `domain/autostart.rs`: `AutostartPolicy`, `AutostartSettings`, `AutostartPolicyUpdate`,
  `AutostartDeleteResult`, `AutostartRunReport`, `AutostartOutcome`, `AutostartResult`.
- `MicroVmRepository`: find/list/insert/update/delete autostart policy (SQLite adapter).
- `manager/autostart.rs`: `create_autostart_policy`, `update_autostart_policy`,
  `delete_autostart_policy`, `autostart_policy`, `list_autostart_policies`,
  `start_autostart_microvms`. The run reuses `start_microvm` with a linear retry backoff.

## CLI

- `microvm autostart add|edit|rm|ls` plus hidden `run` (`cli.rs`, `commands/autostart.rs`).
- `boot.rs`: `SystemdAutostart` writes `/etc/systemd/system/taumaru-microvm-autostart@.service`
  (only when content changes, then `daemon-reload`) and enables or disables the instance
  `taumaru-microvm-autostart@<systemd-escaped home>.service`. `%f` restores the home as
  `TAUMARU_HOME`. `ExecStart` points at the current executable.
- `output/human/autostart.rs`: result blocks, table, and boot run report.

## Constitution Check

- Lifecycle is implemented once: the boot run calls the SDK `start_microvm`.
- The SDK has no output, logging, or knowledge of systemd; the CLI owns the boot hook.
- Dependency direction is unchanged (CLI → SDK).
