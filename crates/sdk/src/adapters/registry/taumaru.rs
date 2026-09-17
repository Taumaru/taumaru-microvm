use std::collections::HashSet;

use reqwest::{Client, Response, Url};

use crate::domain::artifact::{is_valid_sha256, validate_registry_path};
use crate::domain::registry::{
    BinaryFile, BinaryPackage, Distribution, DistributionImage, Kernel, RegistryTypeFile,
    TaumaruRegistry,
};
use crate::error::SdkError;
use crate::ports::artifacts::{ArtifactSource, RegistryFuture};

const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Asynchronous client for the public Taumaru Artifacts Registry.
#[derive(Clone, Debug)]
pub(crate) struct TaumaruRegistryClient {
    client: Client,
    base_url: Url,
}

impl TaumaruRegistryClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, SdkError> {
        let parsed = Url::parse(base_url).map_err(|_| SdkError::InvalidUrl {
            url: base_url.to_owned(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(SdkError::InvalidUrl {
                url: base_url.to_owned(),
            });
        }
        let client = Client::builder().build()?;
        Ok(Self {
            client,
            base_url: parsed,
        })
    }

    fn manifest_url(&self) -> Url {
        self.base_url.clone()
    }
}

impl ArtifactSource for TaumaruRegistryClient {
    fn fetch_manifest(&self) -> RegistryFuture<'_, TaumaruRegistry> {
        let client = self.client.clone();
        let url = self.manifest_url();
        Box::pin(async move {
            let response = client.get(url.clone()).send().await?;
            let response = response.error_for_status().map_err(|error| {
                if let Some(status) = error.status() {
                    SdkError::RegistryStatus {
                        url: url.to_string(),
                        status,
                    }
                } else {
                    SdkError::RegistryTransport(error)
                }
            })?;
            let bytes = response.bytes().await?;
            let manifest: TaumaruRegistry =
                serde_json::from_slice(&bytes).map_err(SdkError::RegistryDecode)?;
            validate_manifest(&manifest)?;
            Ok(manifest)
        })
    }

    fn fetch_file(&self, url: &str) -> RegistryFuture<'_, Response> {
        let client = self.client.clone();
        let url_string = url.to_owned();
        Box::pin(async move {
            let parsed = Url::parse(&url_string).map_err(|_| SdkError::InvalidUrl {
                url: url_string.clone(),
            })?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(SdkError::InvalidUrl { url: url_string });
            }
            let response = client.get(parsed.clone()).send().await?;
            response.error_for_status().map_err(|error| {
                if let Some(status) = error.status() {
                    SdkError::RegistryStatus {
                        url: parsed.to_string(),
                        status,
                    }
                } else {
                    SdkError::RegistryTransport(error)
                }
            })
        })
    }
}

pub(crate) fn validate_manifest(manifest: &TaumaruRegistry) -> Result<(), SdkError> {
    if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(SdkError::UnsupportedSchema {
            actual: manifest.schema_version,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }
    validate_required_text(&manifest.registry.name, "registry", "name")?;
    validate_required_text(&manifest.registry.version, "registry", "version")?;
    validate_required_text(&manifest.registry.updated_at, "registry", "updated_at")?;
    validate_url(&manifest.registry.base_url, "registry")?;
    validate_id_set(
        manifest.kernels.iter().map(|kernel| kernel.id.as_str()),
        "kernel",
    )?;
    validate_id_set(
        manifest.binaries.iter().map(|binary| binary.id.as_str()),
        "binary package",
    )?;
    validate_id_set(
        manifest
            .distributions
            .iter()
            .map(|distribution| distribution.id.as_str()),
        "distribution",
    )?;

    for kernel in &manifest.kernels {
        validate_kernel(kernel)?;
    }
    for binary in &manifest.binaries {
        validate_binary(binary)?;
    }

    let kernel_ids: HashSet<&str> = manifest
        .kernels
        .iter()
        .map(|kernel| kernel.id.as_str())
        .collect();
    for distribution in &manifest.distributions {
        validate_distribution(distribution, &kernel_ids)?;
    }
    for registry_types in &manifest.types {
        for file in &registry_types.files {
            validate_type_file(file)?;
        }
    }
    Ok(())
}

fn validate_kernel(kernel: &Kernel) -> Result<(), SdkError> {
    validate_id(&kernel.id, "kernel")?;
    validate_required_text(&kernel.name, &kernel.id, "name")?;
    validate_required_text(&kernel.display_name, &kernel.id, "display_name")?;
    validate_required_text(&kernel.version, &kernel.id, "version")?;
    validate_required_text(&kernel.format, &kernel.id, "format")?;
    validate_required_text(&kernel.mime_type, &kernel.id, "mime_type")?;
    validate_required_text(&kernel.modified_at, &kernel.id, "modified_at")?;
    validate_common_file(
        &kernel.id,
        &kernel.path,
        &kernel.url,
        &kernel.filename,
        kernel.size_bytes,
        &kernel.sha256,
    )
}

fn validate_binary(binary: &BinaryPackage) -> Result<(), SdkError> {
    validate_id(&binary.id, "binary package")?;
    validate_required_text(&binary.name, &binary.id, "name")?;
    validate_required_text(&binary.display_name, &binary.id, "display_name")?;
    validate_required_text(&binary.version, &binary.id, "version")?;
    let mut names = HashSet::new();
    for file in &binary.files {
        validate_id(&file.name, "binary component")?;
        if !names.insert(file.name.as_str()) {
            return Err(SdkError::invalid_metadata(
                &binary.id,
                format!("duplicate binary component {}", file.name),
            ));
        }
        validate_binary_file(binary, file)?;
    }
    if binary.files.is_empty() {
        return Err(SdkError::invalid_metadata(
            &binary.id,
            "binary package has no files",
        ));
    }
    Ok(())
}

fn validate_binary_file(binary: &BinaryPackage, file: &BinaryFile) -> Result<(), SdkError> {
    let artifact = format!("{}/{}", binary.id, file.name);
    validate_required_text(&file.mime_type, &artifact, "mime_type")?;
    validate_required_text(&file.modified_at, &artifact, "modified_at")?;
    validate_common_file(
        &artifact,
        &file.path,
        &file.url,
        &file.filename,
        file.size_bytes,
        &file.sha256,
    )?;
    if let Some(mode) = &file.mode
        && (mode.is_empty() || !mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte)))
    {
        return Err(SdkError::invalid_metadata(
            artifact,
            "binary mode is not octal",
        ));
    }
    Ok(())
}

fn validate_distribution(
    distribution: &Distribution,
    kernel_ids: &HashSet<&str>,
) -> Result<(), SdkError> {
    validate_id(&distribution.id, "distribution")?;
    validate_required_text(&distribution.name, &distribution.id, "name")?;
    validate_required_text(&distribution.display_name, &distribution.id, "display_name")?;
    validate_required_text(&distribution.description, &distribution.id, "description")?;
    validate_required_text(&distribution.distribution, &distribution.id, "distribution")?;
    validate_required_text(&distribution.version, &distribution.id, "version")?;
    validate_required_text(&distribution.codename, &distribution.id, "codename")?;
    validate_required_text(&distribution.vendor, &distribution.id, "vendor")?;
    validate_required_text(&distribution.homepage, &distribution.id, "homepage")?;
    validate_required_text(
        &distribution.boot.root_device,
        &distribution.id,
        "boot.root_device",
    )?;
    if distribution.requirements.min_vcpus == 0 {
        return Err(SdkError::invalid_metadata(
            &distribution.id,
            "requirements.min_vcpus must be greater than zero",
        ));
    }
    if !kernel_ids.contains(distribution.default_kernel.as_str()) {
        return Err(SdkError::invalid_metadata(
            &distribution.id,
            format!("default kernel {} is absent", distribution.default_kernel),
        ));
    }
    let mut supported = HashSet::new();
    for kernel_id in &distribution.supported_kernels {
        if !kernel_ids.contains(kernel_id.as_str()) {
            return Err(SdkError::invalid_metadata(
                &distribution.id,
                format!("supported kernel {kernel_id} is absent"),
            ));
        }
        if !supported.insert(kernel_id.as_str()) {
            return Err(SdkError::invalid_metadata(
                &distribution.id,
                format!("duplicate supported kernel {kernel_id}"),
            ));
        }
    }
    let mut image_ids = HashSet::new();
    for image in &distribution.images {
        if !image_ids.insert(image.id.as_str()) {
            return Err(SdkError::invalid_metadata(
                &distribution.id,
                format!("duplicate distribution image {}", image.id),
            ));
        }
        validate_image(distribution, image)?;
    }
    if distribution.images.is_empty() {
        return Err(SdkError::invalid_metadata(
            &distribution.id,
            "distribution has no images",
        ));
    }
    Ok(())
}

fn validate_image(distribution: &Distribution, image: &DistributionImage) -> Result<(), SdkError> {
    validate_id(&image.id, "distribution image")?;
    let artifact = format!("{}/{}", distribution.id, image.id);
    validate_required_text(&image.name, &artifact, "name")?;
    validate_required_text(&image.display_name, &artifact, "display_name")?;
    validate_required_text(&image.description, &artifact, "description")?;
    validate_required_text(&image.variant, &artifact, "variant")?;
    validate_required_text(&image.format, &artifact, "format")?;
    validate_required_text(
        &image.filesystem.filesystem_type,
        &artifact,
        "filesystem.type",
    )?;
    validate_required_text(&image.filesystem.uuid, &artifact, "filesystem.uuid")?;
    validate_required_text(&image.mime_type, &artifact, "mime_type")?;
    validate_required_text(&image.modified_at, &artifact, "modified_at")?;
    validate_common_file(
        &artifact,
        &image.path,
        &image.url,
        &image.filename,
        image.size_bytes,
        &image.sha256,
    )
}

fn validate_type_file(file: &RegistryTypeFile) -> Result<(), SdkError> {
    validate_required_text(&file.filename, "registry type", "filename")?;
    validate_required_text(&file.modified_at, "registry type", "modified_at")?;
    validate_common_file(
        &format!("registry type {}", file.filename),
        &file.path,
        &file.url,
        &file.filename,
        file.size_bytes,
        &file.sha256,
    )
}

fn validate_common_file(
    artifact: &str,
    path: &str,
    url: &str,
    filename: &str,
    _size_bytes: u64,
    sha256: &str,
) -> Result<(), SdkError> {
    if !validate_registry_path(path) {
        return Err(SdkError::invalid_metadata(
            artifact,
            "registry path is not a safe relative path",
        ));
    }
    validate_url(url, artifact)?;
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename == "."
        || filename == ".."
    {
        return Err(SdkError::invalid_metadata(
            artifact,
            "filename is not a single safe path component",
        ));
    }
    if !is_valid_sha256(sha256) {
        return Err(SdkError::invalid_metadata(
            artifact,
            "SHA-256 must be 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn validate_url(url: &str, artifact: &str) -> Result<(), SdkError> {
    let parsed = Url::parse(url).map_err(|_| SdkError::InvalidUrl {
        url: url.to_owned(),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(SdkError::invalid_metadata(
            artifact,
            "URL scheme must be HTTP or HTTPS",
        ));
    }
    Ok(())
}

fn validate_id_set<'a>(ids: impl IntoIterator<Item = &'a str>, kind: &str) -> Result<(), SdkError> {
    let mut seen = HashSet::new();
    for id in ids {
        validate_id(id, kind)?;
        if !seen.insert(id) {
            return Err(SdkError::invalid_metadata(
                kind,
                format!("duplicate registry identifier {id}"),
            ));
        }
    }
    Ok(())
}

fn validate_id(id: &str, kind: &str) -> Result<(), SdkError> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(SdkError::invalid_metadata(
            id,
            format!("{kind} identifier is empty or unsafe"),
        ));
    }
    Ok(())
}

fn validate_required_text(value: &str, artifact: &str, field: &str) -> Result<(), SdkError> {
    if value.trim().is_empty() {
        return Err(SdkError::invalid_metadata(
            artifact,
            format!("{field} must not be empty"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::domain::registry::TaumaruRegistry;

    use super::validate_manifest;

    fn fixture() -> TaumaruRegistry {
        serde_json::from_str(include_str!("../../../tests/fixtures/manifest.json"))
            .expect("fixture manifest should decode")
    }

    #[test]
    fn accepts_the_supported_fixture_manifest() {
        assert!(validate_manifest(&fixture()).is_ok());
    }

    #[test]
    fn rejects_an_unsupported_schema_version() {
        let mut manifest = fixture();
        manifest.schema_version = 2;

        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_duplicate_kernel_ids() {
        let mut manifest = fixture();
        let duplicate = manifest.kernels[0].clone();
        manifest.kernels.push(duplicate);

        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_invalid_digest_and_url_metadata() {
        let mut invalid_digest = fixture();
        invalid_digest.kernels[0].sha256 = "bad".to_owned();
        assert!(validate_manifest(&invalid_digest).is_err());

        let mut invalid_url = fixture();
        invalid_url.kernels[0].url = "not a url".to_owned();
        assert!(validate_manifest(&invalid_url).is_err());
    }

    #[test]
    fn rejects_missing_required_metadata() {
        let mut manifest = fixture();
        manifest.binaries[0].files[0].mime_type.clear();
        assert!(validate_manifest(&manifest).is_err());

        let mut invalid_vcpus = fixture();
        invalid_vcpus.distributions[0].requirements.min_vcpus = 0;
        assert!(validate_manifest(&invalid_vcpus).is_err());
    }
}
