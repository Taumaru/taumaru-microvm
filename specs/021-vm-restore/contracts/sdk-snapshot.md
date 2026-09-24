# SDK Snapshot Contract

## Operation

The SDK snapshot operation accepts a VM name, output path, non-empty password, and required typed IPv4 address policy. The operation returns its normal typed result and may report progress through a caller-provided callback. The SDK does not prompt, print, log, or choose a policy implicitly.

## Preserve IPv4 policy

- The manifest records source guest IPv4, prefix, optional guest gateway, optional LAN IPv4, logical mode, LAN exposure, and guest MAC.
- A running VM is read from the existing stable read-only Device Mapper capture. This is a block-level crash-consistent point, not application-consistent quiescing; guest applications rely on normal recovery behavior. A stopped VM may be read directly while serialized against lifecycle operations.
- No guest network configuration is altered.

## Regenerate IPv4 policy

- The manifest records the policy and LAN exposure only from source network configuration. It omits source guest/LAN addresses, prefix, gateway, mode, and MAC.
- The SDK creates a separately owned writable child CoW view derived from the stable capture. It replays committed ext4 journal transactions on that private child and removes only the fixed Taumaru-managed network file at `/etc/systemd/network/10-taumaru.network`.
- A stopped VM uses a read-only source loop view and the same private child transformation.
- The SDK archives the sanitized child view, verifies its validity while streaming, and never changes the source root disk, active guest, or read-only parent.
- If nested classic DM snapshots are not supported by a verified capability check, an isolated regular-file copy of the stable view may be used only after capacity is checked. Do not publish an archive if sanitization, journal handling, child validity, or cleanup requirements fail.
- Arbitrary guest files are not scanned or rewritten.

## Archive and cleanup

Version 2 declares the selected policy and its required network fields. Kernel registry metadata is sufficient to register the embedded exact kernel in a destination artifact inventory. Payload sizes and hashes cover the bytes actually archived after any sanitization. Close and flush the archive in a temporary output, release all owned child/parent mapper and loop resources, and atomically publish only after cleanup succeeds. Remove partial output on failure/cancellation.

Every loop and mapper is operation-owned and identified by VM plus operation identity. Child mapping/COW resources are checked during the full archive read and removed before their parent view; cleanup never removes the `.ext4` source disk.
