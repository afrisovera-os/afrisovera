//! The kernel's DRBG source files, compiled for the host so the NIST CAVP vectors can be run against them.
//! Nothing is copied: `#[path]` points at kernel/src, so the code under test is the code the kernel builds.
#[path = "../../kernel/src/sha256.rs"]
pub mod sha256;
#[path = "../../kernel/src/drbg.rs"]
pub mod drbg;
#[path = "../../kernel/src/drbg_kat.rs"]
pub mod drbg_kat;
