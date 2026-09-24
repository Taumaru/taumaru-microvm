# Snapshot and Restore Quickstart

Snapshots contain an encrypted, crash-consistent copy of a MicroVM disk, its embedded kernel, SSH credentials, portable configuration, and a versioned manifest. They do not contain guest RAM or process state. The guest keeps running while its disk is captured; applications that need transaction-level consistency must quiesce themselves before snapshot creation.

## Create a snapshot interactively

Run:

```sh
microvm snapshot
```

The CLI selects a MicroVM, asks where to save the archive, asks whether to preserve its IPv4 assignments, and requests a password twice. The output file is named `<vm-name>.tmvmsnap` in the selected existing folder.

To choose each value explicitly:

```sh
microvm snapshot web-01 ./web-01.tmvmsnap \
  --address-policy regenerate \
  --password 'use-a-strong-password'
```

The supported policies are:

- `preserve`: records the guest IPv4 address, prefix, applicable gateway and LAN address, network mode, exposure, and MAC. Restore fails if those exact values conflict on the destination.
- `regenerate`: records only whether the VM is exposed on the LAN. The snapshot's private disk view removes `/etc/systemd/network/10-taumaru.network`; restore allocates destination-local addresses and writes the new network configuration into the restored disk.

The interactive policy prompt warns about address conflicts when choosing `preserve`. Non-interactive snapshot creation requires `--address-policy`. The SDK never prompts. Avoid putting passwords in shell history or process arguments; omit `--password` in an interactive terminal to use the masked prompt.

## Restore an archive

Interactive restore asks for the archive path and one masked password prompt (without confirmation):

```sh
microvm restore
```

Or provide them explicitly:

```sh
microvm restore ./web-01.tmvmsnap --password 'the-snapshot-password'
```

Restore uses the VM name and settings from the authenticated archive. It rejects an existing VM name or occupied managed paths, verifies archive authentication and payload hashes, and installs files under the destination SDK home. On success, the VM appears in local inventory in the stopped state. Restore does not launch Firecracker.

The destination must be Linux, use a compatible guest architecture, have available local Firecracker and `firectl` binaries, and provide enough disk space and network resources. The embedded kernel is registered locally from the exact archived bytes, so later start resolves the restored kernel without contacting the source host or registry.

## Consistency and host requirements

The disk image is captured as a point-in-time block view and is crash-consistent, not application-consistent. The capture does not pause the VM, and guest writes after capture do not change the snapshot view. Snapshot operations require the host privileges and Linux loop/Device Mapper and ext4 tools used by the SDK. When the host cannot provide a nested classic Device Mapper snapshot for the private regenerate view, the SDK uses an exact-size private copy; if neither method is available, snapshot creation fails without publishing an archive. CoW overflow also aborts publication.

## Failure and recovery behavior

Version 1, unknown versions, unsupported IPv6, inconsistent policy fields, incorrect passwords, modified or truncated payloads, incompatible runtimes, occupied names/paths, network conflicts, and insufficient storage fail before the VM is published. Failure and cancellation remove resources recorded as owned by that restore operation and preserve existing VMs and the input archive. An interrupted restore is reconciled from its durable journal before a retry for the archived VM name.

Regenerate sanitization changes only the private snapshot view. It removes only the Taumaru-managed network file and does not scan arbitrary guest files, which may still contain source IP text. The source disk and running VM remain unchanged.
