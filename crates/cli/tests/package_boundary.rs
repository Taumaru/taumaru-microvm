use taumaru_microvm as _;

#[test]
fn cli_depends_on_the_publishable_sdk_package() {
    assert_eq!(env!("CARGO_PKG_NAME"), "taumaru-microvm-cli");
}

#[test]
fn cli_manifest_keeps_registry_storage_integrity_and_runtime_ownership_in_the_sdk() {
    let manifest = include_str!("../Cargo.toml");

    assert!(manifest.contains("taumaru-microvm = { path = \"../sdk\" }"));
    assert!(!manifest.contains("reqwest"));
    assert!(!manifest.contains("rusqlite"));
    assert!(!manifest.contains("sha2"));
    assert!(!manifest.contains("firecracker"));
}
