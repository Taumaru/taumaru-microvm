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
    /// Always [`MicroVmState::Configured`] on success.
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

/// Result returned by the independent network reconciliation operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfigurationResult {
    /// Stable VM identifier.
    pub name: String,
    /// Always [`MicroVmState::Configured`] on success.
    pub state: MicroVmState,
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
    pub state: MicroVmState,
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
