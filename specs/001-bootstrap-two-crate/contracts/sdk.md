# SDK Contract: Bootstrap Example API

## Package

- Package: `taumaru-microvm`
- Target: publishable Rust library
- License: MIT
- Public module: `src/lib.rs`
- Artifact language: English

## Public Function

```rust
pub fn example_message() -> &'static str
```

The function MUST return exactly:

```text
taumaru-microvm SDK is ready
```

The function is a temporary bootstrap demonstration API and may be replaced before the first
stable release.

## Behavioral Contract

- The result is deterministic across repeated calls.
- The function takes no input and does not depend on process state, environment variables,
  network access, filesystem state, registry access, database state, or CLI initialization.
- The function MUST NOT panic, terminate the process, write to stdout or stderr, emit logs or
  tracing events, or mutate global state.
- The function and its purpose MUST be documented with English Rustdoc.

## Consumer Example

A consumer can import the package and call the function directly:

```rust
use taumaru_microvm::example_message;

assert_eq!(example_message(), "taumaru-microvm SDK is ready");
```

This contract is intentionally small and exists only to prove the public SDK boundary. It does
not define the future MicroVM manager API.
