use std::panic::catch_unwind;

use taumaru_microvm::example_message;

#[test]
fn example_message_is_deterministic_and_panic_free() {
    let first = catch_unwind(example_message).ok();
    let second = catch_unwind(example_message).ok();

    assert_eq!(first, Some("taumaru-microvm SDK is ready"));
    assert_eq!(second, Some("taumaru-microvm SDK is ready"));
    assert_eq!(first, second);
}
