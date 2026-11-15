//! The Linux bridge as a program: built only for the bare-metal target (`x86_64-unknown-none`), linked at the PC's
//! USER_BASE by linker-x86_64.ld, turned into a flat binary in the cloud and spawned by the kernel's spawn_pair. On
//! any other target (the host, for `cargo test` of the library) it is an empty program, so the library's tests build.
#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
mod bridge;
#[cfg(target_os = "none")]
mod sys;

#[cfg(not(target_os = "none"))]
fn main() {}
