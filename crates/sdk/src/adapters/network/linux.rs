use std::net::{IpAddr, Ipv4Addr};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::domain::lifecycle::NetworkMode;
use crate::domain::microvm::{
    NetworkConfiguration, NetworkResource, PersistedNetwork, PersistedNetworkResource,
};
use crate::error::SdkError;
use crate::ports::network::{NetworkController, NetworkOutcome, NetworkRequest};

const PRIVATE_POOL_START: u32 = (172_u32 << 24) | (30_u32 << 16);
const PRIVATE_POOL_END: u32 = (172_u32 << 24) | (31_u32 << 16) | 0xff00;
const SDK_OWNERSHIP: &str = "sdk:taumaru";

/// Linux host-network adapter using `ip`, `sysctl`, and `nft` argument vectors.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LinuxNetworkController;

impl LinuxNetworkController {
    /// Returns a DHCP address observed for the VM MAC on the selected bridge.
    pub(crate) fn discover_dhcp_address(
        &self,
        bridge_name: &str,
        guest_mac: &str,
    ) -> Result<(IpAddr, String), SdkError> {
        let output = command_output("ip", &["-4", "neigh", "show", "dev", bridge_name])?;
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let Some(address) = fields.first() else {
                continue;
            };
            let has_mac = fields
                .windows(2)
                .any(|pair| pair[0] == "lladdr" && pair[1].eq_ignore_ascii_case(guest_mac));
            if has_mac && let Ok(address) = address.parse::<IpAddr>() {
                return Ok((address, format!("{guest_mac}:{address}")));
            }
        }
        Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "observe DHCP lease".to_owned(),
            resource: guest_mac.to_owned(),
            reason: "no active lease for the VM MAC was observed".to_owned(),
        })
    }

    /// Updates an in-memory outcome after a temporary DHCP observation.
    pub(crate) fn with_dhcp_lease(
        &self,
        mut outcome: NetworkOutcome,
        address: IpAddr,
        lease_reference: String,
    ) -> NetworkOutcome {
        outcome.persisted.config.guest_address = address;
        outcome.persisted.config.prefix_length = 24;
        outcome.persisted.dhcp_lease_reference = Some(lease_reference);
        outcome.persisted.resources.push(resource(
            NetworkResource::DhcpLease,
            &address.to_string(),
            &format!("{}:{address}", outcome.persisted.guest_mac),
        ));
        outcome.applied.push(NetworkResource::DhcpLease);
        outcome.requires_temporary_runtime = false;
        outcome
    }
}

impl NetworkController for LinuxNetworkController {
    fn lan_bridge_identity(&self) -> Result<Option<(String, String)>, SdkError> {
        let uplink = detect_default_uplink()?;
        Ok(Some((bridge_name(&uplink), uplink)))
    }

    fn configure(
        &self,
        request: &NetworkRequest,
        existing: Option<&PersistedNetwork>,
        used_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError> {
        match existing {
            Some(network) => self.reconcile(request, network),
            None => match request.mode {
                NetworkMode::HostOnly => self.create_host_only(request, used_addresses),
                NetworkMode::Lan => self.create_lan(request),
            },
        }
    }

    fn cleanup(&self, network: &PersistedNetwork) -> Result<(), SdkError> {
        let mut failures = Vec::new();
        if network.config.mode == NetworkMode::HostOnly {
            if let Err(error) = delete_link_if_present(&network.config.tap_name) {
                failures.push(error.to_string());
            }
            if let Err(error) = run_nft_delete(&network.config.tap_name) {
                failures.push(error.to_string());
            }
            if (network.nat_table_created_by_sdk || network.nat_chain_created_by_sdk)
                && let Err(error) = cleanup_created_nat_container(
                    !network.nat_table_created_by_sdk,
                    !network.nat_chain_created_by_sdk,
                )
            {
                failures.push(error.to_string());
            }
            if network.forwarding_enabled_by_sdk {
                match nft_chain_has_rules() {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
                        {
                            failures.push(error.to_string());
                        }
                    }
                    Err(error) => failures.push(error.to_string()),
                }
            }
        } else {
            if let Err(error) = delete_link_if_present(&network.config.tap_name) {
                failures.push(error.to_string());
            }
            let mut uplink_restored = true;
            if network.uplink_attached_by_sdk
                && let (Some(bridge), Some(uplink)) = (
                    network.config.bridge_name.as_deref(),
                    network.config.uplink_name.as_deref(),
                )
            {
                let restore_failures = restore_uplink_state(
                    uplink,
                    bridge,
                    &UplinkSnapshot {
                        address_specs: network.host_address_specs.clone(),
                        default_route_specs: network.default_route_specs.clone(),
                    },
                );
                if !restore_failures.is_empty() {
                    uplink_restored = false;
                    failures.extend(restore_failures);
                }
            }
            if network.bridge_created_by_sdk
                && uplink_restored
                && let Some(bridge) = network.config.bridge_name.as_deref()
                && let Err(error) = delete_link_if_present(bridge)
            {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SdkError::Cleanup {
                primary: "network rollback failed".to_owned(),
                failures,
            })
        }
    }

    fn discover_dhcp_address(
        &self,
        bridge_name: &str,
        guest_mac: &str,
    ) -> Result<(IpAddr, String), SdkError> {
        self.discover_dhcp_address(bridge_name, guest_mac)
    }

    fn with_dhcp_lease(
        &self,
        outcome: NetworkOutcome,
        address: IpAddr,
        lease_reference: String,
    ) -> NetworkOutcome {
        self.with_dhcp_lease(outcome, address, lease_reference)
    }
}

impl LinuxNetworkController {
    fn create_host_only(
        &self,
        request: &NetworkRequest,
        used_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError> {
        let mut occupied = used_addresses.to_vec();
        occupied.extend(live_ipv4_addresses()?);
        let (_network_base, gateway, guest) =
            allocate_subnet(&occupied).ok_or_else(|| SdkError::Network {
                mode: NetworkMode::HostOnly.to_string(),
                operation: "allocate private /30".to_owned(),
                resource: "172.30.0.0/16".to_owned(),
                reason: "the private address pool is exhausted or overlaps persisted state"
                    .to_owned(),
            })?;
        let tap_name = tap_name(&request.vm_name);
        let config = NetworkConfiguration {
            mode: NetworkMode::HostOnly,
            guest_address: IpAddr::V4(guest),
            prefix_length: 30,
            gateway: Some(IpAddr::V4(gateway)),
            tap_name: tap_name.clone(),
            bridge_name: None,
            uplink_name: None,
        };
        let desired_boot_parameters = format!(
            "ip={}::{}:{}::{}:off",
            guest, gateway, "255.255.255.252", tap_name
        );
        let mut applied = Vec::new();
        let forwarding_was_enabled = forwarding_enabled()?;
        let nat_table_existed = nft_table_exists()?;
        let nat_chain_existed = nat_table_existed && nft_chain_exists()?;
        let result = (|| {
            ensure_tap(&tap_name, &mut applied, NetworkMode::HostOnly)?;
            ensure_tap_address(&tap_name, gateway, &mut applied)?;
            ensure_forwarding(&mut applied)?;
            ensure_nat(&tap_name, &mut applied)?;
            applied.push(NetworkResource::FirecrackerInterface);
            let resources = vec![
                resource(NetworkResource::Tap, &tap_name, &tap_name),
                resource(
                    NetworkResource::TapAddress,
                    &format!("{tap_name}:{gateway}/30"),
                    &format!("{tap_name}:{gateway}/30"),
                ),
                resource(
                    NetworkResource::Forwarding,
                    "net.ipv4.ip_forward",
                    "net.ipv4.ip_forward=1",
                ),
                resource(
                    NetworkResource::Nat,
                    &tap_name,
                    &format!("{SDK_OWNERSHIP}:{tap_name}"),
                ),
                resource(
                    NetworkResource::FirecrackerInterface,
                    &format!("{tap_name}:{}", request.guest_mac),
                    &format!("tap={tap_name},mac={}", request.guest_mac),
                ),
            ];
            let persisted = PersistedNetwork {
                config,
                host_address: Some(IpAddr::V4(gateway)),
                guest_mac: request.guest_mac.clone(),
                dhcp_lease_reference: None,
                desired_boot_parameters,
                resources,
                bridge_created_by_sdk: false,
                uplink_attached_by_sdk: false,
                forwarding_enabled_by_sdk: !forwarding_was_enabled,
                nat_table_created_by_sdk: !nat_table_existed,
                nat_chain_created_by_sdk: !nat_chain_existed,
                host_address_specs: Vec::new(),
                default_route_specs: Vec::new(),
            };
            Ok(NetworkOutcome {
                persisted,
                applied: applied.clone(),
                skipped: Vec::new(),
                requires_temporary_runtime: false,
            })
        })();
        match result {
            Ok(outcome) => Ok(outcome),
            Err(primary) => {
                let cleanup_failures = cleanup_host_only_attempt(
                    &tap_name,
                    &applied,
                    forwarding_was_enabled,
                    nat_table_existed,
                    nat_chain_existed,
                );
                if cleanup_failures.is_empty() {
                    Err(primary)
                } else {
                    Err(SdkError::Cleanup {
                        primary: primary.to_string(),
                        failures: cleanup_failures,
                    })
                }
            }
        }
    }

    fn create_lan(&self, request: &NetworkRequest) -> Result<NetworkOutcome, SdkError> {
        let uplink = detect_default_uplink()?;
        let bridge = bridge_name(&uplink);
        let tap = tap_name(&request.vm_name);
        let mut applied = Vec::new();
        let bridge_existed = link_exists(&bridge)?;
        if bridge_existed && !request.allow_existing_bridge {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "claim managed bridge".to_owned(),
                resource: bridge,
                reason: "an existing bridge is not registered as SDK-owned".to_owned(),
            });
        }
        if let Err(primary) = ensure_bridge(&bridge, &mut applied) {
            let cleanup_failures =
                cleanup_lan_attempt(&bridge, &tap, bridge_existed, None, &applied);
            return if cleanup_failures.is_empty() {
                Err(primary)
            } else {
                Err(SdkError::Cleanup {
                    primary: primary.to_string(),
                    failures: cleanup_failures,
                })
            };
        }
        if let Err(primary) = ensure_tap(&tap, &mut applied, NetworkMode::Lan) {
            let cleanup_failures =
                cleanup_lan_attempt(&bridge, &tap, bridge_existed, None, &applied);
            return if cleanup_failures.is_empty() {
                Err(primary)
            } else {
                Err(SdkError::Cleanup {
                    primary: primary.to_string(),
                    failures: cleanup_failures,
                })
            };
        }
        let mut transition = None;
        let result = (|| {
            ensure_bridge_attachment(
                &tap,
                &bridge,
                NetworkResource::BridgeTapAttachment,
                &mut applied,
            )?;
            transition = ensure_uplink_on_bridge(&uplink, &bridge)?;
            if transition.is_some() {
                applied.push(NetworkResource::BridgeUplinkAttachment);
            }
            Ok::<(), SdkError>(())
        })();
        if let Err(primary) = result {
            let cleanup_failures =
                cleanup_lan_attempt(&bridge, &tap, bridge_existed, transition.as_ref(), &applied);
            return if cleanup_failures.is_empty() {
                Err(primary)
            } else {
                Err(SdkError::Cleanup {
                    primary: primary.to_string(),
                    failures: cleanup_failures,
                })
            };
        }
        applied.push(NetworkResource::FirecrackerInterface);
        let config = NetworkConfiguration {
            mode: NetworkMode::Lan,
            guest_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            prefix_length: 0,
            gateway: None,
            tap_name: tap.clone(),
            bridge_name: Some(bridge.clone()),
            uplink_name: Some(uplink.clone()),
        };
        let resources = vec![
            resource(
                NetworkResource::Bridge,
                &bridge,
                &format!("{bridge}:{uplink}"),
            ),
            resource(
                NetworkResource::BridgeUplinkAttachment,
                &format!("{bridge}:{uplink}"),
                &format!("bridge={bridge},uplink={uplink}"),
            ),
            resource(
                NetworkResource::BridgeTapAttachment,
                &format!("{bridge}:{tap}"),
                &format!("bridge={bridge},tap={tap}"),
            ),
            resource(NetworkResource::Tap, &tap, &tap),
            resource(
                NetworkResource::FirecrackerInterface,
                &format!("{tap}:{}", request.guest_mac),
                &format!("tap={tap},mac={}", request.guest_mac),
            ),
        ];
        Ok(NetworkOutcome {
            persisted: PersistedNetwork {
                config,
                host_address: None,
                guest_mac: request.guest_mac.clone(),
                dhcp_lease_reference: None,
                desired_boot_parameters: "ip=dhcp".to_owned(),
                resources,
                bridge_created_by_sdk: !bridge_existed,
                uplink_attached_by_sdk: transition.is_some(),
                forwarding_enabled_by_sdk: false,
                nat_table_created_by_sdk: false,
                nat_chain_created_by_sdk: false,
                host_address_specs: transition
                    .as_ref()
                    .map(|value| value.snapshot.address_specs.clone())
                    .unwrap_or_default(),
                default_route_specs: transition
                    .as_ref()
                    .map(|value| value.snapshot.default_route_specs.clone())
                    .unwrap_or_default(),
            },
            applied,
            skipped: Vec::new(),
            requires_temporary_runtime: true,
        })
    }

    fn reconcile(
        &self,
        request: &NetworkRequest,
        network: &PersistedNetwork,
    ) -> Result<NetworkOutcome, SdkError> {
        if request.mode != network.config.mode || request.guest_mac != network.guest_mac {
            return Err(SdkError::Network {
                mode: request.mode.to_string(),
                operation: "validate persisted network identity".to_owned(),
                resource: request.vm_name.clone(),
                reason: "persisted network mode or MAC does not match the request".to_owned(),
            });
        }
        let mut updated = network.clone();
        let (applied, skipped) = match request.mode {
            NetworkMode::HostOnly => self.reconcile_host_only(&mut updated)?,
            NetworkMode::Lan => self.reconcile_lan(&mut updated, &request.vm_name)?,
        };
        Ok(NetworkOutcome {
            persisted: updated,
            applied,
            skipped,
            requires_temporary_runtime: request.mode == NetworkMode::Lan
                && (network.dhcp_lease_reference.is_none()
                    || network.config.guest_address.is_unspecified()),
        })
    }

    fn reconcile_host_only(
        &self,
        network: &mut PersistedNetwork,
    ) -> Result<(Vec<NetworkResource>, Vec<NetworkResource>), SdkError> {
        let Some(IpAddr::V4(gateway)) = network.host_address else {
            return Err(SdkError::Network {
                mode: NetworkMode::HostOnly.to_string(),
                operation: "validate persisted host address".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "host-only network has no persisted IPv4 gateway".to_owned(),
            });
        };
        if network.config.prefix_length != 30 {
            return Err(SdkError::Network {
                mode: NetworkMode::HostOnly.to_string(),
                operation: "validate persisted host prefix".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "host-only network must use a /30 prefix".to_owned(),
            });
        }
        let tap_existed = link_exists(&network.config.tap_name)?;
        let forwarding_was_enabled = forwarding_enabled()?;
        let nat_table_existed = nft_table_exists()?;
        let nat_chain_existed = nat_table_existed && nft_chain_exists()?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let result = (|| {
            reconcile_tap(&network.config.tap_name, &mut applied, &mut skipped)?;
            reconcile_tap_address(
                &network.config.tap_name,
                gateway,
                &mut applied,
                &mut skipped,
            )?;
            reconcile_forwarding(&mut applied, &mut skipped)?;
            reconcile_nat(&network.config.tap_name, &mut applied, &mut skipped)?;
            if applied.contains(&NetworkResource::Forwarding) {
                network.forwarding_enabled_by_sdk = true;
            }
            if applied.contains(&NetworkResource::Nat) {
                network.nat_table_created_by_sdk |= !nat_table_existed;
                network.nat_chain_created_by_sdk |= !nat_chain_existed;
            }
            skipped.push(NetworkResource::FirecrackerInterface);
            Ok::<(), SdkError>(())
        })();
        match result {
            Ok(()) => Ok((applied, skipped)),
            Err(primary) => {
                let cleanup_failures = cleanup_reconciled_host_only(
                    &network.config.tap_name,
                    gateway,
                    tap_existed,
                    &applied,
                    forwarding_was_enabled,
                    nat_table_existed,
                    nat_chain_existed,
                );
                if cleanup_failures.is_empty() {
                    Err(primary)
                } else {
                    Err(SdkError::Cleanup {
                        primary: primary.to_string(),
                        failures: cleanup_failures,
                    })
                }
            }
        }
    }

    fn reconcile_lan(
        &self,
        network: &mut PersistedNetwork,
        vm_name: &str,
    ) -> Result<(Vec<NetworkResource>, Vec<NetworkResource>), SdkError> {
        let bridge = network
            .config
            .bridge_name
            .clone()
            .ok_or_else(|| SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "load persisted bridge".to_owned(),
                resource: vm_name.to_owned(),
                reason: "LAN network has no bridge name".to_owned(),
            })?;
        let uplink = network
            .config
            .uplink_name
            .clone()
            .ok_or_else(|| SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "load persisted uplink".to_owned(),
                resource: vm_name.to_owned(),
                reason: "LAN network has no uplink name".to_owned(),
            })?;
        let bridge_existed = link_exists(&bridge)?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut transition = None;
        let result = (|| {
            reconcile_bridge(&bridge, &mut applied, &mut skipped)?;
            reconcile_tap(&network.config.tap_name, &mut applied, &mut skipped)?;
            reconcile_attachment(
                &network.config.tap_name,
                &bridge,
                NetworkResource::BridgeTapAttachment,
                &mut applied,
                &mut skipped,
            )?;
            transition = ensure_uplink_on_bridge(&uplink, &bridge)?;
            if let Some(value) = transition.as_ref() {
                network.uplink_attached_by_sdk = true;
                network.host_address_specs = value.snapshot.address_specs.clone();
                network.default_route_specs = value.snapshot.default_route_specs.clone();
                applied.push(NetworkResource::BridgeUplinkAttachment);
            } else {
                skipped.push(NetworkResource::BridgeUplinkAttachment);
            }
            if network.dhcp_lease_reference.is_some()
                && !network.config.guest_address.is_unspecified()
            {
                skipped.push(NetworkResource::DhcpLease);
            }
            if !bridge_existed {
                network.bridge_created_by_sdk = true;
            }
            skipped.push(NetworkResource::FirecrackerInterface);
            Ok::<(), SdkError>(())
        })();
        match result {
            Ok(()) => Ok((applied, skipped)),
            Err(primary) => {
                let cleanup_failures = cleanup_lan_attempt(
                    &bridge,
                    &network.config.tap_name,
                    bridge_existed,
                    transition.as_ref(),
                    &applied,
                );
                if cleanup_failures.is_empty() {
                    Err(primary)
                } else {
                    Err(SdkError::Cleanup {
                        primary: primary.to_string(),
                        failures: cleanup_failures,
                    })
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct UplinkSnapshot {
    address_specs: Vec<String>,
    default_route_specs: Vec<Vec<String>>,
}

#[derive(Clone, Debug)]
struct UplinkTransition {
    uplink: String,
    snapshot: UplinkSnapshot,
}

fn allocate_subnet(
    used_addresses: &[(String, IpAddr, String)],
) -> Option<(Ipv4Addr, Ipv4Addr, Ipv4Addr)> {
    let mut base = PRIVATE_POOL_START;
    while base <= PRIVATE_POOL_END {
        let gateway = Ipv4Addr::from(base + 1);
        let guest = Ipv4Addr::from(base + 2);
        let conflict = used_addresses.iter().any(|(_, address, prefix)| {
            let IpAddr::V4(value) = address else {
                return false;
            };
            let prefix = prefix.parse::<u8>().unwrap_or(32);
            let value = u32::from(*value);
            if prefix <= 30 {
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                (value & mask) == (base & mask)
            } else {
                (value & !3) == base
            }
        });
        if !conflict {
            return Some((Ipv4Addr::from(base), gateway, guest));
        }
        base = base.saturating_add(4);
    }
    None
}

fn ensure_uplink_on_bridge(
    uplink: &str,
    bridge: &str,
) -> Result<Option<UplinkTransition>, SdkError> {
    if let Some(current_master) = link_master(uplink)? {
        if current_master == bridge {
            return Ok(None);
        }
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "claim default uplink".to_owned(),
            resource: uplink.to_owned(),
            reason: format!(
                "the default uplink is already attached to foreign bridge {current_master}"
            ),
        });
    }
    let snapshot = capture_uplink_state(uplink)?;
    match move_uplink_to_bridge(uplink, bridge, &snapshot) {
        Ok(()) => Ok(Some(UplinkTransition {
            uplink: uplink.to_owned(),
            snapshot,
        })),
        Err(primary) => {
            let cleanup_failures = restore_uplink_state(uplink, bridge, &snapshot);
            if cleanup_failures.is_empty() {
                Err(primary)
            } else {
                Err(SdkError::Cleanup {
                    primary: primary.to_string(),
                    failures: cleanup_failures,
                })
            }
        }
    }
}

fn capture_uplink_state(uplink: &str) -> Result<UplinkSnapshot, SdkError> {
    let output = command_output("ip", &["-o", "addr", "show", "dev", uplink])?;
    if !output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "inspect uplink addresses".to_owned(),
            resource: uplink.to_owned(),
            reason: format!(
                "interface is unavailable (ip exited with {})",
                output.status
            ),
        });
    }
    let mut address_specs = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(index) = fields
            .iter()
            .position(|field| *field == "inet" || *field == "inet6")
        else {
            continue;
        };
        if let Some(address) = fields.get(index + 1) {
            address_specs.push((*address).to_owned());
        }
    }

    let mut default_route_specs = Vec::new();
    for family in ["-4", "-6"] {
        let output = command_output("ip", &[family, "route", "show", "default", "dev", uplink])?;
        if !output.status.success() {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "inspect uplink default route".to_owned(),
                resource: uplink.to_owned(),
                reason: format!("ip exited with {}", output.status),
            });
        }
        default_route_specs.extend(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| {
                    let mut fields = vec![family.to_owned()];
                    fields.extend(line.split_whitespace().map(str::to_owned));
                    fields
                })
                .filter(|fields| !fields.is_empty()),
        );
    }
    Ok(UplinkSnapshot {
        address_specs,
        default_route_specs,
    })
}

fn move_uplink_to_bridge(
    uplink: &str,
    bridge: &str,
    snapshot: &UplinkSnapshot,
) -> Result<(), SdkError> {
    for address in &snapshot.address_specs {
        if !interface_has_address(bridge, address)? {
            run_ip(&["addr", "add", address, "dev", bridge])?;
        }
    }
    run_ip(&["link", "set", "dev", uplink, "master", bridge])?;
    for address in &snapshot.address_specs {
        if interface_has_address(uplink, address)? {
            run_ip(&["addr", "del", address, "dev", uplink])?;
        }
    }
    for route in &snapshot.default_route_specs {
        let arguments = route_with_device(route, uplink, bridge);
        run_ip_owned(&arguments)?;
    }
    Ok(())
}

fn restore_uplink_state(uplink: &str, bridge: &str, snapshot: &UplinkSnapshot) -> Vec<String> {
    let mut failures = Vec::new();
    if link_master(uplink)
        .map_err(|error| failures.push(error.to_string()))
        .ok()
        .flatten()
        .is_some_and(|master| master == bridge)
        && let Err(error) = run_ip(&["link", "set", "dev", uplink, "nomaster"])
    {
        failures.push(error.to_string());
    }
    for address in &snapshot.address_specs {
        if let Ok(true) = interface_has_address(bridge, address)
            && let Err(error) = run_ip(&["addr", "del", address, "dev", bridge])
        {
            failures.push(error.to_string());
        }
        match interface_has_address(uplink, address) {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = run_ip(&["addr", "add", address, "dev", uplink]) {
                    failures.push(error.to_string());
                }
            }
            Err(error) => failures.push(error.to_string()),
        }
    }
    for route in &snapshot.default_route_specs {
        let arguments = route_with_device(route, bridge, uplink);
        if let Err(error) = run_ip_owned(&arguments) {
            failures.push(error.to_string());
        }
    }
    failures
}

fn cleanup_lan_attempt(
    bridge: &str,
    tap: &str,
    bridge_existed: bool,
    transition: Option<&UplinkTransition>,
    applied: &[NetworkResource],
) -> Vec<String> {
    let mut failures = Vec::new();
    if applied.contains(&NetworkResource::Tap)
        && let Err(error) = delete_link_if_present(tap)
    {
        failures.push(error.to_string());
    }
    if let Some(transition) = transition {
        failures.extend(restore_uplink_state(
            &transition.uplink,
            bridge,
            &transition.snapshot,
        ));
    }
    if !bridge_existed && let Err(error) = delete_link_if_present(bridge) {
        failures.push(error.to_string());
    }
    failures
}

fn interface_has_address(interface: &str, address: &str) -> Result<bool, SdkError> {
    let output = command_output("ip", &["-o", "addr", "show", "dev", interface])?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .any(|field| field == address))
}

fn route_with_device(route: &[String], current_device: &str, replacement: &str) -> Vec<String> {
    let (family, mut index) = match route.first().map(String::as_str) {
        Some("-4") => (Some("-4"), 1),
        Some("-6") => (Some("-6"), 1),
        _ => (None, 0),
    };
    let mut arguments = Vec::new();
    if let Some(family) = family {
        arguments.push(family.to_owned());
    }
    arguments.push("route".to_owned());
    arguments.push("replace".to_owned());
    while index < route.len() {
        if route[index] == "dev"
            && route
                .get(index + 1)
                .is_some_and(|device| device == current_device)
        {
            arguments.push("dev".to_owned());
            arguments.push(replacement.to_owned());
            index += 2;
        } else {
            arguments.push(route[index].clone());
            index += 1;
        }
    }
    arguments
}

fn live_ipv4_addresses() -> Result<Vec<(String, IpAddr, String)>, SdkError> {
    let address_output = command_output("ip", &["-4", "-o", "addr", "show"])?;
    if !address_output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::HostOnly.to_string(),
            operation: "inspect live IPv4 addresses".to_owned(),
            resource: "host interfaces".to_owned(),
            reason: format!("ip exited with {}", address_output.status),
        });
    }
    let mut addresses = Vec::new();
    for line in String::from_utf8_lossy(&address_output.stdout).lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(inet_index) = fields.iter().position(|field| *field == "inet") else {
            continue;
        };
        let Some(address_with_prefix) = fields.get(inet_index + 1) else {
            continue;
        };
        let Some((address, prefix)) = address_with_prefix.split_once('/') else {
            continue;
        };
        let Ok(address) = address.parse::<Ipv4Addr>() else {
            continue;
        };
        let interface = fields
            .get(1)
            .map(|field| field.trim_end_matches(':'))
            .unwrap_or("unknown");
        addresses.push((
            format!("live:{interface}"),
            IpAddr::V4(address),
            prefix.to_owned(),
        ));
    }

    let route_output = command_output("ip", &["-4", "route", "show"])?;
    if !route_output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::HostOnly.to_string(),
            operation: "inspect live IPv4 routes".to_owned(),
            resource: "host routes".to_owned(),
            reason: format!("ip exited with {}", route_output.status),
        });
    }
    for line in String::from_utf8_lossy(&route_output.stdout).lines() {
        let Some(route) = line.split_whitespace().next() else {
            continue;
        };
        let Some((network, prefix)) = route.split_once('/') else {
            continue;
        };
        let Ok(network) = network.parse::<Ipv4Addr>() else {
            continue;
        };
        addresses.push((
            "live-route".to_owned(),
            IpAddr::V4(network),
            prefix.to_owned(),
        ));
    }
    Ok(addresses)
}

fn cleanup_host_only_attempt(
    tap: &str,
    applied: &[NetworkResource],
    forwarding_was_enabled: bool,
    nat_table_existed: bool,
    nat_chain_existed: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if applied.contains(&NetworkResource::Nat)
        && let Err(error) = run_nft_delete(tap)
    {
        failures.push(error.to_string());
    }
    if let Err(error) = cleanup_created_nat_container(nat_table_existed, nat_chain_existed) {
        failures.push(error.to_string());
    }
    if applied.contains(&NetworkResource::Tap)
        && let Err(error) = delete_link_if_present(tap)
    {
        failures.push(error.to_string());
    }
    if applied.contains(&NetworkResource::Forwarding)
        && !forwarding_was_enabled
        && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
    {
        failures.push(error.to_string());
    }
    failures
}

fn cleanup_reconciled_host_only(
    tap: &str,
    gateway: Ipv4Addr,
    tap_existed: bool,
    applied: &[NetworkResource],
    forwarding_was_enabled: bool,
    nat_table_existed: bool,
    nat_chain_existed: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if applied.contains(&NetworkResource::Nat)
        && let Err(error) = run_nft_delete(tap)
    {
        failures.push(error.to_string());
    }
    if let Err(error) = cleanup_created_nat_container(nat_table_existed, nat_chain_existed) {
        failures.push(error.to_string());
    }
    if applied.contains(&NetworkResource::TapAddress)
        && let Err(error) = run_ip(&["addr", "del", &format!("{gateway}/30"), "dev", tap])
    {
        failures.push(error.to_string());
    }
    if !tap_existed
        && applied.contains(&NetworkResource::Tap)
        && let Err(error) = delete_link_if_present(tap)
    {
        failures.push(error.to_string());
    }
    if applied.contains(&NetworkResource::Forwarding)
        && !forwarding_was_enabled
        && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
    {
        failures.push(error.to_string());
    }
    failures
}

fn cleanup_created_nat_container(table_existed: bool, chain_existed: bool) -> Result<(), SdkError> {
    if table_existed && chain_existed {
        return Ok(());
    }
    if !nft_table_exists()? {
        return Ok(());
    }
    if !chain_existed && nft_chain_exists()? {
        if !nft_chain_has_rules()? {
            run_command(
                "nft",
                &["delete", "chain", "ip", "taumaru_microvm", "postrouting"],
            )?;
        } else {
            return Ok(());
        }
    }
    if !table_existed && !nft_chain_exists()? && nft_table_has_no_chains()? {
        run_command("nft", &["delete", "table", "ip", "taumaru_microvm"])?;
    }
    Ok(())
}

fn tap_name(vm_name: &str) -> String {
    let digest = Sha256::digest(vm_name.as_bytes());
    format!(
        "tm-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

pub(crate) fn guest_mac(vm_name: &str) -> String {
    let digest = Sha256::digest(vm_name.as_bytes());
    format!(
        "02:fc:{:02x}:{:02x}:{:02x}:{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

fn bridge_name(uplink: &str) -> String {
    let digest = Sha256::digest(uplink.as_bytes());
    format!(
        "tb-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

fn resource(
    resource: NetworkResource,
    identity: &str,
    fingerprint: &str,
) -> PersistedNetworkResource {
    PersistedNetworkResource {
        resource,
        identity: identity.to_owned(),
        fingerprint: fingerprint.to_owned(),
        ownership: SDK_OWNERSHIP.to_owned(),
        adapter_handle: None,
        last_observed: "desired".to_owned(),
    }
}

fn detect_default_uplink() -> Result<String, SdkError> {
    let output = command_output("ip", &["route", "show", "default"])?;
    let output_text = String::from_utf8_lossy(&output.stdout);
    let fields = output_text
        .lines()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>();
    let Some(index) = fields.iter().position(|field| *field == "dev") else {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "detect default uplink".to_owned(),
            resource: "default route".to_owned(),
            reason: "the host has no default-route device".to_owned(),
        });
    };
    let Some(uplink) = fields.get(index + 1) else {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "detect default uplink".to_owned(),
            resource: "default route".to_owned(),
            reason: "the default route has no device name".to_owned(),
        });
    };
    if uplink.is_empty() || uplink.len() > 15 {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "validate default uplink".to_owned(),
            resource: (*uplink).to_owned(),
            reason: "the interface name is not valid for Linux networking".to_owned(),
        });
    }
    Ok((*uplink).to_owned())
}

fn ensure_tap(
    tap: &str,
    applied: &mut Vec<NetworkResource>,
    mode: NetworkMode,
) -> Result<(), SdkError> {
    if link_exists(tap)? {
        return Err(SdkError::Network {
            mode: mode.to_string(),
            operation: "claim TAP interface".to_owned(),
            resource: tap.to_owned(),
            reason: "an unowned interface with the deterministic name already exists".to_owned(),
        });
    }
    run_ip(&["tuntap", "add", "dev", tap, "mode", "tap"])?;
    applied.push(NetworkResource::Tap);
    run_ip(&["link", "set", "dev", tap, "up"])?;
    Ok(())
}

fn ensure_tap_address(
    tap: &str,
    gateway: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    run_ip(&["addr", "add", &format!("{gateway}/30"), "dev", tap])?;
    applied.push(NetworkResource::TapAddress);
    Ok(())
}

fn ensure_forwarding(applied: &mut Vec<NetworkResource>) -> Result<(), SdkError> {
    if !forwarding_enabled()? {
        run_command("sysctl", &["-w", "net.ipv4.ip_forward=1"])?;
        applied.push(NetworkResource::Forwarding);
    }
    Ok(())
}

fn forwarding_enabled() -> Result<bool, SdkError> {
    let output = command_output("sysctl", &["-n", "net.ipv4.ip_forward"])?;
    if !output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::HostOnly.to_string(),
            operation: "inspect IPv4 forwarding".to_owned(),
            resource: "net.ipv4.ip_forward".to_owned(),
            reason: format!("sysctl exited with {}", output.status),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn ensure_nat(tap: &str, applied: &mut Vec<NetworkResource>) -> Result<(), SdkError> {
    let table_exists = nft_table_exists()?;
    if !table_exists {
        run_command("nft", &["add", "table", "ip", "taumaru_microvm"])?;
        run_command(
            "nft",
            &[
                "add",
                "chain",
                "ip",
                "taumaru_microvm",
                "postrouting",
                "{",
                "type",
                "nat",
                "hook",
                "postrouting",
                "priority",
                "100",
                ";",
                "}",
            ],
        )?;
    } else if !nft_chain_exists()? {
        run_command(
            "nft",
            &[
                "add",
                "chain",
                "ip",
                "taumaru_microvm",
                "postrouting",
                "{",
                "type",
                "nat",
                "hook",
                "postrouting",
                "priority",
                "100",
                ";",
                "}",
            ],
        )?;
    }
    let comment = format!("comment {SDK_OWNERSHIP}:{tap}");
    run_command(
        "nft",
        &[
            "add",
            "rule",
            "ip",
            "taumaru_microvm",
            "postrouting",
            "oifname",
            "!=",
            tap,
            "ip",
            "saddr",
            "172.30.0.0/16",
            "masquerade",
            &comment,
        ],
    )?;
    applied.push(NetworkResource::Nat);
    Ok(())
}

fn ensure_bridge(bridge: &str, applied: &mut Vec<NetworkResource>) -> Result<(), SdkError> {
    if link_exists(bridge)? {
        if !link_is_bridge(bridge)? {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "claim managed bridge".to_owned(),
                resource: bridge.to_owned(),
                reason: "an existing non-bridge interface uses the SDK-managed name".to_owned(),
            });
        }
    } else {
        run_ip(&["link", "add", "name", bridge, "type", "bridge"])?;
        applied.push(NetworkResource::Bridge);
        run_ip(&["link", "set", "dev", bridge, "up"])?;
    }
    Ok(())
}

fn ensure_bridge_attachment(
    member: &str,
    bridge: &str,
    resource_kind: NetworkResource,
    applied: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    if let Some(current_master) = link_master(member)? {
        if current_master == bridge {
            return Ok(());
        }
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "claim bridge attachment".to_owned(),
            resource: member.to_owned(),
            reason: format!("the interface is already attached to foreign bridge {current_master}"),
        });
    }
    run_ip(&["link", "set", "dev", member, "master", bridge])?;
    applied.push(resource_kind);
    Ok(())
}

fn reconcile_tap(
    tap: &str,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    if link_exists(tap)? {
        if !link_is_tap(tap)? {
            return Err(SdkError::Network {
                mode: NetworkMode::HostOnly.to_string(),
                operation: "validate persisted TAP interface".to_owned(),
                resource: tap.to_owned(),
                reason: "the persisted interface name is owned by a non-TAP device".to_owned(),
            });
        }
        skipped.push(NetworkResource::Tap);
        Ok(())
    } else {
        run_ip(&["tuntap", "add", "dev", tap, "mode", "tap"])?;
        applied.push(NetworkResource::Tap);
        run_ip(&["link", "set", "dev", tap, "up"])?;
        Ok(())
    }
}

fn reconcile_tap_address(
    tap: &str,
    gateway: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let output = command_output("ip", &["-4", "addr", "show", "dev", tap])?;
    if !output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::HostOnly.to_string(),
            operation: "inspect persisted TAP address".to_owned(),
            resource: tap.to_owned(),
            reason: format!(
                "interface is unavailable (ip exited with {})",
                output.status
            ),
        });
    }
    let desired = format!("{gateway}/30");
    let output_text = String::from_utf8_lossy(&output.stdout);
    let fields = output_text.split_whitespace().collect::<Vec<_>>();
    let addresses = fields
        .windows(2)
        .filter_map(|pair| (pair[0] == "inet").then_some(pair[1]))
        .collect::<Vec<_>>();
    if addresses.iter().any(|address| *address == desired) {
        skipped.push(NetworkResource::TapAddress);
    } else if !addresses.is_empty() {
        return Err(SdkError::Network {
            mode: NetworkMode::HostOnly.to_string(),
            operation: "validate persisted TAP address ownership".to_owned(),
            resource: tap.to_owned(),
            reason: format!(
                "the TAP has a conflicting IPv4 address: {}",
                addresses.join(", ")
            ),
        });
    } else {
        run_ip(&["addr", "add", &desired, "dev", tap])?;
        applied.push(NetworkResource::TapAddress);
    }
    Ok(())
}

fn reconcile_forwarding(
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let output = command_output("sysctl", &["-n", "net.ipv4.ip_forward"])?;
    if String::from_utf8_lossy(&output.stdout).trim() == "1" {
        skipped.push(NetworkResource::Forwarding);
    } else {
        run_command("sysctl", &["-w", "net.ipv4.ip_forward=1"])?;
        applied.push(NetworkResource::Forwarding);
    }
    Ok(())
}

fn reconcile_nat(
    tap: &str,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    if nft_rule_exists(tap)? {
        skipped.push(NetworkResource::Nat);
        Ok(())
    } else {
        ensure_nat(tap, applied)
    }
}

fn reconcile_bridge(
    bridge: &str,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    if link_exists(bridge)? {
        if !link_is_bridge(bridge)? {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "validate persisted bridge".to_owned(),
                resource: bridge.to_owned(),
                reason: "the persisted bridge name is owned by a non-bridge device".to_owned(),
            });
        }
        skipped.push(NetworkResource::Bridge);
        return Ok(());
    }
    ensure_bridge(bridge, applied)
}

fn reconcile_attachment(
    member: &str,
    bridge: &str,
    resource_kind: NetworkResource,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let output = command_output("ip", &["link", "show", "dev", member])?;
    if !output.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "inspect bridge attachment".to_owned(),
            resource: member.to_owned(),
            reason: format!(
                "interface does not exist (ip exited with {})",
                output.status
            ),
        });
    }
    let bridge_marker = format!("master {bridge}");
    if String::from_utf8_lossy(&output.stdout).contains(&bridge_marker) {
        skipped.push(resource_kind);
    } else if let Some(current_master) = extract_master(&String::from_utf8_lossy(&output.stdout)) {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "validate bridge attachment ownership".to_owned(),
            resource: member.to_owned(),
            reason: format!("the interface is already attached to foreign bridge {current_master}"),
        });
    } else {
        run_ip(&["link", "set", "dev", member, "master", bridge])?;
        applied.push(resource_kind);
    }
    Ok(())
}

fn link_exists(name: &str) -> Result<bool, SdkError> {
    Ok(link_output(name)?.is_some())
}

fn link_output(name: &str) -> Result<Option<String>, SdkError> {
    let output = Command::new("ip")
        .args(["-d", "link", "show", "dev", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "ip".to_owned(),
            reason: error.to_string(),
        })?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else {
        Ok(None)
    }
}

fn link_is_bridge(name: &str) -> Result<bool, SdkError> {
    Ok(link_output(name)?.is_some_and(|output| {
        output
            .lines()
            .any(|line| line.trim_start().starts_with("bridge "))
    }))
}

fn link_is_tap(name: &str) -> Result<bool, SdkError> {
    Ok(link_output(name)?.is_some_and(|output| {
        output.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("tun ") || line.starts_with("tap ") || line.contains(" tun ")
        })
    }))
}

fn link_master(name: &str) -> Result<Option<String>, SdkError> {
    Ok(link_output(name)?.and_then(|output| extract_master(&output)))
}

fn extract_master(output: &str) -> Option<String> {
    let fields = output.split_whitespace().collect::<Vec<_>>();
    fields
        .windows(2)
        .find(|pair| pair[0] == "master")
        .map(|pair| pair[1].to_owned())
}

fn command_output(program: &str, arguments: &[&str]) -> Result<std::process::Output, SdkError> {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: program.to_owned(),
            reason: error.to_string(),
        })
}

fn run_command(program: &str, arguments: &[&str]) -> Result<(), SdkError> {
    let output = command_output(program, arguments)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(SdkError::HostCommand {
            program: program.to_owned(),
            reason: format!("command exited with {}", output.status),
        })
    }
}

fn run_ip(arguments: &[&str]) -> Result<(), SdkError> {
    run_command("ip", arguments)
}

fn run_ip_owned(arguments: &[String]) -> Result<(), SdkError> {
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    run_ip(&arguments)
}

fn delete_link_if_present(name: &str) -> Result<(), SdkError> {
    if !link_exists(name)? {
        return Ok(());
    }
    match run_ip(&["link", "delete", "dev", name]) {
        Ok(()) => Ok(()),
        Err(_primary) if !link_exists(name)? => Ok(()),
        Err(primary) => Err(primary),
    }
}

fn run_nft_delete(tap: &str) -> Result<(), SdkError> {
    let output = command_output(
        "nft",
        &[
            "-a",
            "list",
            "chain",
            "ip",
            "taumaru_microvm",
            "postrouting",
        ],
    )?;
    if !output.status.success() {
        return Ok(());
    }
    let marker = format!("{SDK_OWNERSHIP}:{tap}");
    let Some(handle) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            if !line.contains(&marker) {
                return None;
            }
            line.split("# handle ")
                .nth(1)
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned)
        })
    else {
        return Ok(());
    };
    run_command(
        "nft",
        &[
            "delete",
            "rule",
            "ip",
            "taumaru_microvm",
            "postrouting",
            "handle",
            &handle,
        ],
    )
}

fn nft_table_exists() -> Result<bool, SdkError> {
    Ok(
        command_output("nft", &["list", "table", "ip", "taumaru_microvm"])?
            .status
            .success(),
    )
}

fn nft_chain_exists() -> Result<bool, SdkError> {
    Ok(command_output(
        "nft",
        &["list", "chain", "ip", "taumaru_microvm", "postrouting"],
    )?
    .status
    .success())
}

fn nft_rule_exists(tap: &str) -> Result<bool, SdkError> {
    let output = command_output(
        "nft",
        &[
            "-a",
            "list",
            "chain",
            "ip",
            "taumaru_microvm",
            "postrouting",
        ],
    )?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).contains(&format!("{SDK_OWNERSHIP}:{tap}")))
}

fn nft_chain_has_rules() -> Result<bool, SdkError> {
    let output = command_output(
        "nft",
        &[
            "-a",
            "list",
            "chain",
            "ip",
            "taumaru_microvm",
            "postrouting",
        ],
    )?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains("# handle ")))
}

fn nft_table_has_no_chains() -> Result<bool, SdkError> {
    let output = command_output("nft", &["list", "table", "ip", "taumaru_microvm"])?;
    if !output.status.success() {
        return Ok(true);
    }
    Ok(!String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim_start().starts_with("chain ")))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::{allocate_subnet, bridge_name, guest_mac, route_with_device, tap_name};

    #[test]
    fn allocates_the_next_private_subnet_after_a_persisted_collision() {
        let used = vec![(
            "existing".to_owned(),
            IpAddr::V4(Ipv4Addr::new(172, 30, 0, 2)),
            "tm-existing".to_owned(),
        )];

        assert_eq!(
            allocate_subnet(&used),
            Some((
                Ipv4Addr::new(172, 30, 0, 4),
                Ipv4Addr::new(172, 30, 0, 5),
                Ipv4Addr::new(172, 30, 0, 6),
            ))
        );
    }

    #[test]
    fn names_and_macs_are_stable_and_fit_linux_limits() {
        assert_eq!(tap_name("build_vm"), tap_name("build_vm"));
        assert_eq!(guest_mac("build_vm"), guest_mac("build_vm"));
        assert!(tap_name("build_vm").len() <= 15);
        assert!(bridge_name("enp0s3").len() <= 15);
        assert!(guest_mac("build_vm").starts_with("02:fc:"));
    }

    #[test]
    fn route_rewrite_preserves_the_ip_family() {
        let route = vec![
            "-6".to_owned(),
            "default".to_owned(),
            "via".to_owned(),
            "2001:db8::1".to_owned(),
            "dev".to_owned(),
            "eth0".to_owned(),
        ];

        assert_eq!(
            route_with_device(&route, "eth0", "tb-bridge"),
            vec![
                "-6",
                "route",
                "replace",
                "default",
                "via",
                "2001:db8::1",
                "dev",
                "tb-bridge"
            ]
        );
    }
}
