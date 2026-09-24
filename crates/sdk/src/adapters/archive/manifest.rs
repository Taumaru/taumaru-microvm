use std::net::Ipv4Addr;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::config::{validate_identifier, validate_vm_name};
use crate::domain::snapshot::SnapshotAddressPolicy;
use crate::error::SdkError;

pub(crate) const FORMAT_ID: &str = "taumaru.microvm.snapshot";
pub(crate) const FORMAT_VERSION: u32 = 2;
pub(crate) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const ROOTFS_MEMBER: &str = "payload/rootfs.ext4";
pub(crate) const KERNEL_MEMBER: &str = "payload/kernel/vmlinux";
pub(crate) const PRIVATE_KEY_MEMBER: &str = "payload/ssh/id_ed25519";
pub(crate) const PUBLIC_KEY_MEMBER: &str = "payload/ssh/id_ed25519.pub";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotManifest {
    pub format: String,
    pub format_version: u32,
    pub created_at_unix_seconds: u64,
    pub vm: VmManifest,
    pub compatibility: CompatibilityManifest,
    pub boot: BootManifest,
    pub network: NetworkManifest,
    pub ssh: SshManifest,
    pub consistency: ConsistencyManifest,
    pub payloads: Vec<PayloadRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VmManifest {
    pub name: String,
    pub disk_size_bytes: u64,
    pub memory_bytes: u64,
    pub memory_effective_mib: u64,
    pub vcpu_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompatibilityManifest {
    pub host_os: String,
    pub guest_architecture: String,
    pub requires_kvm: bool,
    pub runtime: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BootManifest {
    pub distribution_id: String,
    pub distribution_name: String,
    pub distribution_version: String,
    pub image_id: String,
    pub image_sha256: String,
    pub kernel: KernelManifest,
    pub root_device: String,
    pub kernel_args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KernelManifest {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub architecture: String,
    pub registry_path: String,
    pub registry_url: String,
    pub filename: String,
    pub format: String,
    pub mime_type: String,
    pub modified_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "address_policy", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NetworkManifest {
    PreserveIpv4 {
        guest_ipv4: Ipv4Addr,
        prefix_length: u8,
        guest_gateway_ipv4: Option<Ipv4Addr>,
        lan_ipv4: Option<Ipv4Addr>,
        mode: String,
        expose_on_lan: bool,
        guest_mac: String,
    },
    RegenerateIpv4 {
        expose_on_lan: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SshManifest {
    pub user: String,
    pub port: u16,
    pub key_type: String,
    pub public_key_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConsistencyManifest {
    pub kind: String,
    pub includes_guest_memory: bool,
    pub includes_process_state: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadRecord {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub mode: u32,
}

impl SnapshotManifest {
    pub(crate) fn address_policy(&self) -> SnapshotAddressPolicy {
        match self.network {
            NetworkManifest::PreserveIpv4 { .. } => SnapshotAddressPolicy::PreserveIpv4,
            NetworkManifest::RegenerateIpv4 { .. } => SnapshotAddressPolicy::RegenerateIpv4,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), SdkError> {
        if self.format != FORMAT_ID {
            return Err(invalid_manifest("the format identifier is not supported"));
        }
        if self.format_version != FORMAT_VERSION {
            return Err(SdkError::UnsupportedSnapshotVersion {
                actual: self.format_version,
                supported: FORMAT_VERSION,
            });
        }
        validate_vm_name(&self.vm.name).map_err(|_| invalid_manifest("VM name is unsafe"))?;
        if self.vm.disk_size_bytes == 0
            || self.vm.memory_bytes == 0
            || self.vm.memory_effective_mib == 0
            || self.vm.vcpu_count == 0
        {
            return Err(invalid_manifest(
                "VM resource limits must be greater than zero",
            ));
        }
        require_text(&self.compatibility.guest_architecture, "guest architecture")?;
        if self.compatibility.host_os != "linux" || self.compatibility.runtime != "firecracker" {
            return Err(invalid_manifest(
                "runtime compatibility metadata is unsupported",
            ));
        }
        if !self.compatibility.requires_kvm {
            return Err(invalid_manifest(
                "the archive must declare its KVM requirement",
            ));
        }
        for (value, field) in [
            (&self.boot.distribution_id, "distribution ID"),
            (&self.boot.distribution_name, "distribution name"),
            (&self.boot.distribution_version, "distribution version"),
            (&self.boot.image_id, "image ID"),
            (&self.boot.root_device, "root device"),
        ] {
            require_text(value, field)?;
        }
        validate_identifier(&self.boot.distribution_id, "distribution_id")
            .map_err(|_| invalid_manifest("distribution ID is unsafe"))?;
        validate_identifier(&self.boot.image_id, "image_id")
            .map_err(|_| invalid_manifest("image ID is unsafe"))?;
        validate_kernel(&self.boot.kernel)?;
        match &self.network {
            NetworkManifest::PreserveIpv4 {
                prefix_length,
                mode,
                expose_on_lan,
                guest_mac,
                lan_ipv4,
                ..
            } => {
                if !(1..=32).contains(prefix_length) {
                    return Err(invalid_manifest(
                        "IPv4 prefix length must be between 1 and 32",
                    ));
                }
                if mode != "host_only" && mode != "lan" {
                    return Err(SdkError::UnsupportedSnapshotPolicy {
                        policy: mode.clone(),
                    });
                }
                if (*mode == "lan") != *expose_on_lan || (*mode == "lan") != lan_ipv4.is_some() {
                    return Err(invalid_manifest(
                        "preserved LAN mode and address fields are inconsistent",
                    ));
                }
                if !valid_mac(guest_mac) {
                    return Err(invalid_manifest("preserved guest MAC is invalid"));
                }
            }
            NetworkManifest::RegenerateIpv4 { .. } => {}
        }
        if self.ssh.user.is_empty() || self.ssh.port == 0 || self.ssh.key_type != "ed25519" {
            return Err(invalid_manifest(
                "SSH metadata is incomplete or unsupported",
            ));
        }
        if !self.ssh.public_key_fingerprint.starts_with("SHA256:") {
            return Err(invalid_manifest("SSH public-key fingerprint is invalid"));
        }
        if self.consistency.kind != "disk_only_crash_consistent"
            || self.consistency.includes_guest_memory
            || self.consistency.includes_process_state
        {
            return Err(invalid_manifest(
                "snapshot consistency metadata is unsupported",
            ));
        }
        validate_payloads(self)?;
        Ok(())
    }
}

pub(crate) fn parse_manifest(bytes: &[u8]) -> Result<SnapshotManifest, SdkError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| SdkError::SnapshotArchive {
            operation: "parse snapshot manifest",
            reason: error.to_string(),
        })?;
    let version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| invalid_manifest("format_version is missing or invalid"))?;
    if version != FORMAT_VERSION {
        return Err(SdkError::UnsupportedSnapshotVersion {
            actual: version,
            supported: FORMAT_VERSION,
        });
    }
    let manifest: SnapshotManifest =
        serde_json::from_value(value).map_err(|error| SdkError::SnapshotArchive {
            operation: "parse snapshot manifest",
            reason: error.to_string(),
        })?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_kernel(kernel: &KernelManifest) -> Result<(), SdkError> {
    for (value, field) in [
        (&kernel.id, "kernel ID"),
        (&kernel.name, "kernel name"),
        (&kernel.display_name, "kernel display name"),
        (&kernel.version, "kernel version"),
        (&kernel.architecture, "kernel architecture"),
        (&kernel.registry_path, "kernel registry path"),
        (&kernel.registry_url, "kernel registry URL"),
        (&kernel.filename, "kernel filename"),
        (&kernel.format, "kernel format"),
        (&kernel.mime_type, "kernel MIME type"),
        (&kernel.modified_at, "kernel modified time"),
    ] {
        require_text(value, field)?;
    }
    validate_identifier(&kernel.id, "kernel_id")
        .map_err(|_| invalid_manifest("kernel ID is unsafe"))?;
    let filename = Path::new(&kernel.filename);
    if filename.components().count() != 1
        || filename.file_name().is_none()
        || kernel.filename == "."
        || kernel.filename == ".."
        || !kernel.filename.is_ascii()
        || kernel
            .filename
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    {
        return Err(invalid_manifest(
            "kernel filename must be a safe single path component",
        ));
    }
    Ok(())
}

fn validate_payloads(manifest: &SnapshotManifest) -> Result<(), SdkError> {
    let expected = [
        ROOTFS_MEMBER,
        KERNEL_MEMBER,
        PRIVATE_KEY_MEMBER,
        PUBLIC_KEY_MEMBER,
    ];
    if manifest.payloads.len() != expected.len() {
        return Err(invalid_manifest(
            "payload list does not contain the exact required member set",
        ));
    }
    for member in expected {
        let mut matches = manifest
            .payloads
            .iter()
            .filter(|payload| payload.path == member);
        let payload = matches
            .next()
            .ok_or_else(|| invalid_manifest("a required payload record is missing"))?;
        if matches.next().is_some() {
            return Err(invalid_manifest("payload list contains a duplicate member"));
        }
        if payload.size_bytes == 0 || !valid_sha256(&payload.sha256) {
            return Err(invalid_manifest("payload size or SHA-256 value is invalid"));
        }
        let expected_mode = match member {
            ROOTFS_MEMBER | PRIVATE_KEY_MEMBER => 0o600,
            _ => 0o644,
        };
        if payload.mode != expected_mode {
            return Err(invalid_manifest("payload file mode is invalid"));
        }
    }
    let root = manifest
        .payloads
        .iter()
        .find(|payload| payload.path == ROOTFS_MEMBER);
    if root.is_none_or(|payload| payload.size_bytes != manifest.vm.disk_size_bytes) {
        return Err(invalid_manifest(
            "root disk payload size does not match the VM disk size",
        ));
    }
    Ok(())
}

fn require_text(value: &str, field: &str) -> Result<(), SdkError> {
    if value.trim().is_empty() {
        return Err(invalid_manifest(&format!("{field} is empty")));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_mac(value: &str) -> bool {
    let octets = value.split(':').collect::<Vec<_>>();
    octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn invalid_manifest(reason: &str) -> SdkError {
    SdkError::InvalidSnapshotManifest {
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FORMAT_ID, KERNEL_MEMBER, NetworkManifest, SnapshotManifest, parse_manifest};

    fn minimal_manifest(network: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "format": FORMAT_ID,
            "format_version": 2,
            "created_at_unix_seconds": 1,
            "vm": {"name":"demo","disk_size_bytes":4096,"memory_bytes":1024,"memory_effective_mib":1,"vcpu_count":1},
            "compatibility": {"host_os":"linux","guest_architecture":"x86_64","requires_kvm":true,"runtime":"firecracker"},
            "boot": {
                "distribution_id":"distro","distribution_name":"Linux","distribution_version":"1.0",
                "image_id":"image","image_sha256":"a".repeat(64),
                "kernel": {"id":"kernel","name":"kernel","display_name":"Kernel","version":"6.0","architecture":"x86_64","registry_path":"kernels/kernel/vmlinux","registry_url":"https://example.test/kernel","filename":"vmlinux","format":"elf","mime_type":"application/octet-stream","modified_at":"2026-01-01T00:00:00Z"},
                "root_device":"/dev/vda","kernel_args":["console=ttyS0"]
            },
            "network": network,
            "ssh": {"user":"root","port":22,"key_type":"ed25519","public_key_fingerprint":"SHA256:abc"},
            "consistency": {"kind":"disk_only_crash_consistent","includes_guest_memory":false,"includes_process_state":false},
            "payloads": [
                {"path":"payload/rootfs.ext4","size_bytes":4096,"sha256":"a".repeat(64),"mode":384},
                {"path":KERNEL_MEMBER,"size_bytes":1,"sha256":"b".repeat(64),"mode":420},
                {"path":"payload/ssh/id_ed25519","size_bytes":1,"sha256":"c".repeat(64),"mode":384},
                {"path":"payload/ssh/id_ed25519.pub","size_bytes":1,"sha256":"d".repeat(64),"mode":420}
            ]
        })
    }

    #[test]
    fn regenerate_policy_serializes_only_lan_exposure() {
        let value = minimal_manifest(serde_json::json!({
            "address_policy":"regenerate_ipv4", "expose_on_lan":true
        }));
        let manifest = parse_manifest(&serde_json::to_vec(&value).expect("serialize fixture"))
            .expect("valid regenerate manifest");
        assert!(matches!(
            manifest.network,
            NetworkManifest::RegenerateIpv4 {
                expose_on_lan: true
            }
        ));
        let serialized = serde_json::to_value(manifest).expect("serialize manifest");
        assert_eq!(
            serialized["network"]
                .as_object()
                .expect("network map")
                .len(),
            2
        );
    }

    #[test]
    fn preserve_policy_accepts_ipv4_and_requires_consistent_lan_fields() {
        let value = minimal_manifest(serde_json::json!({
            "address_policy":"preserve_ipv4", "guest_ipv4":"192.0.2.2", "prefix_length":30,
            "guest_gateway_ipv4":"192.0.2.1", "lan_ipv4":"198.51.100.4", "mode":"lan",
            "expose_on_lan":true, "guest_mac":"02:00:00:00:00:01"
        }));
        let bytes = serde_json::to_vec(&value).expect("serialize fixture");
        assert!(parse_manifest(&bytes).is_ok());
    }

    #[test]
    fn rejects_ipv6_and_version_one_manifests() {
        let ipv6 = minimal_manifest(serde_json::json!({
            "address_policy":"preserve_ipv4", "guest_ipv4":"2001:db8::1", "prefix_length":64,
            "guest_gateway_ipv4":null, "lan_ipv4":null, "mode":"host_only",
            "expose_on_lan":false, "guest_mac":"02:00:00:00:00:01"
        }));
        assert!(parse_manifest(&serde_json::to_vec(&ipv6).expect("serialize fixture")).is_err());

        let mut v1 = minimal_manifest(serde_json::json!({
            "address_policy":"regenerate_ipv4", "expose_on_lan":false
        }));
        v1["format_version"] = serde_json::json!(1);
        assert!(matches!(
            parse_manifest(&serde_json::to_vec(&v1).expect("serialize fixture")),
            Err(crate::error::SdkError::UnsupportedSnapshotVersion { actual: 1, .. })
        ));
    }

    #[test]
    fn rejects_source_assignments_in_regenerate_policy() {
        let value = minimal_manifest(serde_json::json!({
            "address_policy":"regenerate_ipv4", "expose_on_lan":false,
            "guest_ipv4":"192.0.2.2"
        }));
        assert!(parse_manifest(&serde_json::to_vec(&value).expect("serialize fixture")).is_err());
    }

    #[test]
    fn typed_preserve_network_variant_is_serializable() {
        let network = NetworkManifest::PreserveIpv4 {
            guest_ipv4: "192.0.2.2".parse().expect("valid IPv4"),
            prefix_length: 30,
            guest_gateway_ipv4: None,
            lan_ipv4: None,
            mode: "host_only".to_owned(),
            expose_on_lan: false,
            guest_mac: "02:00:00:00:00:01".to_owned(),
        };
        let serialized = serde_json::to_value(network).expect("serialize typed network");
        assert_eq!(serialized["address_policy"], "preserve_ipv4");
        assert!(serde_json::from_value::<SnapshotManifest>(minimal_manifest(serialized)).is_ok());
    }
}
