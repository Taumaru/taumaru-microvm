# Runtime Disk Lifecycle Contract

## Public SDK surface

No public method signature or result type changes.

- Start remains MicroVmSdk.start_microvm(name).
- Stop remains MicroVmSdk.stop_microvm(name).
- MicroVmStartResult.rootfs_path continues to identify the durable rootfs.ext4 file from inventory.
- The transient /dev/mapper path is internal and is supplied only to firectl.
- No loop number, Device Mapper name, or mapping UUID is added to public results or persisted inventory.

## Start contract

A successful start must:

1. Verify the VM inventory and persistent root disk using existing validation.
2. Reuse one existing runtime mapping only if its stable UUID, live snapshot-origin table, range, loop dependency, and backing-file identity match the requested VM.
3. Otherwise create a writable loop association and an owned snapshot-origin mapping, unless a foreign, ambiguous, or in-use resource prevents safe setup.
4. Launch firectl using the verified mapper path as its read-write root drive.
5. Return the existing public start result with rootfs_path still pointing to rootfs.ext4.

An already-running VM with verified process and socket evidence remains idempotent and does not create another process or mapping. A persisted starting process is adopted only when its PID, executable identity, and ready socket agree. If that process is alive but not ready, start returns a retryable error and retains its identity and mappings instead of launching a duplicate.

A per-VM cross-process advisory lock is held for the entire start transition so separate SDK instances sharing a home cannot race process startup or mapping setup. Lock acquisition must not block a Tokio worker. A partial chain may be rebuilt only after ownership and lack of use are verified. A name collision with a foreign UUID or backing disk is an error with no mutation. Setup failures do not fall back to launching against the raw .ext4 path.

## Stop contract

The existing graceful shutdown and forced termination behavior remains. A per-VM cross-process advisory lock is held through shutdown and cleanup. Mapping cleanup starts only after the process is verified exited and no active process or open mapper reference uses the disk. If the socket is silent but a recorded provisional process still matches a live process, stop retains the PID and mappings and returns a retryable error.

Cleanup order is:

1. Remove the exact owned Device Mapper table without force or deferred removal.
2. Detach its verified loop association.
3. Verify the association is gone.
4. Clear stale process metadata and remove a stale socket.
5. Return Stopped while leaving the rootfs.ext4 file at its original path and contents.

Calling stop for an already-stopped VM also attempts idempotent cleanup of verified owned leftovers. Busy or ambiguous resources remain in place and return a typed error so the caller can retry.

## Resource identity contract

The mapper name includes a recognizable VM-name component and a digest derived from canonical SDK home plus validated VM name. The Device Mapper UUID contains the stable full digest. Names and UUIDs must remain within Linux kernel limits and use safe characters.

The SDK discovers the active loop from the mapper dependency and verifies that loop against the VM rootfs backing file. The dynamic /dev/loopN number is never treated as durable identity. Mapping discovery does not query SQLite runtime mapping rows because no such rows exist.

## Failure mapping

Use existing SDK error variants to preserve the public error enum contract:

| Failure | SDK error family |
|---|---|
| Missing tool, kernel target, permissions, or mapper-node access | RuntimeIncompatible or HostCommand |
| Foreign/ambiguous mapper identity or backing-file collision | StorageConflict |
| Process not confirmed stopped or mapper remains open | TemporaryRuntime |
| Primary operation and rollback both fail | Cleanup |

SDK failures remain returned as Result errors. The adapter captures subprocess output and does not write diagnostics to SDK stdout, stderr, or logging.
