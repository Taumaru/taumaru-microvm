use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use super::lifecycle::{MicroVmState, NetworkMode};

/// Caller-selected input for creating and initially configuring a MicroVM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateMicroVmRequest {
    /// Stable path-friendly VM identifier.
    pub name: String,
    /// Registry ID of the distribution metadata.
    pub distribution_id: String,
    /// Exact registry ID of the image to copy.
    pub image_id: String,
    /// Requested root-disk size in bytes.
    pub disk_size_bytes: u64,
    /// Requested virtual CPU count.
    pub vcpu_count: u32,
    /// Requested memory size in bytes.
    pub memory_bytes: u64,
    /// Whether the VM should use routed LAN mode with a LAN address.
    pub expose_on_lan: bool,
    /// Optional explicit LAN address override. `None` selects automatically.
    ///
    /// Only meaningful with `expose_on_lan`; providing a value for a host-only
    /// VM is an invalid request.
    pub lan_address: Option<Ipv4Addr>,
    /// Optional absolute VM-volume directory. Defaults below the SDK home.
    pub volume_path: Option<PathBuf>,
}

/// SSH connection metadata returned after the VM is configured.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshConnectionInfo {
    /// Fixed guest account.
    pub user: String,
    /// Fixed SSH port.
    pub port: u16,
    /// Last configured guest address.
    pub address: IpAddr,
    /// Host path of the protected private key.
    pub private_key_path: PathBuf,
    /// Host path of the public key.
    pub public_key_path: PathBuf,
}

/// Persisted network configuration exposed by creation and reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfiguration {
    /// Selected network mode.
    pub mode: NetworkMode,
    /// Guest address used for SSH.
    pub guest_address: IpAddr,
    /// Prefix length of the guest address.
    pub prefix_length: u8,
    /// Host or DHCP gateway, when known.
    pub gateway: Option<IpAddr>,
    /// VM-specific TAP interface.
    pub tap_name: String,
    /// SDK-managed bridge for legacy LAN mode. Always `None` for routed LAN.
    pub bridge_name: Option<String>,
    /// Default-route uplink used for LAN mode.
    pub uplink_name: Option<String>,
    /// Committed LAN address for routed LAN mode. `None` for host-only VMs.
    pub lan_address: Option<IpAddr>,
}

/// A resource considered by network reconciliation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NetworkResource {
    /// VM-specific TAP interface.
    Tap,
    /// Host address on the TAP interface.
    TapAddress,
    /// Host route for the VM's LAN address through the TAP interface.
    HostRoute,
    /// Proxy ARP entry for the VM's LAN address on the uplink interface.
    ProxyArpEntry,
    /// SDK-managed bridge (legacy LAN rows only).
    Bridge,
    /// Uplink-to-bridge attachment (legacy LAN rows only).
    BridgeUplinkAttachment,
    /// TAP-to-bridge attachment (legacy LAN rows only).
    BridgeTapAttachment,
    /// Host forwarding configuration.
    Forwarding,
    /// Per-VM iptables FORWARD rule.
    ForwardRule,
    /// Per-VM NAT rule.
    Nat,
    /// Per-VM iptables MASQUERADE rule for the private host-only range.
    IptablesNat,
    /// Firecracker network interface configuration.
    FirecrackerInterface,
    /// External DHCP lease observed for the VM (legacy LAN rows only).
    DhcpLease,
}

impl NetworkResource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::TapAddress => "tap_address",
            Self::HostRoute => "host_route",
            Self::ProxyArpEntry => "proxy_arp_entry",
            Self::Bridge => "bridge",
            Self::BridgeUplinkAttachment => "bridge_uplink_attachment",
            Self::BridgeTapAttachment => "bridge_tap_attachment",
            Self::Forwarding => "forwarding",
            Self::ForwardRule => "forward_rule",
            Self::Nat => "nat",
            Self::IptablesNat => "iptables_nat",
            Self::FirecrackerInterface => "firecracker_interface",
            Self::DhcpLease => "dhcp_lease",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "tap" => Some(Self::Tap),
            "tap_address" => Some(Self::TapAddress),
            "host_route" => Some(Self::HostRoute),
            "proxy_arp_entry" => Some(Self::ProxyArpEntry),
            "bridge" => Some(Self::Bridge),
            "bridge_uplink_attachment" => Some(Self::BridgeUplinkAttachment),
            "bridge_tap_attachment" => Some(Self::BridgeTapAttachment),
            "forwarding" => Some(Self::Forwarding),
            "forward_rule" => Some(Self::ForwardRule),
            "nat" => Some(Self::Nat),
            "iptables_nat" => Some(Self::IptablesNat),
            "firecracker_interface" => Some(Self::FirecrackerInterface),
            "dhcp_lease" => Some(Self::DhcpLease),
            _ => None,
        }
    }
}

/// Result returned by [`crate::MicroVmSdk::create_microvm`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicroVmCreationResult {
    /// Stable VM identifier.
    pub name: String,
    /// Always [`MicroVmState::Stopped`] on success. The socket is verified inactive at return.
    pub state: MicroVmState,
    /// Selected distribution ID.
    pub distribution_id: String,
    /// Selected image ID.
    pub image_id: String,
    /// VM-exclusive volume directory.
    pub volume_path: PathBuf,
    /// Writable VM-local root filesystem.
    pub rootfs_path: PathBuf,
    /// Expected future Firecracker socket path. It is inactive on return.
    pub socket_path: PathBuf,
    /// Requested vCPU count.
    pub vcpu_count: u32,
    /// Requested memory in bytes.
    pub memory_bytes: u64,
    /// Requested root-disk size in bytes.
    pub disk_size_bytes: u64,
    /// Configured network metadata.
    pub network: NetworkConfiguration,
    /// SSH connection metadata with path-only credential references.
    pub ssh: SshConnectionInfo,
}

/// Result returned by [`crate::MicroVmSdk::start_microvm`].
///
/// The operation takes only the VM name: the volume directory is read from
/// the inventory record, never from the caller. `socket_path` is always the
/// volume-local control socket and is the preferred channel for later
/// control; `process_id` is the background machine process and exists as the
/// fallback for forced termination of an unresponsive machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicroVmStartResult {
    /// Stable VM identifier.
    pub name: String,
    /// Always [`MicroVmState::Running`] on success. The socket answered at return.
    pub state: MicroVmState,
    /// VM-exclusive volume directory read from the inventory record.
    pub volume_path: PathBuf,
    /// Writable VM-local root filesystem used for the launch.
    pub rootfs_path: PathBuf,
    /// Volume-local control socket. It answers on return and is the
    /// preferred control channel for later operations.
    pub socket_path: PathBuf,
    /// Background machine process identifier for forced termination fallback.
    pub process_id: u32,
    /// Reconciled network metadata with the persisted identity unchanged.
    pub network: NetworkConfiguration,
    /// SSH connection metadata with path-only credential references.
    pub ssh: SshConnectionInfo,
}
/// Result returned by [`crate::MicroVmSdk::stop_microvm`].
///
/// The operation takes only the VM name: the volume directory and every
/// runtime reference are read from the inventory record, never from the
/// caller. `socket_path` is always the volume-local control socket and is
/// silent at return. `forced` is `true` only when SIGKILL was delivered to
/// the recorded process; it is `false` for graceful exit, already-stopped,
/// and natural-exit-during-wait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicroVmStopResult {
    /// Stable VM identifier.
    pub name: String,
    /// Always [`MicroVmState::Stopped`] on success. The socket is silent at return.
    pub state: MicroVmState,
    /// Volume-local control socket. It is silent at return.
    pub socket_path: PathBuf,
    /// Whether forced termination was used. `true` only when SIGKILL was
    /// delivered to the recorded process.
    pub forced: bool,
}

/// Read-only inventory snapshot of one persisted MicroVM.
///
/// Returned by [`crate::MicroVmSdk::list_microvms`] for selectors and listing
/// surfaces. The state is verified at call time: `Running` if and only if the
/// VM's volume-local control socket answers, `Stopped` otherwise. The
/// remaining fields are the values persisted at creation and never
/// live-measured.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MicroVmSummary {
    /// Stable VM identifier, ordered by name.
    pub name: String,
    /// Call-time verified state (`Running` iff the socket answers, else `Stopped`).
    pub state: MicroVmState,
    /// Configured virtual CPU count, already persisted at creation.
    pub vcpu_count: u32,
    /// Configured memory size in bytes, already persisted at creation. This is
    /// the configured value chosen at creation, never live-measured usage.
    pub memory_bytes: u64,
    /// Configured root-disk size in bytes, already persisted at creation. This is
    /// the configured value chosen at creation, never live-measured usage.
    pub disk_size_bytes: u64,
    /// Registry ID of the distribution metadata, already persisted at creation.
    pub distribution_id: String,
    /// Exact registry ID of the image, already persisted at creation.
    pub image_id: String,
    /// Stored network mode, already persisted at creation. `None` for incomplete records.
    pub network_mode: Option<NetworkMode>,
    /// Stored guest address used for SSH. `None` for incomplete records.
    pub guest_address: Option<IpAddr>,
    /// Stored LAN address for routed LAN mode. `None` for host-only machines and incomplete records.
    pub lan_address: Option<IpAddr>,
}

/// Read-only snapshot of one actually-running MicroVM.
///
/// Returned by [`crate::MicroVmSdk::list_running_microvms`] for the SSH selector and named
/// SSH resolution. Liveness is verified at call time with the same probes the start operation
/// uses; the last persisted lifecycle state alone never decides membership. The SSH material
/// is the stored connection metadata verbatim: paths only, never key contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunningMicroVm {
    /// Stable VM identifier, ordered by name.
    pub name: String,
    /// Stored SSH connection metadata with path-only credential references.
    pub ssh: SshConnectionInfo,
}

/// Total number of creation stages reported through the progress observer.
///
/// Every observed `create_microvm` operation reports step counters out of this fixed
/// total: each finished stage reports `N` of `TOTAL_CREATION_STEPS`, and the terminal
/// event repeats the finished-step count out of the same total plus its outcome.
pub const TOTAL_CREATION_STEPS: u64 = 6;

/// One named discrete phase of MicroVM creation, in fixed emission order.
///
/// The SDK emits exactly one event when each stage finishes. Stage identities are
/// stable and lowercase through [`std::fmt::Display`] so callers can render labels
/// without additional SDK information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreationStage {
    /// Request validation plus the existing-VM lookup decision.
    Validation,
    /// Artifact, kernel, and runtime resolution with volume availability checks.
    PrerequisiteResolution,
    /// VM-local `rootfs.ext4` copy, grown to the requested size when needed.
    VolumePreparation,
    /// Ed25519 key generation plus public-key injection into the VM-local copy.
    CredentialSetup,
    /// Network port configuration plus guest network config writes.
    NetworkConfiguration,
    /// Runtime verification, metadata persistence, and the state commit.
    Finalization,
}

impl CreationStage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::PrerequisiteResolution => "prerequisite_resolution",
            Self::VolumePreparation => "volume_preparation",
            Self::CredentialSetup => "credential_setup",
            Self::NetworkConfiguration => "network_configuration",
            Self::Finalization => "finalization",
        }
    }
    #[allow(dead_code)]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "validation" => Some(Self::Validation),
            "prerequisite_resolution" => Some(Self::PrerequisiteResolution),
            "volume_preparation" => Some(Self::VolumePreparation),
            "credential_setup" => Some(Self::CredentialSetup),
            "network_configuration" => Some(Self::NetworkConfiguration),
            "finalization" => Some(Self::Finalization),
            _ => None,
        }
    }
}

impl std::fmt::Display for CreationStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The single closing state of an observed `create_microvm` operation.
///
/// Every observed operation ends with exactly one terminal event carrying one of
/// these outcomes. Terminal events accompany, never replace, the operation's typed
/// `Result`: `Completed` accompanies `Ok`, while `AlreadyConfigured` accompanies the
/// returned existing VM and `Failed` accompanies the unchanged typed error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreationOutcome {
    /// A new VM was configured and stopped.
    Completed,
    /// An identical already-configured VM was returned unchanged.
    AlreadyConfigured,
    /// The operation failed at the named stage.
    Failed {
        /// Stage that observed the error.
        stage: CreationStage,
    },
}

/// Transient progress notification emitted while `create_microvm` runs.
///
/// Events are emitted in real time: each stage emits a `Started` event when it
/// begins, byte-moving work emits `InProgress` ticks as bytes advance, and each
/// stage emits a `Finished` event when it completes. Terminal events carry
/// `outcome: Some(_)` and repeat the finished-step count plus the closing outcome.
/// `overall_percent` always reflects total creation work (completed plus fractional
/// stage progress, scaled to 0–100), so a caller can drive a single progress bar
/// from this field alone. Events are never persisted and cannot change the
/// operation's outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreationProgress {
    /// Stage this event belongs to, or the failed stage for a `Failed` terminal.
    pub stage: CreationStage,
    /// Finished stages so far, out of [`TOTAL_CREATION_STEPS`].
    pub completed_steps: u64,
    /// Always [`TOTAL_CREATION_STEPS`].
    pub total_steps: u64,
    /// Overall creation progress in percent (0–100), derived from completed stages
    /// plus fractional progress inside the current stage.
    pub overall_percent: u64,
    /// What happened inside the stage for this event.
    pub phase: CreationEventPhase,
    /// Finished bytes for byte-moving work. `None` on steps-only events.
    pub bytes_completed: Option<u64>,
    /// Expected bytes for byte-moving work. `None` on steps-only events.
    pub expected_bytes: Option<u64>,
    /// `None` for stage events; the closing outcome for terminal events.
    pub outcome: Option<CreationOutcome>,
}

/// What happened inside a creation stage for one progress event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreationEventPhase {
    /// The stage started; no work inside it is finished yet.
    Started,
    /// Byte-moving work inside the stage advanced.
    InProgress,
    /// The stage finished.
    Finished,
}

/// Result returned by the independent network reconciliation operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfigurationResult {
    /// Stable VM identifier.
    pub name: String,
    /// Desired network configuration after reconciliation.
    pub configuration: NetworkConfiguration,
    /// Resources created or repaired by the operation.
    pub applied: Vec<NetworkResource>,
    /// Resources that were already correct.
    pub skipped: Vec<NetworkResource>,
}

/// Internal durable VM record used by SQLite and the manager.
#[derive(Clone, Debug)]
pub(crate) struct MicroVmRecord {
    pub id: i64,
    pub name: String,
    pub distribution_id: String,
    pub image_id: String,
    pub kernel_id: String,
    pub firecracker_package_id: String,
    pub firectl_package_id: String,
    pub disk_size_bytes: u64,
    pub memory_bytes: u64,
    pub memory_effective_mib: u64,
    pub vcpu_count: u32,
    pub volume_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub socket_path: PathBuf,
    pub expose_on_lan: bool,
    pub created_at: i64,
}

/// Internal persisted network attachment including identity and ownership details.
#[derive(Clone, Debug)]
pub(crate) struct PersistedNetwork {
    pub config: NetworkConfiguration,
    pub host_address: Option<IpAddr>,
    pub guest_mac: String,
    pub dhcp_lease_reference: Option<String>,
    pub desired_boot_parameters: String,
    pub resources: Vec<PersistedNetworkResource>,
    /// Uplink CIDR the LAN address was selected from. `None` for host-only VMs.
    pub uplink_cidr: Option<String>,
    /// Whether this VM caused proxy ARP enablement on the uplink.
    pub proxy_arp_enabled_by_sdk: bool,
    /// Legacy bridge ownership flags. Unused for routed LAN rows.
    pub bridge_created_by_sdk: bool,
    /// Legacy uplink attachment flag. Unused for routed LAN rows.
    pub uplink_attached_by_sdk: bool,
    pub forwarding_enabled_by_sdk: bool,
    /// Legacy nftables ownership flags. Unused for routed LAN rows.
    pub nat_table_created_by_sdk: bool,
    /// Legacy nftables ownership flags. Unused for routed LAN rows.
    pub nat_chain_created_by_sdk: bool,
    /// Whether the host route for the LAN address was created by the SDK.
    pub host_route_created_by_sdk: bool,
    /// Whether the proxy ARP entry for the LAN address was created by the SDK.
    pub proxy_arp_entry_created_by_sdk: bool,
    /// Legacy uplink address snapshot. Unused for routed LAN rows.
    pub host_address_specs: Vec<String>,
    /// Legacy default-route snapshot. Unused for routed LAN rows.
    pub default_route_specs: Vec<Vec<String>>,
}

/// Internal resource fingerprint used to reconcile and clean up network state safely.
#[derive(Clone, Debug)]
pub(crate) struct PersistedNetworkResource {
    pub resource: NetworkResource,
    pub identity: String,
    pub fingerprint: String,
    pub ownership: String,
    pub adapter_handle: Option<String>,
    pub last_observed: String,
}

/// Internal credential metadata; key contents are intentionally not represented.
#[derive(Clone, Debug)]
pub(crate) struct PersistedCredential {
    pub private_key_path: PathBuf,
    pub public_key_path: PathBuf,
    pub guest_authorized_keys_path: String,
    pub key_type: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub public_key_fingerprint: String,
    pub file_mode: String,
}

/// Internal runtime metadata for future lifecycle operations.
#[derive(Clone, Debug)]
pub(crate) struct PersistedRuntime {
    pub firecracker_path: PathBuf,
    pub firectl_path: PathBuf,
    pub socket_path: PathBuf,
    pub process_id: Option<u32>,
    pub process_state: String,
}

pub(crate) fn unspecified_address() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

#[cfg(test)]
mod tests {
    use super::CreationStage;

    #[test]
    fn creation_stages_render_stable_snake_case_labels() {
        let cases = [
            (CreationStage::Validation, "validation"),
            (
                CreationStage::PrerequisiteResolution,
                "prerequisite_resolution",
            ),
            (CreationStage::VolumePreparation, "volume_preparation"),
            (CreationStage::CredentialSetup, "credential_setup"),
            (CreationStage::NetworkConfiguration, "network_configuration"),
            (CreationStage::Finalization, "finalization"),
        ];
        for (stage, label) in cases {
            assert_eq!(stage.to_string(), label);
            assert_eq!(CreationStage::parse(label), Some(stage));
        }
        assert_eq!(CreationStage::parse("unknown"), None);
    }
}
