use std::net::IpAddr;

use crate::domain::lifecycle::NetworkMode;
use crate::domain::microvm::{NetworkResource, PersistedNetwork};
use crate::error::SdkError;

/// Network inputs that are stable for one VM creation attempt.
#[derive(Clone, Debug)]
pub(crate) struct NetworkRequest {
    pub vm_name: String,
    pub mode: NetworkMode,
    pub guest_mac: String,
    pub allow_existing_bridge: bool,
}

/// Network result with internal ownership and boot metadata.
#[derive(Clone, Debug)]
pub(crate) struct NetworkOutcome {
    pub persisted: PersistedNetwork,
    pub applied: Vec<NetworkResource>,
    pub skipped: Vec<NetworkResource>,
    pub requires_temporary_runtime: bool,
}

/// Replaceable host-network reconciliation boundary.
pub(crate) trait NetworkController: Send + Sync {
    fn lan_bridge_identity(&self) -> Result<Option<(String, String)>, SdkError>;

    fn configure(
        &self,
        request: &NetworkRequest,
        existing: Option<&PersistedNetwork>,
        used_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError>;

    fn cleanup(&self, network: &PersistedNetwork) -> Result<(), SdkError>;

    fn discover_dhcp_address(
        &self,
        bridge_name: &str,
        guest_mac: &str,
    ) -> Result<(IpAddr, String), SdkError>;

    fn with_dhcp_lease(
        &self,
        outcome: NetworkOutcome,
        address: IpAddr,
        lease_reference: String,
    ) -> NetworkOutcome;
}
