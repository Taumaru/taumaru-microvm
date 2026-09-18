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
    /// Whether the VM should use the explicit LAN bridge/DHCP mode.
    pub expose_on_lan: bool,
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
    /// SDK-managed bridge for LAN mode.
    pub bridge_name: Option<String>,
    /// Default-route uplink used for LAN mode.
    pub uplink_name: Option<String>,
}

/// A resource considered by network reconciliation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NetworkResource {
    /// VM-specific TAP interface.
    Tap,
    /// Host address on the TAP interface.
    TapAddress,
    /// SDK-managed bridge.
    Bridge,
    /// Uplink-to-bridge attachment.
    BridgeUplinkAttachment,
    /// TAP-to-bridge attachment.
    BridgeTapAttachment,
    /// Host forwarding configuration.
    Forwarding,
    /// Per-VM NAT rule.
    Nat,
    /// Firecracker network interface configuration.
    FirecrackerInterface,
    /// External DHCP lease observed for the VM.
    DhcpLease,
}

impl NetworkResource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::TapAddress => "tap_address",
            Self::Bridge => "bridge",
            Self::BridgeUplinkAttachment => "bridge_uplink_attachment",
            Self::BridgeTapAttachment => "bridge_tap_attachment",
            Self::Forwarding => "forwarding",
            Self::Nat => "nat",
            Self::FirecrackerInterface => "firecracker_interface",
            Self::DhcpLease => "dhcp_lease",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "tap" => Some(Self::Tap),
            "tap_address" => Some(Self::TapAddress),
            "bridge" => Some(Self::Bridge),
            "bridge_uplink_attachment" => Some(Self::BridgeUplinkAttachment),
            "bridge_tap_attachment" => Some(Self::BridgeTapAttachment),
            "forwarding" => Some(Self::Forwarding),
            "nat" => Some(Self::Nat),
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
    pub bridge_created_by_sdk: bool,
    pub uplink_attached_by_sdk: bool,
    pub forwarding_enabled_by_sdk: bool,
    pub nat_table_created_by_sdk: bool,
    pub nat_chain_created_by_sdk: bool,
    pub host_address_specs: Vec<String>,
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
