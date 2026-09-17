use taumaru_microvm as _;

#[test]
fn cli_depends_on_the_publishable_sdk_package() {
    assert_eq!(env!("CARGO_PKG_NAME"), "taumaru-microvm-cli");
}
