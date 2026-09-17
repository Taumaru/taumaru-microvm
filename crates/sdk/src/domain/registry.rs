//! Taumaru Artifacts Registry
//!
//! Official Rust / serde_json types for:
//! https://artifacts.taumaru.com/v1/registry.json

use serde::{Deserialize, Serialize};

/* -------------------------------------------------------------------------- */
/*                                  REGISTRY                                  */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaumaruRegistry {
    pub schema_version: u32,

    pub registry: RegistryMetadata,

    pub kernels: Vec<Kernel>,

    pub binaries: Vec<BinaryPackage>,

    pub distributions: Vec<Distribution>,

    pub types: Vec<RegistryTypePackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryMetadata {
    /// Human-readable registry name.
    pub name: String,

    /// Registry content/release version.
    ///
    /// Example: `1.0.0`
    pub version: String,

    pub description: String,

    /// Example:
    /// `https://artifacts.taumaru.com/v1/`
    pub base_url: String,

    /// ISO 8601 timestamp.
    pub updated_at: String,
}

/* -------------------------------------------------------------------------- */
/*                                   COMMON                                   */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86_64,
    Aarch64,
    Arm,
    Riscv64,
    X86,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    Little,
    Big,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Linkage {
    Static,
    Dynamic,
}

/// Common metadata for a physical artifact stored in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactFile {
    /// Relative path from the registry root.
    pub path: String,

    /// Absolute public URL.
    pub url: String,

    pub filename: String,

    pub size_bytes: u64,

    pub sha256: String,

    pub mime_type: String,

    /// ISO 8601 timestamp.
    pub modified_at: String,
}

/* -------------------------------------------------------------------------- */
/*                                    ELF                                     */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElfMetadata {
    /// Example: `ELF64`
    pub class: String,

    pub endianness: Endianness,

    /// JSON field is named `type`.
    #[serde(rename = "type")]
    pub elf_type: String,

    pub machine: String,

    /// Hexadecimal ELF entry point.
    pub entry_point: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpreter: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needed_libraries: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkage: Option<Linkage>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripped: Option<bool>,
}

/* -------------------------------------------------------------------------- */
/*                                   KERNEL                                   */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kernel {
    /// Example:
    /// `linux-6.18.48-x86_64`
    pub id: String,

    pub name: String,

    pub display_name: String,

    /// Kernel-reported version.
    ///
    /// Example:
    /// `6.18.48+`
    pub version: String,

    pub architecture: Architecture,

    pub path: String,

    pub url: String,

    pub filename: String,

    pub size_bytes: u64,

    pub sha256: String,

    /// Examples:
    /// - `elf`
    /// - `linux-arm64-image`
    pub format: String,

    pub mime_type: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elf: Option<ElfMetadata>,

    pub modified_at: String,
}

/* -------------------------------------------------------------------------- */
/*                                  BINARIES                                  */
/* -------------------------------------------------------------------------- */

/// A versioned binary package.
///
/// Example:
///
/// `firecracker / 1.14.1 / x86_64`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryPackage {
    /// Example:
    /// `firecracker-1.14.1-x86_64`
    pub id: String,

    /// Machine-friendly product name.
    ///
    /// Example:
    /// `firecracker`
    pub name: String,

    /// Human-readable product name.
    ///
    /// Example:
    /// `Firecracker`
    pub display_name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Upstream/product version.
    ///
    /// Example:
    /// `1.14.1`
    pub version: String,

    pub architecture: Architecture,

    pub files: Vec<BinaryFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryFile {
    /// Logical component name.
    ///
    /// Examples:
    /// - `firecracker`
    /// - `jailer`
    /// - `firectl`
    pub name: String,

    pub path: String,

    pub url: String,

    pub filename: String,

    pub size_bytes: u64,

    pub sha256: String,

    pub mime_type: String,

    pub executable: bool,

    /// Numeric Unix file mode.
    ///
    /// Example:
    /// `755`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Human-readable Unix permissions.
    ///
    /// Example:
    /// `-rwxr-xr-x`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elf: Option<ElfMetadata>,

    pub modified_at: String,
}

/* -------------------------------------------------------------------------- */
/*                              DISTRIBUTIONS                                 */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    /// Example:
    /// `ubuntu-24.04`
    pub id: String,

    /// Example:
    /// `Ubuntu`
    pub name: String,

    /// Example:
    /// `Ubuntu 24.04 LTS`
    pub display_name: String,

    pub description: String,

    /// Machine-friendly distribution identifier.
    ///
    /// Example:
    /// `ubuntu`
    pub distribution: String,

    /// Example:
    /// `24.04`
    pub version: String,

    /// Example:
    /// `noble`
    pub codename: String,

    pub architecture: Architecture,

    /// Example:
    /// `Canonical`
    pub vendor: String,

    pub homepage: String,

    /// Kernel ID from `kernels`.
    pub default_kernel: String,

    /// Kernel IDs from `kernels`.
    pub supported_kernels: Vec<String>,

    pub boot: BootConfiguration,

    pub requirements: DistributionRequirements,

    pub images: Vec<DistributionImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootConfiguration {
    /// Example:
    /// `/dev/vda`
    pub root_device: String,

    pub kernel_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionRequirements {
    pub min_memory_mb: u64,

    pub min_vcpus: u32,
}

/* -------------------------------------------------------------------------- */
/*                                   IMAGES                                   */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionImage {
    /// Example:
    /// `ubuntu-24.04-docker`
    pub id: String,

    /// Example:
    /// `Docker`
    pub name: String,

    /// Example:
    /// `Ubuntu 24.04 LTS + Docker`
    pub display_name: String,

    pub description: String,

    /// Examples:
    /// - `minimal`
    /// - `docker`
    pub variant: String,

    pub path: String,

    pub url: String,

    pub filename: String,

    /// Example:
    /// `ext4`
    pub format: String,

    pub filesystem: FilesystemMetadata,

    pub size_bytes: u64,

    pub sha256: String,

    /// Examples:
    /// - `systemd`
    /// - `networking`
    /// - `docker`
    pub capabilities: Vec<String>,

    pub mime_type: String,

    pub modified_at: String,
}

/* -------------------------------------------------------------------------- */
/*                                FILESYSTEM                                  */
/* -------------------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemMetadata {
    /// JSON field is named `type`.
    #[serde(rename = "type")]
    pub filesystem_type: String,

    pub uuid: String,

    pub block_size: u64,

    pub block_count: u64,

    pub free_blocks: u64,

    pub inode_count: u64,

    pub free_inodes: u64,

    pub features: Vec<String>,
}

/* -------------------------------------------------------------------------- */
/*                                    TYPES                                   */
/* -------------------------------------------------------------------------- */

/// Official registry type definitions.
///
/// Current paths:
///
/// - `types/registry.ts`
/// - `types/registry.rs`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryTypePackage {
    /// Example:
    /// `registry-types`
    pub id: String,

    /// Example:
    /// `Registry Types`
    pub name: String,

    /// Example:
    /// `Taumaru Registry Types`
    pub display_name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Serialization/data format represented by these types.
    ///
    /// Current value:
    /// `serde_json`
    pub format: String,

    /// Registry schema version these definitions support.
    pub schema_version: u32,

    pub files: Vec<RegistryTypeFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryTypeFile {
    pub language: RegistryTypeLanguage,

    /// Example:
    /// `types/registry.rs`
    pub path: String,

    pub url: String,

    pub filename: String,

    pub size_bytes: u64,

    pub sha256: String,

    pub mime_type: String,

    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryTypeLanguage {
    Typescript,
    Rust,
}
