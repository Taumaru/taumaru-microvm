# Snapshot and Restore Quickstart

This guide describes the expected workflow after feature 021 is implemented. It is a planning artifact; no implementation or command execution is part of this plan.

## Create an interactive snapshot

Run:

`microvm snapshot`

The snapshot is block-level and crash-consistent; it does not coordinate guest applications or database transactions. Guest software relies on its normal recovery behavior. The CLI asks whether to include source IPv4 settings. Choosing preserve keeps exact guest/LAN values and may conflict with addresses already used on another host. Choosing regenerate stores only LAN exposure, sanitizes the Taumaru-managed network file in the private archive copy, and lets restore allocate destination-local addresses. The SDK does not prompt for the choice.

## Restore from an archive

Interactive restore asks for a path and a single hidden password prompt:

`microvm restore`

Restore an explicit file:

`microvm restore /path/to/test.tmvmsnap`

Non-interactive restore:

`microvm restore /path/to/test.tmvmsnap --password 'your-password'`

Success reports the archived VM name and stopped state. The VM is stored under the destination SDK home and can later be started when compatible local runtime artifacts are available.

## Address policy outcomes

- A preserve-policy archive restores the recorded guest IPv4, prefix, gateway, optional LAN IPv4, mode, exposure, and MAC. If any exact value is unavailable or conflicts on the destination, restore fails without publishing the VM.
- A regenerate-policy archive contains no source address assignments. Restore derives network mode from LAN exposure, allocates local IPv4/network identity, writes the destination settings into the recovered disk, and leaves the VM stopped.
- Regenerate sanitization removes only the Taumaru-managed network file from a private copy. Arbitrary guest files are neither scanned nor rewritten, and may still contain IP text.

## Expected rejection cases

- Version 1, unknown version, inconsistent policy fields, unsupported IPv6, incorrect password, modified payload, incomplete archive, or invalid manifest publishes no VM.
- Existing VM name, occupied managed path, exact-address/MAC conflict, incompatible runtime, unavailable network capability, insufficient storage, child CoW overflow, or failed sanitization stops the operation and cleans only owned state.
- Cancellation or abrupt host shutdown leaves enough journal data for cleanup before a later restore retry; the source archive and source VM disk remain unchanged.

## Implementation validation scenarios

Verify both snapshot policy manifests and disk contents; online parent/child CoW isolation from guest writes; stopped-disk source immutability; journal recovery and managed-file sanitization; parent and child overflow checks; archive authentication and payload hashes; preserve and regenerate restore allocation; exact guest config after restore; kernel cache resolution by the existing start path; duplicate-name races; private key mode; network rollback; cancellation cleanup; and durable journal reconciliation after interruption.
