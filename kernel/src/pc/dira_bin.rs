// Written by scripts/build-dira.sh: do not edit. Suite dira-bin rebuilds the program and compares.
/// SHA-256 of dira-x86_64.bin; the kernel starts no other bytes.
pub const DIRA_SHA256: [u8; 32] = [0x98,0x29,0x75,0xe4,0x4f,0xd3,0x22,0xf0,0x58,0x27,0x96,0xdb,0x79,0x6f,0xc4,0x15,0xc8,0x4d,0x0d,0x6a,0x68,0x39,0xb0,0x96,0xae,0x6e,0x68,0xba,0xf3,0x1a,0x33,0x50,];
/// The program's size in bytes.
pub const DIRA_BYTES: usize = 63828;
/// SHA-256 of the source it was built from (userland/dira and the three parsers in kernel/src/dira).
pub const DIRA_SOURCE: &str = "9481fa78df53f582d1cbe3e9833a21a5127060711839cbeb822668ba5457dc8c";
/// The toolchain that built it.
pub const DIRA_TOOLCHAIN: &str = "rustc 1.97.1 (8bab26f4f 2026-07-14)";
