use std::path::{Component, Path, PathBuf};

use crate::error::SdkError;

use super::microvm::CreateMicroVmRequest;

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Validated creation input with paths normalized against the explicit SDK home.
#[derive(Clone, Debug)]
pub(crate) struct ValidatedCreateRequest {
    pub request: CreateMicroVmRequest,
    pub volume_path: PathBuf,
    pub memory_effective_mib: u64,
}

impl CreateMicroVmRequest {
    pub(crate) fn validate(self, home: &Path) -> Result<ValidatedCreateRequest, SdkError> {
        validate_vm_name(&self.name)?;
        validate_identifier(&self.distribution_id, "distribution_id")?;
        validate_identifier(&self.image_id, "image_id")?;
        validate_positive_u64(self.disk_size_bytes, "disk_size_bytes")?;
        validate_positive_u32(self.vcpu_count, "vcpu_count")?;
        validate_positive_u64(self.memory_bytes, "memory_bytes")?;
        if self.disk_size_bytes > i64::MAX as u64 {
            return Err(SdkError::InvalidRequest {
                field: "disk_size_bytes".to_owned(),
                reason: "value exceeds the SQLite INTEGER range".to_owned(),
            });
        }
        if self.memory_bytes > i64::MAX as u64 {
            return Err(SdkError::InvalidRequest {
                field: "memory_bytes".to_owned(),
                reason: "value exceeds the SQLite INTEGER range".to_owned(),
            });
        }
        let memory_effective_mib = self
            .memory_bytes
            .checked_add(BYTES_PER_MIB - 1)
            .ok_or_else(|| SdkError::InvalidRequest {
                field: "memory_bytes".to_owned(),
                reason: "value cannot be converted to MiB".to_owned(),
            })?
            / BYTES_PER_MIB;
        if memory_effective_mib == 0 || memory_effective_mib > i64::MAX as u64 {
            return Err(SdkError::InvalidRequest {
                field: "memory_bytes".to_owned(),
                reason: "value cannot be represented as a Firecracker MiB value".to_owned(),
            });
        }

        let volume_path = match self.volume_path.as_deref() {
            Some(path) => normalize_volume_path(path)?,
            None => home.join("vms").join(&self.name),
        };
        if volume_path.as_os_str().is_empty() {
            return Err(SdkError::InvalidRequest {
                field: "volume_path".to_owned(),
                reason: "path must not be empty".to_owned(),
            });
        }

        Ok(ValidatedCreateRequest {
            request: self,
            volume_path,
            memory_effective_mib,
        })
    }
}

pub(crate) fn validate_vm_name(name: &str) -> Result<(), SdkError> {
    let length = name.len();
    let valid_length = (1..=64).contains(&length);
    let valid_chars = name.bytes().enumerate().all(|(index, byte)| {
        if index == 0 {
            byte.is_ascii_alphanumeric()
        } else {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
        }
    });
    if !name.is_ascii() || !valid_length || !valid_chars {
        return Err(SdkError::InvalidRequest {
            field: "name".to_owned(),
            reason: "must be 1-64 ASCII characters, start with a letter or digit, and contain only letters, digits, '-' or '_'".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_identifier(value: &str, field: &str) -> Result<(), SdkError> {
    if value.is_empty()
        || !value.is_ascii()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(SdkError::InvalidRequest {
            field: field.to_owned(),
            reason: "must be a non-empty safe registry identifier".to_owned(),
        });
    }
    Ok(())
}

fn validate_positive_u64(value: u64, field: &str) -> Result<(), SdkError> {
    if value == 0 {
        return Err(SdkError::InvalidRequest {
            field: field.to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

fn validate_positive_u32(value: u32, field: &str) -> Result<(), SdkError> {
    if value == 0 {
        return Err(SdkError::InvalidRequest {
            field: field.to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn normalize_volume_path(path: &Path) -> Result<PathBuf, SdkError> {
    if !path.is_absolute() {
        return Err(SdkError::InvalidRequest {
            field: "volume_path".to_owned(),
            reason: "must be an absolute directory path".to_owned(),
        });
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized != Path::new(std::path::MAIN_SEPARATOR_STR) {
                    normalized.pop();
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(SdkError::InvalidRequest {
            field: "volume_path".to_owned(),
            reason: "must resolve to a directory".to_owned(),
        });
    }
    Ok(normalized)
}

pub(crate) fn minimum_memory_bytes(min_memory_mb: u64) -> Result<u64, SdkError> {
    min_memory_mb
        .checked_mul(BYTES_PER_MIB)
        .ok_or_else(|| SdkError::InvalidMetadata {
            artifact: "distribution requirements".to_owned(),
            reason: "minimum memory exceeds the supported range".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CreateMicroVmRequest, validate_vm_name};

    #[test]
    fn validates_path_friendly_names() {
        assert!(validate_vm_name("vm_01-test").is_ok());
        assert!(validate_vm_name("vm name").is_err());
        assert!(validate_vm_name("vm.name").is_err());
        assert!(validate_vm_name("../vm").is_err());
        assert!(validate_vm_name("").is_err());
    }

    #[test]
    fn derives_the_default_volume_path() {
        let request = CreateMicroVmRequest {
            name: "vm".to_owned(),
            distribution_id: "dist".to_owned(),
            image_id: "image".to_owned(),
            disk_size_bytes: 1,
            vcpu_count: 1,
            memory_bytes: 1,
            expose_on_lan: false,
            volume_path: None,
        };
        let validated = request
            .validate(Path::new("/var/lib/taumaru"))
            .expect("request should validate");
        assert_eq!(validated.volume_path, Path::new("/var/lib/taumaru/vms/vm"));
    }

    #[test]
    fn rounds_memory_up_to_the_effective_firecracker_mib() {
        let request = CreateMicroVmRequest {
            name: "vm".to_owned(),
            distribution_id: "dist".to_owned(),
            image_id: "image".to_owned(),
            disk_size_bytes: 1,
            vcpu_count: 1,
            memory_bytes: 1024 * 1024 + 1,
            expose_on_lan: false,
            volume_path: None,
        };

        let validated = request
            .validate(Path::new("/var/lib/taumaru"))
            .expect("request should validate");
        assert_eq!(validated.memory_effective_mib, 2);
    }

    #[test]
    fn rejects_values_that_cannot_fit_sqlite_integer_columns() {
        let request = CreateMicroVmRequest {
            name: "vm".to_owned(),
            distribution_id: "dist".to_owned(),
            image_id: "image".to_owned(),
            disk_size_bytes: i64::MAX as u64 + 1,
            vcpu_count: 1,
            memory_bytes: 1,
            expose_on_lan: false,
            volume_path: None,
        };

        assert!(request.validate(Path::new("/var/lib/taumaru")).is_err());
    }
}
