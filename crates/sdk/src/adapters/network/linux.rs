use std::net::{IpAddr, Ipv4Addr};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::domain::lifecycle::NetworkMode;
use crate::domain::microvm::{
    NetworkConfiguration, NetworkResource, PersistedNetwork, PersistedNetworkResource,
};
use crate::error::SdkError;
use crate::ports::network::{
    LanAddressOffer, NetworkController, NetworkOutcome, NetworkRequest, UplinkIdentity,
};

const PRIVATE_POOL_START: u32 = (172_u32 << 24) | (30_u32 << 16);
const PRIVATE_POOL_END: u32 = (172_u32 << 24) | (31_u32 << 16) | 0xff00;
const SDK_OWNERSHIP: &str = "sdk:taumaru";
const PRIVILEGE_HINT: &str = "the operation requires elevated network privileges (CAP_NET_ADMIN); run as root or grant the capability";
/// Interface name inside the guest for Firecracker's network device.
const GUEST_INTERFACE: &str = "eth0";

/// Linux host-network adapter using `ip`, `sysctl`, and `nft` argument vectors.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LinuxNetworkController;

impl NetworkController for LinuxNetworkController {
    fn detect_uplink(&self) -> Result<UplinkIdentity, SdkError> {
        detect_uplink_identity()
    }

    fn select_lan_offer(
        &self,
        uplink: &UplinkIdentity,
        lan_override: Option<Ipv4Addr>,
        previous: Option<Ipv4Addr>,
        used_addresses: &[(String, IpAddr, String)],
    ) -> Result<LanAddressOffer, SdkError> {
        let _ = uplink.prefix_length;
        let offer = select_lan_offer(uplink, lan_override, previous, used_addresses)?;
        let _ = offer.source;
        Ok(offer)
    }

    fn configure(
        &self,
        request: &NetworkRequest,
        existing: Option<&PersistedNetwork>,
        used_addresses: &[(String, IpAddr, String)],
        used_lan_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError> {
        match existing {
            Some(network) => self.reconcile(request, network),
            None => match request.mode {
                NetworkMode::HostOnly => self.create_host_only(request, used_addresses),
                NetworkMode::Lan => self.create_lan(request, used_lan_addresses),
            },
        }
    }

    fn cleanup(&self, network: &PersistedNetwork) -> Result<(), SdkError> {
        let mut failures = Vec::new();
        if network.config.mode == NetworkMode::HostOnly {
            if let Err(error) = delete_link_if_present(&network.config.tap_name) {
                failures.push(error.to_string());
            }
            if network.forwarding_enabled_by_sdk
                && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
            {
                failures.push(error.to_string());
            }
        } else {
            let Some(IpAddr::V4(lan)) = network.config.lan_address else {
                failures.push("routed LAN network has no committed LAN address".to_owned());
                return Err(SdkError::Cleanup {
                    primary: "network rollback failed".to_owned(),
                    failures,
                });
            };
            let uplink = network.config.uplink_name.clone().unwrap_or_default();
            let IpAddr::V4(private_guest) = network.config.guest_address else {
                failures.push("routed LAN network has no private guest address".to_owned());
                return Err(SdkError::Cleanup {
                    primary: "network rollback failed".to_owned(),
                    failures,
                });
            };
            let private_network = private_guest_network(private_guest);
            failures.extend(cleanup_routed_attempt(
                &uplink,
                &network.config.tap_name,
                lan,
                private_network,
                private_guest,
                &network
                    .resources
                    .iter()
                    .map(|item| item.resource)
                    .collect::<Vec<_>>(),
                !network.forwarding_enabled_by_sdk,
                !network.proxy_arp_enabled_by_sdk,
            ));
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

    fn cleanup_for_delete(&self, network: &PersistedNetwork) -> Result<(), SdkError> {
        let mut failures = Vec::new();
        if network.config.mode == NetworkMode::HostOnly {
            if let Err(error) = delete_link_if_present(&network.config.tap_name) {
                failures.push(error.to_string());
            }
            if network.forwarding_enabled_by_sdk
                && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
            {
                failures.push(error.to_string());
            }
        } else {
            let Some(IpAddr::V4(lan)) = network.config.lan_address else {
                failures.push("routed LAN network has no committed LAN address".to_owned());
                return Err(SdkError::Cleanup {
                    primary: "network cleanup failed".to_owned(),
                    failures,
                });
            };
            let uplink = network.config.uplink_name.clone().unwrap_or_default();
            let IpAddr::V4(private_guest) = network.config.guest_address else {
                failures.push("routed LAN network has no private guest address".to_owned());
                return Err(SdkError::Cleanup {
                    primary: "network cleanup failed".to_owned(),
                    failures,
                });
            };
            let private_network = private_guest_network(private_guest);
            failures.extend(cleanup_routed_for_delete(&RoutedCleanup {
                uplink: &uplink,
                tap: &network.config.tap_name,
                lan,
                private_network,
                private_guest,
                applied: &network
                    .resources
                    .iter()
                    .map(|item| item.resource)
                    .collect::<Vec<_>>(),
                forwarding_was_enabled: !network.forwarding_enabled_by_sdk,
                proxy_was_enabled: !network.proxy_arp_enabled_by_sdk,
            }));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SdkError::Cleanup {
                primary: "network cleanup failed".to_owned(),
                failures,
            })
        }
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
        let (private_network, gateway, guest) = select_private_subnet(&occupied, request)?;
        let tap_name = tap_name(&request.vm_name);
        let guest_gateway = if request.exact_network_values {
            request.gateway_override
        } else {
            Some(gateway)
        };
        let config = NetworkConfiguration {
            mode: NetworkMode::HostOnly,
            guest_address: IpAddr::V4(guest),
            prefix_length: request.prefix_length_override.unwrap_or(30),
            gateway: guest_gateway.map(IpAddr::V4),
            tap_name: tap_name.clone(),
            bridge_name: None,
            uplink_name: None,
            lan_address: None,
        };
        // The device field of `ip=` is resolved inside the guest, where the host TAP
        // name is unknown.
        let desired_boot_parameters = guest_boot_parameters(
            guest,
            guest_gateway,
            request.prefix_length_override.unwrap_or(30),
        );
        let mut applied = Vec::new();
        let forwarding_was_enabled = forwarding_enabled()?;
        let result = (|| {
            ensure_tap(&tap_name, &mut applied, NetworkMode::HostOnly)?;
            ensure_tap_address(&tap_name, gateway, &mut applied)?;
            ensure_forwarding(&mut applied)?;
            ensure_iptables_host_only(&tap_name, private_network, guest, &mut applied)?;
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
                    NetworkResource::ForwardRule,
                    &format!("{tap_name}:forward"),
                    &format!("{SDK_OWNERSHIP}:{tap_name}:forward"),
                ),
                resource(
                    NetworkResource::IptablesNat,
                    &tap_name,
                    &format!("{SDK_OWNERSHIP}:{tap_name}:nat"),
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
                uplink_cidr: None,
                proxy_arp_enabled_by_sdk: false,
                bridge_created_by_sdk: false,
                uplink_attached_by_sdk: false,
                forwarding_enabled_by_sdk: !forwarding_was_enabled,
                nat_table_created_by_sdk: false,
                nat_chain_created_by_sdk: false,
                host_route_created_by_sdk: false,
                proxy_arp_entry_created_by_sdk: false,
                host_address_specs: Vec::new(),
                default_route_specs: Vec::new(),
            };
            Ok(NetworkOutcome {
                persisted,
                applied: applied.clone(),
                skipped: Vec::new(),
            })
        })();
        match result {
            Ok(outcome) => Ok(outcome),
            Err(primary) => {
                let cleanup_failures = cleanup_host_only_attempt(
                    &tap_name,
                    private_network,
                    guest,
                    &applied,
                    forwarding_was_enabled,
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

    fn create_lan(
        &self,
        request: &NetworkRequest,
        used_lan_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError> {
        let uplink = self.detect_uplink()?;
        let mut occupied = Vec::new();
        occupied.extend(live_ipv4_addresses()?);
        let (_network_base, gateway, guest) = select_private_subnet(&occupied, request)?;
        let tap = tap_name(&request.vm_name);
        let offer = self.select_lan_offer(
            &uplink,
            request.lan_address_override,
            None,
            used_lan_addresses,
        )?;
        let guest_gateway = if request.exact_network_values {
            request.gateway_override
        } else {
            Some(gateway)
        };
        let config = NetworkConfiguration {
            mode: NetworkMode::Lan,
            guest_address: IpAddr::V4(guest),
            prefix_length: request.prefix_length_override.unwrap_or(30),
            gateway: guest_gateway.map(IpAddr::V4),
            tap_name: tap.clone(),
            bridge_name: None,
            uplink_name: Some(uplink.interface.clone()),
            lan_address: Some(IpAddr::V4(offer.address)),
        };
        let desired_boot_parameters = guest_boot_parameters(
            guest,
            guest_gateway,
            request.prefix_length_override.unwrap_or(30),
        );
        let mut applied = Vec::new();
        let forwarding_was_enabled = forwarding_enabled()?;
        let proxy_was_enabled = proxy_arp_enabled(&uplink.interface)?;
        let result = (|| {
            ensure_tap(&tap, &mut applied, NetworkMode::Lan)?;
            ensure_routed_host(&uplink, &tap, offer.address, gateway, &mut applied)?;
            ensure_forwarding(&mut applied)?;
            ensure_proxy_arp(&uplink.interface, &mut applied)?;
            ensure_iptables_routed(
                &uplink.interface,
                &tap,
                _network_base,
                guest,
                offer.address,
                &mut applied,
            )?;
            applied.push(NetworkResource::FirecrackerInterface);
            Ok::<(), SdkError>(())
        })();
        match result {
            Ok(()) => {
                let resources = vec![
                    resource(NetworkResource::Tap, &tap, &tap),
                    resource(
                        NetworkResource::TapAddress,
                        &format!("{tap}:{gateway}/30"),
                        &format!("{tap}:{gateway}/30"),
                    ),
                    resource(
                        NetworkResource::HostRoute,
                        &format!("{tap}:{}", offer.address),
                        &format!("{tap}:{}", offer.address),
                    ),
                    resource(
                        NetworkResource::ProxyArpEntry,
                        &format!("{}:{}", uplink.interface, offer.address),
                        &format!("{}:{}", uplink.interface, offer.address),
                    ),
                    resource(
                        NetworkResource::Forwarding,
                        "net.ipv4.ip_forward",
                        "net.ipv4.ip_forward=1",
                    ),
                    resource(
                        NetworkResource::ForwardRule,
                        &format!("{tap}:forward"),
                        &format!("{SDK_OWNERSHIP}:{tap}:forward"),
                    ),
                    resource(
                        NetworkResource::IptablesNat,
                        &tap,
                        &format!("{SDK_OWNERSHIP}:{tap}"),
                    ),
                    resource(
                        NetworkResource::FirecrackerInterface,
                        &format!("{tap}:{}", request.guest_mac),
                        &format!("tap={tap},mac={}", request.guest_mac),
                    ),
                ];
                Ok(NetworkOutcome {
                    persisted: PersistedNetwork {
                        config,
                        host_address: Some(IpAddr::V4(gateway)),
                        guest_mac: request.guest_mac.clone(),
                        dhcp_lease_reference: None,
                        desired_boot_parameters,
                        resources,
                        uplink_cidr: Some(offer.uplink_cidr.clone()),
                        proxy_arp_enabled_by_sdk: !proxy_was_enabled,
                        bridge_created_by_sdk: false,
                        uplink_attached_by_sdk: false,
                        forwarding_enabled_by_sdk: !forwarding_was_enabled,
                        nat_table_created_by_sdk: false,
                        nat_chain_created_by_sdk: false,
                        host_route_created_by_sdk: true,
                        proxy_arp_entry_created_by_sdk: true,
                        host_address_specs: Vec::new(),
                        default_route_specs: Vec::new(),
                    },
                    applied: applied.clone(),
                    skipped: Vec::new(),
                })
            }
            Err(primary) => {
                let cleanup_failures = cleanup_routed_attempt(
                    &uplink.interface,
                    &tap,
                    offer.address,
                    _network_base,
                    guest,
                    &applied,
                    forwarding_was_enabled,
                    proxy_was_enabled,
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
        let IpAddr::V4(guest) = network.config.guest_address else {
            return Err(SdkError::Network {
                mode: NetworkMode::HostOnly.to_string(),
                operation: "validate persisted guest address".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "host-only network has no IPv4 guest address".to_owned(),
            });
        };
        let private_network = private_guest_network(guest);
        let tap_existed = link_exists(&network.config.tap_name)?;
        let forwarding_was_enabled = forwarding_enabled()?;
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
            reconcile_forward_rules(&network.config.tap_name, guest, &mut applied, &mut skipped)?;
            reconcile_iptables_nat(
                &network.config.tap_name,
                private_network,
                &mut applied,
                &mut skipped,
            )?;
            if applied.contains(&NetworkResource::Forwarding) {
                network.forwarding_enabled_by_sdk = true;
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
                    private_network,
                    guest,
                    tap_existed,
                    &applied,
                    forwarding_was_enabled,
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
        let Some(IpAddr::V4(lan)) = network.config.lan_address else {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "load persisted LAN address".to_owned(),
                resource: vm_name.to_owned(),
                reason: "LAN network has no committed LAN address".to_owned(),
            });
        };
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
        let Some(IpAddr::V4(gateway)) = network.host_address else {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "validate persisted host address".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "routed LAN network has no persisted gateway".to_owned(),
            });
        };
        let IpAddr::V4(private_guest) = network.config.guest_address else {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "validate persisted guest address".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "routed LAN network has no IPv4 guest address".to_owned(),
            });
        };
        if network.config.prefix_length != 30 {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "validate persisted host prefix".to_owned(),
                resource: network.config.tap_name.clone(),
                reason: "routed LAN network must use a /30 prefix".to_owned(),
            });
        }
        let tap_existed = link_exists(&network.config.tap_name)?;
        let forwarding_was_enabled = forwarding_enabled()?;
        let proxy_was_enabled = proxy_arp_enabled(&uplink)?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let private_network = private_guest_network(private_guest);
        let result = (|| {
            reconcile_tap(&network.config.tap_name, &mut applied, &mut skipped)?;
            reconcile_tap_address(
                &network.config.tap_name,
                gateway,
                &mut applied,
                &mut skipped,
            )?;
            reconcile_host_route(&network.config.tap_name, lan, &mut applied, &mut skipped)?;
            reconcile_proxy_entry(&uplink, lan, &mut applied, &mut skipped)?;
            reconcile_forwarding(&mut applied, &mut skipped)?;
            reconcile_proxy_arp(&uplink, &mut applied, &mut skipped)?;
            reconcile_iptables_routed(
                &uplink,
                &network.config.tap_name,
                private_network,
                private_guest,
                lan,
                &mut applied,
                &mut skipped,
            )?;
            if applied.contains(&NetworkResource::Forwarding) {
                network.forwarding_enabled_by_sdk = true;
            }
            if applied.contains(&NetworkResource::ProxyArpEntry) {
                network.proxy_arp_entry_created_by_sdk = true;
            }
            if applied.contains(&NetworkResource::HostRoute) {
                network.host_route_created_by_sdk = true;
            }
            skipped.push(NetworkResource::FirecrackerInterface);
            Ok::<(), SdkError>(())
        })();
        match result {
            Ok(()) => Ok((applied, skipped)),
            Err(primary) => {
                let cleanup_failures = cleanup_routed_attempt(
                    &uplink,
                    &network.config.tap_name,
                    lan,
                    private_network,
                    private_guest,
                    &applied,
                    forwarding_was_enabled,
                    proxy_was_enabled,
                );
                let _ = tap_existed;
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

fn reconcile_host_route(
    tap: &str,
    lan: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let output = command_output(
        "ip",
        &["-4", "route", "show", &format!("{lan}/32"), "dev", tap],
    )?;
    if !output.status.success() {
        return Err(host_command_error(
            "ip",
            &["-4", "route", "show", &format!("{lan}/32"), "dev", tap],
            &output,
        ));
    }
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains(&format!("{lan}/32")) && line.contains(tap))
    {
        skipped.push(NetworkResource::HostRoute);
    } else {
        run_ip(&["route", "replace", &format!("{lan}/32"), "dev", tap])?;
        applied.push(NetworkResource::HostRoute);
    }
    Ok(())
}

fn reconcile_proxy_entry(
    uplink: &str,
    lan: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let output = command_output("ip", &["neigh", "show", "proxy", "dev", uplink])?;
    if !output.status.success() {
        return Err(host_command_error(
            "ip",
            &["neigh", "show", "proxy", "dev", uplink],
            &output,
        ));
    }
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains(&lan.to_string()))
    {
        skipped.push(NetworkResource::ProxyArpEntry);
    } else {
        run_ip(&["neigh", "replace", "proxy", &lan.to_string(), "dev", uplink])?;
        applied.push(NetworkResource::ProxyArpEntry);
    }
    Ok(())
}

fn reconcile_proxy_arp(
    uplink: &str,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    if proxy_arp_enabled(uplink)? {
        skipped.push(NetworkResource::Forwarding);
    } else {
        ensure_proxy_arp(uplink, applied)?;
    }
    Ok(())
}

fn reconcile_iptables_routed(
    uplink: &str,
    tap: &str,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    lan: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let mut missing = 0;
    for spec in routed_forward_specs(uplink, tap, private_guest, lan) {
        let arguments = spec.iter().map(String::as_str).collect::<Vec<_>>();
        if iptables_rule_exists(&arguments)? {
            skipped.push(NetworkResource::ForwardRule);
        } else {
            run_iptables_spec(&spec)?;
            applied.push(NetworkResource::ForwardRule);
            missing += 1;
        }
    }
    let nat = routed_nat_spec(uplink, tap, private_network);
    let nat_rule: Vec<&str> = nat.iter().map(String::as_str).collect();
    if iptables_nat_rule_exists(&nat_rule)? {
        skipped.push(NetworkResource::IptablesNat);
    } else {
        iptables_nat_insert_rule(&nat_rule)?;
        applied.push(NetworkResource::IptablesNat);
        missing += 1;
    }
    let _ = missing;
    Ok(())
}

fn select_private_subnet(
    used_addresses: &[(String, IpAddr, String)],
    request: &NetworkRequest,
) -> Result<(Ipv4Addr, Ipv4Addr, Ipv4Addr), SdkError> {
    if !request.exact_network_values {
        return allocate_subnet(used_addresses).ok_or_else(|| SdkError::Network {
            mode: request.mode.to_string(),
            operation: "allocate private /30".to_owned(),
            resource: "172.30.0.0/16".to_owned(),
            reason: "the private address pool is exhausted or overlaps persisted state".to_owned(),
        });
    }
    let (Some(guest), Some(30)) = (
        request.guest_address_override,
        request.prefix_length_override,
    ) else {
        return Err(SdkError::InvalidRequest {
            field: "network_override".to_owned(),
            reason: "exact IPv4 restore requires a guest address and /30 prefix".to_owned(),
        });
    };
    let guest_number = u32::from(guest);
    let network_base = guest_number & !3;
    let expected_gateway = Ipv4Addr::from(network_base.saturating_add(1));
    let expected_guest = Ipv4Addr::from(network_base.saturating_add(2));
    let gateway = request.gateway_override.unwrap_or(expected_gateway);
    let conflicts = private_subnet_conflicts(network_base, used_addresses);
    if guest != expected_guest || gateway != expected_gateway || conflicts {
        return Err(SdkError::SnapshotNetworkConflict {
            field: "guest/gateway IPv4 subnet".to_owned(),
            value: format!(
                "{guest}/30 via {}",
                request
                    .gateway_override
                    .map_or_else(|| "(none)".to_owned(), |value| value.to_string())
            ),
        });
    }
    Ok((Ipv4Addr::from(network_base), expected_gateway, guest))
}

fn guest_boot_parameters(guest: Ipv4Addr, gateway: Option<Ipv4Addr>, prefix_length: u8) -> String {
    let netmask = Ipv4Addr::from(
        u32::MAX
            .checked_shl(u32::from(32 - prefix_length))
            .unwrap_or(0),
    );
    format!(
        "ip={guest}::{}:{netmask}::{GUEST_INTERFACE}:off",
        gateway.map_or_else(String::new, |value| value.to_string())
    )
}

fn allocate_subnet(
    used_addresses: &[(String, IpAddr, String)],
) -> Option<(Ipv4Addr, Ipv4Addr, Ipv4Addr)> {
    let mut base = PRIVATE_POOL_START;
    while base <= PRIVATE_POOL_END {
        let gateway = Ipv4Addr::from(base + 1);
        let guest = Ipv4Addr::from(base + 2);
        let conflict = private_subnet_conflicts(base, used_addresses);
        if !conflict {
            return Some((Ipv4Addr::from(base), gateway, guest));
        }
        base = base.saturating_add(4);
    }
    None
}

fn private_subnet_conflicts(
    network_base: u32,
    used_addresses: &[(String, IpAddr, String)],
) -> bool {
    used_addresses.iter().any(|(_, address, prefix)| {
        let IpAddr::V4(address) = address else {
            return false;
        };
        let prefix = prefix.parse::<u8>().unwrap_or(32).min(32);
        let address = u32::from(*address);
        if prefix <= 30 {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            (address & mask) == (network_base & mask)
        } else {
            (address & !3) == network_base
        }
    })
}

fn private_guest_network(guest: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(guest) & !3)
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
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    applied: &[NetworkResource],
    forwarding_was_enabled: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if applied.contains(&NetworkResource::ForwardRule) {
        for spec in host_only_forward_specs(tap, private_guest) {
            if let Err(error) = delete_iptables_spec(&spec) {
                failures.push(error.to_string());
            }
        }
    }
    if applied.contains(&NetworkResource::IptablesNat)
        && let Err(error) = delete_iptables_spec(&host_only_nat_spec(tap, private_network))
    {
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
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    tap_existed: bool,
    applied: &[NetworkResource],
    forwarding_was_enabled: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if applied.contains(&NetworkResource::ForwardRule) {
        for spec in host_only_forward_specs(tap, private_guest) {
            if let Err(error) = delete_iptables_spec(&spec) {
                failures.push(error.to_string());
            }
        }
    }
    if applied.contains(&NetworkResource::IptablesNat)
        && let Err(error) = delete_iptables_spec(&host_only_nat_spec(tap, private_network))
    {
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

fn host_only_nat_spec(tap: &str, private_network: Ipv4Addr) -> Vec<String> {
    vec![
        "POSTROUTING".to_owned(),
        "-s".to_owned(),
        format!("{private_network}/30"),
        "!".to_owned(),
        "-o".to_owned(),
        tap.to_owned(),
        "-j".to_owned(),
        "MASQUERADE".to_owned(),
        "-m".to_owned(),
        "comment".to_owned(),
        "--comment".to_owned(),
        iptables_comment(tap, "nat"),
    ]
}

fn host_only_forward_specs(tap: &str, private_guest: Ipv4Addr) -> Vec<Vec<String>> {
    vec![
        vec![
            "FORWARD".to_owned(),
            "-i".to_owned(),
            tap.to_owned(),
            "-j".to_owned(),
            "ACCEPT".to_owned(),
            "-m".to_owned(),
            "comment".to_owned(),
            "--comment".to_owned(),
            iptables_comment(tap, "forward-out"),
        ],
        vec![
            "FORWARD".to_owned(),
            "-o".to_owned(),
            tap.to_owned(),
            "-d".to_owned(),
            format!("{private_guest}/32"),
            "-m".to_owned(),
            "conntrack".to_owned(),
            "--ctstate".to_owned(),
            "RELATED,ESTABLISHED".to_owned(),
            "-j".to_owned(),
            "ACCEPT".to_owned(),
            "-m".to_owned(),
            "comment".to_owned(),
            "--comment".to_owned(),
            iptables_comment(tap, "forward-in"),
        ],
    ]
}

fn ensure_iptables_host_only(
    tap: &str,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    for spec in host_only_forward_specs(tap, private_guest) {
        run_iptables_spec(&spec)?;
        applied.push(NetworkResource::ForwardRule);
    }
    run_iptables_spec(&host_only_nat_spec(tap, private_network))?;
    applied.push(NetworkResource::IptablesNat);
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

fn detect_uplink_identity() -> Result<UplinkIdentity, SdkError> {
    let uplink = detect_default_uplink()?;
    let output = command_output("ip", &["-o", "-4", "addr", "show", "dev", &uplink])?;
    if !output.status.success() {
        return Err(host_command_error(
            "ip",
            &["-o", "-4", "addr", "show", "dev", &uplink],
            &output,
        ));
    }
    let mut cidr = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(index) = fields.iter().position(|field| *field == "inet") else {
            continue;
        };
        let Some(spec) = fields.get(index + 1) else {
            continue;
        };
        if spec.contains("/scope link") || *spec == "127.0.0.1/8" {
            continue;
        }
        if let Some((address, prefix)) = spec.split_once('/')
            && let (Ok(address), Ok(prefix)) = (address.parse::<Ipv4Addr>(), prefix.parse::<u8>())
            && !address.is_loopback()
            && !address.is_link_local()
        {
            cidr = Some((address, prefix, format!("{address}/{prefix}")));
            break;
        }
    }
    let Some((address, prefix_length, cidr)) = cidr else {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "detect uplink address".to_owned(),
            resource: uplink.clone(),
            reason: "the uplink has no global IPv4 address".to_owned(),
        });
    };
    let gateway = default_gateway_for_uplink(&uplink)?;
    Ok(UplinkIdentity {
        interface: uplink,
        address,
        prefix_length,
        gateway,
        cidr,
    })
}

fn default_gateway_for_uplink(uplink: &str) -> Result<Option<Ipv4Addr>, SdkError> {
    let output = command_output("ip", &["-4", "route", "show", "default", "dev", uplink])?;
    if !output.status.success() {
        return Err(host_command_error(
            "ip",
            &["-4", "route", "show", "default", "dev", uplink],
            &output,
        ));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if let Some(index) = fields.iter().position(|field| *field == "via")
            && let Some(gateway) = fields.get(index + 1)
            && let Ok(gateway) = gateway.parse::<Ipv4Addr>()
        {
            return Ok(Some(gateway));
        }
    }
    Ok(None)
}

fn select_lan_offer(
    uplink: &UplinkIdentity,
    lan_override: Option<Ipv4Addr>,
    previous: Option<Ipv4Addr>,
    used_addresses: &[(String, IpAddr, String)],
) -> Result<LanAddressOffer, SdkError> {
    use crate::ports::network::LanOfferSource;

    let (network, prefix) = parse_uplink_cidr(&uplink.cidr)?;
    let host = uplink.address;
    let gateway = uplink.gateway;
    let check = |candidate: Ipv4Addr| -> Result<(), SdkError> {
        validate_lan_candidate(candidate, network, prefix, host, gateway)?;
        if lan_address_in_use(candidate, used_addresses) {
            return Err(SdkError::Network {
                mode: NetworkMode::Lan.to_string(),
                operation: "allocate LAN address".to_owned(),
                resource: candidate.to_string(),
                reason: "the LAN address is already used by another VM".to_owned(),
            });
        }
        probe_lan_candidate(&uplink.interface, candidate)?;
        Ok(())
    };
    if let Some(candidate) = lan_override {
        check(candidate)?;
        return Ok(LanAddressOffer {
            address: candidate,
            source: LanOfferSource::ExplicitOverride,
            uplink_cidr: uplink.cidr.clone(),
        });
    }
    if let Some(candidate) = previous
        && lan_candidate_in_subnet(candidate, network, prefix)
        && !lan_address_in_use(candidate, used_addresses)
        && probe_lan_candidate(&uplink.interface, candidate).is_ok()
    {
        return Ok(LanAddressOffer {
            address: candidate,
            source: LanOfferSource::PreviousAssignment,
            uplink_cidr: uplink.cidr.clone(),
        });
    }
    for candidate in automatic_lan_candidates(network, prefix, host, gateway) {
        if lan_address_in_use(candidate, used_addresses) {
            continue;
        }
        if probe_lan_candidate(&uplink.interface, candidate).is_ok() {
            return Ok(LanAddressOffer {
                address: candidate,
                source: LanOfferSource::AutomaticSearch,
                uplink_cidr: uplink.cidr.clone(),
            });
        }
    }
    Err(SdkError::Network {
        mode: NetworkMode::Lan.to_string(),
        operation: "allocate LAN address".to_owned(),
        resource: uplink.cidr.clone(),
        reason: "no apparently free LAN address exists in the uplink subnet".to_owned(),
    })
}

fn parse_uplink_cidr(cidr: &str) -> Result<(u32, u8), SdkError> {
    let Some((address, prefix)) = cidr.split_once('/') else {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "parse uplink subnet".to_owned(),
            resource: cidr.to_owned(),
            reason: "the uplink CIDR is not valid".to_owned(),
        });
    };
    let address = address.parse::<Ipv4Addr>().map_err(|_| SdkError::Network {
        mode: NetworkMode::Lan.to_string(),
        operation: "parse uplink subnet".to_owned(),
        resource: cidr.to_owned(),
        reason: "the uplink address is not valid IPv4".to_owned(),
    })?;
    let prefix = prefix.parse::<u8>().map_err(|_| SdkError::Network {
        mode: NetworkMode::Lan.to_string(),
        operation: "parse uplink subnet".to_owned(),
        resource: cidr.to_owned(),
        reason: "the uplink prefix is not valid".to_owned(),
    })?;
    if prefix > 30 {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "parse uplink subnet".to_owned(),
            resource: cidr.to_owned(),
            reason: "the uplink subnet leaves no usable host address".to_owned(),
        });
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Ok((u32::from(address) & mask, prefix))
}

fn lan_candidate_in_subnet(candidate: Ipv4Addr, network: u32, prefix: u8) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    (u32::from(candidate) & mask) == network
}

fn validate_lan_candidate(
    candidate: Ipv4Addr,
    network: u32,
    prefix: u8,
    host: Ipv4Addr,
    gateway: Option<Ipv4Addr>,
) -> Result<(), SdkError> {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let value = u32::from(candidate);
    let broadcast = network | !mask;
    if (value & mask) != network
        || value == network
        || value == broadcast
        || candidate == host
        || Some(candidate) == gateway
    {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "validate LAN address".to_owned(),
            resource: candidate.to_string(),
            reason: "the LAN address is outside the uplink subnet or reserved".to_owned(),
        });
    }
    Ok(())
}

fn lan_address_in_use(candidate: Ipv4Addr, used_addresses: &[(String, IpAddr, String)]) -> bool {
    used_addresses.iter().any(|(_, address, _)| match address {
        IpAddr::V4(value) => *value == candidate,
        _ => false,
    })
}

fn automatic_lan_candidates(
    network: u32,
    prefix: u8,
    host: Ipv4Addr,
    gateway: Option<Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let broadcast = network | !mask;
    let host_value = u32::from(host);
    let gateway_value = gateway.map(u32::from);
    let mut candidates = Vec::new();
    let mut value = broadcast.saturating_sub(1);
    let low = network.saturating_add(2);
    let stop = broadcast.saturating_sub(200).max(low);
    while value >= low && value >= stop {
        if value != host_value && Some(value) != gateway_value {
            candidates.push(Ipv4Addr::from(value));
        }
        if value == low {
            break;
        }
        value -= 1;
    }
    candidates
}

fn probe_lan_candidate(interface: &str, candidate: Ipv4Addr) -> Result<(), SdkError> {
    let address = candidate.to_string();
    let arping = command_output(
        "arping",
        &["-D", "-I", interface, "-c", "2", "-w", "2", &address],
    );
    match arping {
        Ok(output) if output.status.success() => return Ok(()),
        Ok(output) => {
            let stderr = command_stderr(&output);
            if !is_privilege_denied(&stderr) && !stderr.contains("not found") {
                return Err(SdkError::Network {
                    mode: NetworkMode::Lan.to_string(),
                    operation: "probe LAN address".to_owned(),
                    resource: address,
                    reason: "the LAN address appears to be in use".to_owned(),
                });
            }
        }
        Err(_) => {}
    }
    let ping = command_output("ping", &["-c", "1", "-W", "1", &address])?;
    if ping.status.success() {
        return Err(SdkError::Network {
            mode: NetworkMode::Lan.to_string(),
            operation: "probe LAN address".to_owned(),
            resource: address,
            reason: "the LAN address appears to be in use".to_owned(),
        });
    }
    Ok(())
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

fn proxy_arp_enabled(uplink: &str) -> Result<bool, SdkError> {
    let key = format!("net.ipv4.conf.{uplink}.proxy_arp");
    let output = command_output("sysctl", &["-n", &key])?;
    if !output.status.success() {
        return Err(host_command_error("sysctl", &["-n", &key], &output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn ensure_proxy_arp(uplink: &str, applied: &mut Vec<NetworkResource>) -> Result<(), SdkError> {
    if !proxy_arp_enabled(uplink)? {
        let key = format!("net.ipv4.conf.{uplink}.proxy_arp=1");
        run_command("sysctl", &["-w", &key])?;
        applied.push(NetworkResource::Forwarding);
    }
    Ok(())
}

fn ensure_routed_host(
    uplink: &UplinkIdentity,
    tap: &str,
    lan: Ipv4Addr,
    gateway: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    run_command("ip", &["-4", "addr", "flush", "dev", tap])?;
    run_ip(&["addr", "add", &format!("{gateway}/30"), "dev", tap])?;
    applied.push(NetworkResource::TapAddress);
    run_ip(&["route", "replace", &format!("{lan}/32"), "dev", tap])?;
    applied.push(NetworkResource::HostRoute);
    run_ip(&[
        "neigh",
        "replace",
        "proxy",
        &lan.to_string(),
        "dev",
        &uplink.interface,
    ])?;
    applied.push(NetworkResource::ProxyArpEntry);
    Ok(())
}

fn iptables_comment(tap: &str, scope: &str) -> String {
    format!("{SDK_OWNERSHIP}:{tap}:{scope}")
}

fn iptables_rule_exists(spec: &[&str]) -> Result<bool, SdkError> {
    let mut arguments = vec!["-C"];
    arguments.extend(spec.iter().copied());
    let output = command_output("iptables", &arguments)?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = command_stderr(&output);
    if stderr.contains("No chain/target/match by that name")
        || stderr.contains("Bad rule")
        || stderr.contains("No such")
    {
        return Ok(false);
    }
    Err(host_command_error("iptables", &arguments, &output))
}

fn iptables_add_rule(spec: &[&str]) -> Result<(), SdkError> {
    if iptables_rule_exists(spec)? {
        return Ok(());
    }
    // Insert at the top so SDK rules are evaluated before terminal UFW rules.
    // Existence is still checked with `-C`, which is position-independent.
    let mut arguments = vec!["-I", spec[0], "1"];
    arguments.extend(spec.iter().skip(1).copied());
    run_command("iptables", &arguments)?;
    Ok(())
}

fn iptables_nat_rule_exists(chain_and_rule: &[&str]) -> Result<bool, SdkError> {
    let arguments = nat_check_arguments(chain_and_rule);
    let output = command_output("iptables", &arguments)?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = command_stderr(&output);
    if stderr.contains("No chain/target/match by that name")
        || stderr.contains("Bad rule")
        || stderr.contains("No such")
    {
        return Ok(false);
    }
    Err(host_command_error("iptables", &arguments, &output))
}

fn iptables_nat_insert_rule(chain_and_rule: &[&str]) -> Result<(), SdkError> {
    if iptables_nat_rule_exists(chain_and_rule)? {
        return Ok(());
    }
    // Insert at the top so SDK rules are evaluated before terminal UFW rules.
    // Existence is still checked with `-C`, which is position-independent.
    let mut arguments = vec!["-t", "nat", "-I", chain_and_rule[0], "1"];
    arguments.extend(chain_and_rule.iter().skip(1).copied());
    run_command("iptables", &arguments)?;
    Ok(())
}

/// Builds `iptables -t nat -C <chain> <rule>` for a stored NAT spec.
fn nat_check_arguments<'a>(chain_and_rule: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["-t", "nat", "-C"];
    arguments.extend(chain_and_rule.iter().copied());
    arguments
}

/// Builds `iptables -t nat -D <chain> <rule>` for a stored NAT spec.
fn nat_delete_arguments<'a>(chain_and_rule: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["-t", "nat", "-D"];
    arguments.extend(chain_and_rule.iter().copied());
    arguments
}

fn iptables_delete_rule(spec: &[&str]) -> Result<(), SdkError> {
    if !iptables_rule_exists(spec)? {
        return Ok(());
    }
    let mut arguments = vec!["-D"];
    arguments.extend(spec.iter().copied());
    run_command("iptables", &arguments)?;
    Ok(())
}

fn routed_forward_specs(
    uplink: &str,
    tap: &str,
    private_guest: Ipv4Addr,
    lan: Ipv4Addr,
) -> Vec<Vec<String>> {
    vec![
        vec![
            "FORWARD".to_owned(),
            "-i".to_owned(),
            tap.to_owned(),
            "-o".to_owned(),
            uplink.to_owned(),
            "-j".to_owned(),
            "ACCEPT".to_owned(),
            "-m".to_owned(),
            "comment".to_owned(),
            "--comment".to_owned(),
            iptables_comment(tap, "forward-out"),
        ],
        vec![
            "FORWARD".to_owned(),
            "-i".to_owned(),
            uplink.to_owned(),
            "-o".to_owned(),
            tap.to_owned(),
            "-d".to_owned(),
            format!("{private_guest}/32"),
            "-m".to_owned(),
            "conntrack".to_owned(),
            "--ctstate".to_owned(),
            "RELATED,ESTABLISHED".to_owned(),
            "-j".to_owned(),
            "ACCEPT".to_owned(),
            "-m".to_owned(),
            "comment".to_owned(),
            "--comment".to_owned(),
            iptables_comment(tap, "forward-in-private"),
        ],
        vec![
            "FORWARD".to_owned(),
            "-i".to_owned(),
            uplink.to_owned(),
            "-o".to_owned(),
            tap.to_owned(),
            "-d".to_owned(),
            format!("{lan}/32"),
            "-j".to_owned(),
            "ACCEPT".to_owned(),
            "-m".to_owned(),
            "comment".to_owned(),
            "--comment".to_owned(),
            iptables_comment(tap, "forward-in-lan"),
        ],
    ]
}

fn routed_nat_spec(uplink: &str, tap: &str, private_network: Ipv4Addr) -> Vec<String> {
    vec![
        "POSTROUTING".to_owned(),
        "-s".to_owned(),
        format!("{private_network}/30"),
        "-o".to_owned(),
        uplink.to_owned(),
        "-j".to_owned(),
        "MASQUERADE".to_owned(),
        "-m".to_owned(),
        "comment".to_owned(),
        "--comment".to_owned(),
        iptables_comment(tap, "nat"),
    ]
}

fn run_iptables_spec(spec: &[String]) -> Result<(), SdkError> {
    let arguments = spec.iter().map(String::as_str).collect::<Vec<_>>();
    if arguments.first() == Some(&"POSTROUTING") {
        let mut with_table = vec!["-t", "nat", "-I", "POSTROUTING", "1"];
        with_table.extend(arguments.iter().skip(1).copied());
        run_command("iptables", &with_table)?;
    } else {
        iptables_add_rule(&arguments)?;
    }
    Ok(())
}

fn delete_iptables_spec(spec: &[String]) -> Result<(), SdkError> {
    let arguments: Vec<&str> = spec.iter().map(String::as_str).collect();
    if arguments.first() == Some(&"POSTROUTING") {
        if !iptables_nat_rule_exists(&arguments)? {
            return Ok(());
        }
        run_command("iptables", &nat_delete_arguments(&arguments))?;
    } else {
        iptables_delete_rule(&arguments)?;
    }
    Ok(())
}

fn ensure_iptables_routed(
    uplink: &str,
    tap: &str,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    lan: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    for spec in routed_forward_specs(uplink, tap, private_guest, lan) {
        run_iptables_spec(&spec)?;
        applied.push(NetworkResource::ForwardRule);
    }
    run_iptables_spec(&routed_nat_spec(uplink, tap, private_network))?;
    applied.push(NetworkResource::IptablesNat);
    Ok(())
}

struct RoutedCleanup<'a> {
    uplink: &'a str,
    tap: &'a str,
    lan: Ipv4Addr,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    applied: &'a [NetworkResource],
    forwarding_was_enabled: bool,
    proxy_was_enabled: bool,
}

#[allow(clippy::too_many_arguments)]
fn cleanup_routed_attempt(
    uplink: &str,
    tap: &str,
    lan: Ipv4Addr,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
    applied: &[NetworkResource],
    forwarding_was_enabled: bool,
    proxy_was_enabled: bool,
) -> Vec<String> {
    cleanup_routed(&RoutedCleanup {
        uplink,
        tap,
        lan,
        private_network,
        private_guest,
        applied,
        forwarding_was_enabled,
        proxy_was_enabled,
    })
}

fn cleanup_routed(state: &RoutedCleanup<'_>) -> Vec<String> {
    let mut failures = Vec::new();
    let rules_owned = state.applied.contains(&NetworkResource::IptablesNat)
        || state.applied.contains(&NetworkResource::ForwardRule);
    if rules_owned
        && let Err(error) = cleanup_routed_rules(
            state.uplink,
            state.tap,
            state.lan,
            state.private_network,
            state.private_guest,
        )
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::ProxyArpEntry)
        && let Err(error) = run_ip(&[
            "neigh",
            "del",
            "proxy",
            &state.lan.to_string(),
            "dev",
            state.uplink,
        ])
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::HostRoute)
        && let Err(error) = run_ip(&[
            "route",
            "del",
            &format!("{}/32", state.lan),
            "dev",
            state.tap,
        ])
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::Tap)
        && let Err(error) = delete_link_if_present(state.tap)
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::Forwarding)
        && !state.forwarding_was_enabled
        && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
    {
        failures.push(error.to_string());
    }
    if !state.proxy_was_enabled
        && state.applied.contains(&NetworkResource::Forwarding)
        && let Err(error) = run_command(
            "sysctl",
            &[&format!("net.ipv4.conf.{}.proxy_arp=0", state.uplink)],
        )
    {
        failures.push(error.to_string());
    }
    failures
}

fn cleanup_routed_rules(
    uplink: &str,
    tap: &str,
    lan: Ipv4Addr,
    private_network: Ipv4Addr,
    private_guest: Ipv4Addr,
) -> Result<(), SdkError> {
    for spec in routed_forward_specs(uplink, tap, private_guest, lan) {
        delete_iptables_spec(&spec)?;
    }
    delete_iptables_spec(&routed_nat_spec(uplink, tap, private_network))?;
    Ok(())
}

/// Delete-path routed cleanup: like [`cleanup_routed`], but an already-absent
/// host route or proxy-neighbour entry counts as converged instead of failing.
/// TAP removal, iptables-rule removal, and sysctl restorations reuse the same
/// absent-tolerant primitives as the rollback path.
fn cleanup_routed_for_delete(state: &RoutedCleanup<'_>) -> Vec<String> {
    let mut failures = Vec::new();
    let rules_owned = state.applied.contains(&NetworkResource::IptablesNat)
        || state.applied.contains(&NetworkResource::ForwardRule);
    if rules_owned
        && let Err(error) = cleanup_routed_rules(
            state.uplink,
            state.tap,
            state.lan,
            state.private_network,
            state.private_guest,
        )
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::ProxyArpEntry)
        && let Err(error) = delete_proxy_entry_if_present(state.uplink, state.lan)
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::HostRoute)
        && let Err(error) = delete_host_route_if_present(state.tap, state.lan)
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::Tap)
        && let Err(error) = delete_link_if_present(state.tap)
    {
        failures.push(error.to_string());
    }
    if state.applied.contains(&NetworkResource::Forwarding)
        && !state.forwarding_was_enabled
        && let Err(error) = run_command("sysctl", &["-w", "net.ipv4.ip_forward=0"])
    {
        failures.push(error.to_string());
    }
    if !state.proxy_was_enabled
        && state.applied.contains(&NetworkResource::Forwarding)
        && let Err(error) = run_command(
            "sysctl",
            &[&format!("net.ipv4.conf.{}.proxy_arp=0", state.uplink)],
        )
    {
        failures.push(error.to_string());
    }
    failures
}

/// Removes one `/32` host route when present; an already-absent route is converged.
fn delete_host_route_if_present(tap: &str, lan: Ipv4Addr) -> Result<(), SdkError> {
    let output = command_output(
        "ip",
        &["-4", "route", "show", &format!("{lan}/32"), "dev", tap],
    )?;
    if !output.status.success() {
        if is_device_missing(&output) {
            return Ok(());
        }
        return Err(host_command_error(
            "ip",
            &["-4", "route", "show", &format!("{lan}/32"), "dev", tap],
            &output,
        ));
    }
    if !String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains(&format!("{lan}/32")) && line.contains(tap))
    {
        return Ok(());
    }
    match run_ip(&["route", "del", &format!("{lan}/32"), "dev", tap]) {
        Ok(()) => Ok(()),
        Err(primary) => {
            if primary_device_is_missing(&primary) {
                return Ok(());
            }
            let output = command_output(
                "ip",
                &["-4", "route", "show", &format!("{lan}/32"), "dev", tap],
            )?;
            if is_device_missing(&output) {
                return Ok(());
            }
            if output.status.success()
                && !String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.contains(&format!("{lan}/32")) && line.contains(tap))
            {
                Ok(())
            } else {
                Err(primary)
            }
        }
    }
}

/// Removes one proxy-neighbour entry when present; an already-absent entry is converged.
fn delete_proxy_entry_if_present(uplink: &str, lan: Ipv4Addr) -> Result<(), SdkError> {
    let output = command_output("ip", &["neigh", "show", "proxy", "dev", uplink])?;
    if !output.status.success() {
        if is_device_missing(&output) {
            return Ok(());
        }
        return Err(host_command_error(
            "ip",
            &["neigh", "show", "proxy", "dev", uplink],
            &output,
        ));
    }
    if !String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.contains(&lan.to_string()))
    {
        return Ok(());
    }
    match run_ip(&["neigh", "del", "proxy", &lan.to_string(), "dev", uplink]) {
        Ok(()) => Ok(()),
        Err(primary) => {
            if primary_device_is_missing(&primary) {
                return Ok(());
            }
            let output = command_output("ip", &["neigh", "show", "proxy", "dev", uplink])?;
            if is_device_missing(&output) {
                return Ok(());
            }
            if output.status.success()
                && !String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.contains(&lan.to_string()))
            {
                Ok(())
            } else {
                Err(primary)
            }
        }
    }
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

fn reconcile_forward_rules(
    tap: &str,
    private_guest: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    for spec in host_only_forward_specs(tap, private_guest) {
        let arguments = spec.iter().map(String::as_str).collect::<Vec<_>>();
        if iptables_rule_exists(&arguments)? {
            skipped.push(NetworkResource::ForwardRule);
        } else {
            run_iptables_spec(&spec)?;
            applied.push(NetworkResource::ForwardRule);
        }
    }
    Ok(())
}

fn reconcile_iptables_nat(
    tap: &str,
    private_network: Ipv4Addr,
    applied: &mut Vec<NetworkResource>,
    skipped: &mut Vec<NetworkResource>,
) -> Result<(), SdkError> {
    let spec = host_only_nat_spec(tap, private_network);
    let nat_rule: Vec<&str> = spec.iter().map(String::as_str).collect();
    if iptables_nat_rule_exists(&nat_rule)? {
        skipped.push(NetworkResource::IptablesNat);
        Ok(())
    } else {
        iptables_nat_insert_rule(&nat_rule)?;
        applied.push(NetworkResource::IptablesNat);
        Ok(())
    }
}

fn link_exists(name: &str) -> Result<bool, SdkError> {
    Ok(link_output(name)?.is_some())
}

fn link_output(name: &str) -> Result<Option<String>, SdkError> {
    let output = Command::new("ip")
        .args(["-d", "link", "show", "dev", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SdkError::HostCommand {
            program: "ip".to_owned(),
            reason: error.to_string(),
        })?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else if is_link_missing(&output) {
        Ok(None)
    } else {
        Err(host_command_error(
            "ip",
            &["-d", "link", "show", "dev", name],
            &output,
        ))
    }
}

/// Reports whether a failed `ip` deletion already implies the device is gone.
fn primary_device_is_missing(error: &SdkError) -> bool {
    let SdkError::HostCommand { reason, .. } = error else {
        return false;
    };
    reason.contains("Cannot find device") && !is_privilege_denied(reason)
}

fn is_link_missing(output: &std::process::Output) -> bool {
    let stderr = command_stderr(output);
    stderr.contains("does not exist") && !is_privilege_denied(&stderr)
}

/// Reports whether an `ip` failure means the queried device is gone.
///
/// A route or proxy-neighbour entry bound to a missing device cannot exist,
/// so the delete path treats this as converged rather than failed. Anything
/// else — including privilege errors — stays a real failure.
fn is_device_missing(output: &std::process::Output) -> bool {
    let stderr = command_stderr(output);
    stderr.contains("Cannot find device") && !is_privilege_denied(&stderr)
}

fn link_is_tap(name: &str) -> Result<bool, SdkError> {
    Ok(link_output(name)?.is_some_and(|output| {
        output.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("tun ") || line.starts_with("tap ") || line.contains(" tun ")
        })
    }))
}

fn command_output(program: &str, arguments: &[&str]) -> Result<std::process::Output, SdkError> {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
        Err(host_command_error(program, arguments, &output))
    }
}

fn host_command_error(
    program: &str,
    arguments: &[&str],
    output: &std::process::Output,
) -> SdkError {
    let command = if arguments.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {}", arguments.join(" "))
    };
    SdkError::HostCommand {
        program: program.to_owned(),
        reason: format!("{command} {}", output_failure_reason(output)),
    }
}

fn output_failure_reason(output: &std::process::Output) -> String {
    let stderr = command_stderr(output);
    let mut reason = format!("command exited with {}", output.status);
    if !stderr.is_empty() {
        const LIMIT: usize = 500;
        let truncated = if stderr.len() > LIMIT {
            format!("{}...", stderr[..LIMIT].trim_end())
        } else {
            stderr.clone()
        };
        let flattened = truncated.split_whitespace().collect::<Vec<_>>().join(" ");
        reason.push_str(": ");
        reason.push_str(&flattened);
    }
    if is_privilege_denied(&stderr) {
        reason.push_str("; ");
        reason.push_str(PRIVILEGE_HINT);
    }
    reason
}

fn command_stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

fn is_privilege_denied(text: &str) -> bool {
    let normalized = text.to_lowercase();
    normalized.contains("operation not permitted") || normalized.contains("permission denied")
}

fn run_ip(arguments: &[&str]) -> Result<(), SdkError> {
    run_command("ip", arguments)
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

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::{
        GUEST_INTERFACE, allocate_subnet, automatic_lan_candidates, guest_mac, is_device_missing,
        is_privilege_denied, parse_uplink_cidr, primary_device_is_missing, select_lan_offer,
        select_private_subnet, tap_name, validate_lan_candidate,
    };
    use crate::domain::lifecycle::NetworkMode;
    use crate::ports::network::{NetworkRequest, UplinkIdentity};

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
    fn exact_private_restore_rejects_a_used_guest_subnet() {
        let request = NetworkRequest {
            vm_name: "restored_vm".to_owned(),
            mode: NetworkMode::HostOnly,
            guest_mac: "02:fc:00:00:00:02".to_owned(),
            lan_address_override: None,
            guest_address_override: Some(Ipv4Addr::new(172, 30, 0, 2)),
            prefix_length_override: Some(30),
            gateway_override: Some(Ipv4Addr::new(172, 30, 0, 1)),
            exact_network_values: true,
        };
        let used = vec![(
            "existing_vm".to_owned(),
            IpAddr::V4(Ipv4Addr::new(172, 30, 0, 2)),
            "tap-existing_vm".to_owned(),
        )];

        let error = select_private_subnet(&used, &request)
            .expect_err("preserve policy must reject a subnet already used by another VM");

        assert!(matches!(
            error,
            crate::error::SdkError::SnapshotNetworkConflict { ref field, .. }
                if field == "guest/gateway IPv4 subnet"
        ));
    }

    #[test]
    fn exact_lan_restore_rejects_a_used_address_before_host_probing() {
        let uplink = UplinkIdentity {
            interface: "test0".to_owned(),
            address: Ipv4Addr::new(192, 168, 3, 12),
            prefix_length: 24,
            gateway: Some(Ipv4Addr::new(192, 168, 3, 1)),
            cidr: "192.168.3.12/24".to_owned(),
        };
        let archived_lan_address = Ipv4Addr::new(192, 168, 3, 50);
        let used = vec![(
            "existing_vm".to_owned(),
            IpAddr::V4(archived_lan_address),
            "tap-existing_vm".to_owned(),
        )];

        let error = select_lan_offer(&uplink, Some(archived_lan_address), None, &used)
            .expect_err("preserve policy must reject a LAN address already used by another VM");

        assert!(matches!(
            error,
            crate::error::SdkError::Network { ref operation, ref resource, .. }
                if operation == "allocate LAN address" && resource == "192.168.3.50"
        ));
    }

    #[test]
    fn names_and_macs_are_stable_and_fit_linux_limits() {
        assert_eq!(tap_name("build_vm"), tap_name("build_vm"));
        assert_eq!(guest_mac("build_vm"), guest_mac("build_vm"));
        assert!(tap_name("build_vm").len() <= 15);
        assert!(guest_mac("build_vm").starts_with("02:fc:"));
    }

    #[test]
    fn lan_candidates_prefer_the_upper_subnet_and_skip_reserved() {
        let (network, prefix) = parse_uplink_cidr("192.168.3.12/24").expect("CIDR should parse");
        let host = Ipv4Addr::new(192, 168, 3, 12);
        let gateway = Ipv4Addr::new(192, 168, 3, 1);
        let candidates = automatic_lan_candidates(network, prefix, host, Some(gateway));

        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], Ipv4Addr::new(192, 168, 3, 254));
        assert!(!candidates.contains(&host));
        assert!(!candidates.contains(&gateway));
        assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 3, 0)));
        assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 3, 255)));
    }

    #[test]
    fn lan_override_validation_rejects_reserved_addresses() {
        let (network, prefix) = parse_uplink_cidr("192.168.3.12/24").expect("CIDR should parse");
        let host = Ipv4Addr::new(192, 168, 3, 12);
        let gateway = Some(Ipv4Addr::new(192, 168, 3, 1));

        assert!(
            validate_lan_candidate(
                Ipv4Addr::new(192, 168, 3, 50),
                network,
                prefix,
                host,
                gateway
            )
            .is_ok()
        );
        assert!(
            validate_lan_candidate(
                Ipv4Addr::new(192, 168, 3, 0),
                network,
                prefix,
                host,
                gateway
            )
            .is_err()
        );
        assert!(
            validate_lan_candidate(
                Ipv4Addr::new(192, 168, 3, 255),
                network,
                prefix,
                host,
                gateway
            )
            .is_err()
        );
        assert!(validate_lan_candidate(host, network, prefix, host, gateway).is_err());
        assert!(
            validate_lan_candidate(
                Ipv4Addr::new(192, 168, 3, 1),
                network,
                prefix,
                host,
                gateway
            )
            .is_err()
        );
        assert!(
            validate_lan_candidate(
                Ipv4Addr::new(192, 168, 4, 50),
                network,
                prefix,
                host,
                gateway
            )
            .is_err()
        );
    }

    #[test]
    fn privilege_denied_detection_covers_common_kernel_messages() {
        assert!(is_privilege_denied(
            "RTNETLINK answers: Operation not permitted"
        ));
        assert!(is_privilege_denied("sysctl: permission denied on key"));
        assert!(!is_privilege_denied("Device does not exist"));
    }

    #[test]
    fn missing_device_counts_as_converged_without_hiding_denied_rights() {
        use std::os::unix::process::ExitStatusExt;
        use std::process::Output;

        let missing = Output {
            status: ExitStatusExt::from_raw(0x100),
            stdout: Vec::new(),
            stderr: b"Cannot find device \"tm-f76d0b8f\"".to_vec(),
        };
        assert!(is_device_missing(&missing));
        assert!(primary_device_is_missing(
            &crate::error::SdkError::HostCommand {
                program: "ip".to_owned(),
                reason: "host command ip failed: Cannot find device \"tm-f76d0b8f\"".to_owned(),
            }
        ));

        let denied = Output {
            status: ExitStatusExt::from_raw(0x100),
            stdout: Vec::new(),
            stderr: b"Cannot find device \"tm-x\"; Operation not permitted".to_vec(),
        };
        assert!(!is_device_missing(&denied));
    }

    #[test]
    fn iptables_comments_carry_sdk_ownership() {
        assert!(super::iptables_comment("tm-test", "nat").starts_with("sdk:taumaru:tm-test"));
    }

    #[test]
    fn host_only_boot_device_is_the_guest_interface() {
        assert_eq!(GUEST_INTERFACE, "eth0");
    }

    #[test]
    fn nat_specs_stay_paired_with_the_nat_table() {
        let host_only =
            super::host_only_nat_spec("tm-test", std::net::Ipv4Addr::new(172, 30, 0, 0));
        let routed =
            super::routed_nat_spec("enp3s0", "tm-test", std::net::Ipv4Addr::new(172, 30, 0, 0));
        for spec in [&host_only, &routed] {
            assert_eq!(spec.first().map(String::as_str), Some("POSTROUTING"));
        }
        // The NAT rules must be checked, inserted, and deleted against the
        // nat table: a filter-table `-C POSTROUTING` probe fails with
        // `Bad argument 'nat'` on real hosts (exit status 2).
        let host_rule: Vec<&str> = host_only.iter().map(String::as_str).collect();
        let routed_rule: Vec<&str> = routed.iter().map(String::as_str).collect();
        let host_check = super::nat_check_arguments(&host_rule);
        let routed_check = super::nat_check_arguments(&routed_rule);
        assert_eq!(&host_check[..3], ["-t", "nat", "-C"]);
        assert_eq!(&routed_check[..3], ["-t", "nat", "-C"]);
        assert_eq!(
            &super::nat_delete_arguments(&host_rule)[..3],
            ["-t", "nat", "-D"]
        );
        assert_eq!(
            &super::nat_delete_arguments(&routed_rule)[..3],
            ["-t", "nat", "-D"]
        );
    }
}
