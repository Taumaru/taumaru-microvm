use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};

use crate::error::SdkError;
use crate::ports::credentials::{CredentialStore, GeneratedCredential};

/// Rust-backed Ed25519 credential adapter.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ed25519CredentialStore;

impl CredentialStore for Ed25519CredentialStore {
    fn generate(&self, volume_path: &Path) -> Result<GeneratedCredential, SdkError> {
        let ssh_directory = volume_path.join("ssh");
        create_directory(&ssh_directory)?;
        set_mode(&ssh_directory, 0o700)?;

        let private_key_path = ssh_directory.join("id_ed25519");
        let public_key_path = ssh_directory.join("id_ed25519.pub");
        reject_existing_managed_file(&private_key_path)?;
        reject_existing_managed_file(&public_key_path)?;

        let rng = SystemRandom::new();
        let mut seed = [0_u8; 32];
        rng.fill(&mut seed).map_err(|_| SdkError::Credential {
            operation: "generate Ed25519 seed".to_owned(),
            path: private_key_path.clone(),
            reason: "the operating system secure random source failed".to_owned(),
        })?;
        let key_pair =
            Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| SdkError::Credential {
                operation: "construct Ed25519 key pair".to_owned(),
                path: private_key_path.clone(),
                reason: "the generated seed was rejected".to_owned(),
            })?;
        let public_key_bytes = key_pair.public_key().as_ref();
        let public_blob =
            encode_public_blob(public_key_bytes).map_err(|_| SdkError::Credential {
                operation: "serialize Ed25519 public key".to_owned(),
                path: public_key_path.clone(),
                reason: "the public key encoding is too large".to_owned(),
            })?;
        let public_key = format!(
            "ssh-ed25519 {}\n",
            base64::engine::general_purpose::STANDARD.encode(&public_blob)
        );
        let private_key =
            encode_private_key(&rng, &public_blob, &seed, public_key_bytes).map_err(|_| {
                SdkError::Credential {
                    operation: "serialize Ed25519 private key".to_owned(),
                    path: private_key_path.clone(),
                    reason: "the key encoding or secure random source failed".to_owned(),
                }
            })?;
        let fingerprint = public_key_fingerprint(&public_blob);

        let result = write_new_file(&private_key_path, &private_key, 0o600)
            .and_then(|_| write_new_file(&public_key_path, public_key.as_bytes(), 0o644));
        if let Err(error) = result {
            let _ = fs::remove_file(&private_key_path);
            let _ = fs::remove_file(&public_key_path);
            return Err(error);
        }

        Ok(GeneratedCredential {
            private_key_path,
            public_key_path,
            public_key,
            fingerprint,
            file_mode: "0600".to_owned(),
        })
    }
}

fn encode_public_blob(public_key: &[u8]) -> Result<Vec<u8>, ()> {
    let mut blob = Vec::with_capacity(4 + 11 + 4 + public_key.len());
    append_ssh_string(&mut blob, b"ssh-ed25519")?;
    append_ssh_string(&mut blob, public_key)?;
    Ok(blob)
}

#[derive(Debug)]
enum PrivateKeyEncodingError {
    Random,
    TooLarge,
}

fn encode_private_key(
    rng: &SystemRandom,
    public_blob: &[u8],
    seed: &[u8; 32],
    public_key: &[u8],
) -> Result<Vec<u8>, PrivateKeyEncodingError> {
    let mut check_bytes = [0_u8; 4];
    rng.fill(&mut check_bytes)
        .map_err(|_| PrivateKeyEncodingError::Random)?;
    let check = u32::from_be_bytes(check_bytes);
    let mut private_block = Vec::new();
    private_block.extend_from_slice(&check.to_be_bytes());
    private_block.extend_from_slice(&check.to_be_bytes());
    append_ssh_string(&mut private_block, b"ssh-ed25519")
        .map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut private_block, public_key)
        .map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    let mut private_and_public = Vec::with_capacity(seed.len() + public_key.len());
    private_and_public.extend_from_slice(seed);
    private_and_public.extend_from_slice(public_key);
    append_ssh_string(&mut private_block, &private_and_public)
        .map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut private_block, b"").map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    let padding = 8 - (private_block.len() % 8);
    for value in 1..=padding {
        private_block.push(value as u8);
    }

    let mut encoded = Vec::new();
    append_ssh_string(&mut encoded, b"openssh-key-v1\0")
        .map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut encoded, b"none").map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut encoded, b"none").map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut encoded, b"").map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    encoded.extend_from_slice(&1_u32.to_be_bytes());
    append_ssh_string(&mut encoded, public_blob).map_err(|_| PrivateKeyEncodingError::TooLarge)?;
    append_ssh_string(&mut encoded, &private_block)
        .map_err(|_| PrivateKeyEncodingError::TooLarge)?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(encoded);
    let mut pem = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(70) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    Ok(pem.into_bytes())
}

fn public_key_fingerprint(public_blob: &[u8]) -> String {
    let digest = Sha256::digest(public_blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

fn append_ssh_string(buffer: &mut Vec<u8>, value: &[u8]) -> Result<(), ()> {
    let length = u32::try_from(value.len()).map_err(|_| ())?;
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(value);
    Ok(())
}

fn create_directory(path: &Path) -> Result<(), SdkError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(SdkError::Credential {
                operation: "prepare SSH directory".to_owned(),
                path: path.to_path_buf(),
                reason: "the managed SSH path is not a real directory".to_owned(),
            })
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)
            .map_err(|source| SdkError::filesystem("create SSH directory", path, source)),
        Err(error) => Err(SdkError::filesystem("inspect SSH directory", path, error)),
    }
}

fn reject_existing_managed_file(path: &Path) -> Result<(), SdkError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Err(SdkError::Credential {
            operation: "create per-VM key file".to_owned(),
            path: path.to_path_buf(),
            reason: if metadata.file_type().is_symlink() {
                "refusing to follow an existing symlink".to_owned()
            } else {
                "key file already exists".to_owned()
            },
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SdkError::filesystem("inspect SSH key file", path, source)),
    }
}

fn write_new_file(path: &Path, contents: &[u8], mode: u32) -> Result<(), SdkError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(mode);
    }
    let mut file = options
        .open(path)
        .map_err(|source| SdkError::filesystem("create SSH key file", path, source))?;
    set_mode(path, mode)?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|source| SdkError::filesystem("write SSH key file", path, source))
}

fn set_mode(path: &Path, mode: u32) -> Result<(), SdkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|source| SdkError::filesystem("set SSH key permissions", path, source))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::ports::credentials::CredentialStore;

    use super::Ed25519CredentialStore;

    #[test]
    fn writes_distinct_open_ssh_keys_without_returning_private_contents() {
        let directory = tempdir().expect("temporary directory should exist");
        let credential = Ed25519CredentialStore
            .generate(directory.path())
            .expect("key generation should succeed");

        assert!(credential.private_key_path.exists());
        assert!(credential.public_key_path.exists());
        assert!(credential.public_key.starts_with("ssh-ed25519 "));
        assert!(
            std::fs::read(&credential.private_key_path)
                .expect("private key should be readable")
                .starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert!(!credential.fingerprint.is_empty());
    }
}
