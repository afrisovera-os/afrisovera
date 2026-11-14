//! The kernel's DRBG source files, compiled for the host so the NIST CAVP vectors can be run against them.
//! Nothing is copied: `#[path]` points at kernel/src, so the code under test is the code the kernel builds.
#[path = "../../kernel/src/sha256.rs"]
pub mod sha256;
#[path = "../../kernel/src/drbg.rs"]
pub mod drbg;
#[path = "../../kernel/src/drbg_kat.rs"]
pub mod drbg_kat;
#[path = "../../kernel/src/drbg_apt.rs"]
pub mod drbg_apt;
// F3 step 3: the PC TPM driver's command bytes and response parser (pure; unit tests in tests/tpm_wire.rs).
#[path = "../../kernel/src/pc/tpm_wire.rs"]
pub mod tpm_wire;
// The TIS protocol rules (Startup once, bounded reads, the start-up ceiling), driven by a scripted TPM in
// tests/tpm_tis.rs (architect's TPM merge follow-up 2).
#[path = "../../kernel/src/pc/tpm_tis.rs"]
pub mod tpm_tis;
