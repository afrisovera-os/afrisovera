//! daraja-linux: Daraja's Linux bridge (stage 0). See Cargo.toml for the sources it is written from.
//!
//! The parts here need no kernel and are tested on the host:
//! - [`bundle`]: the bundle's 48-byte header, checked before anything else (DARAJA02);
//! - [`calls`]: the Linux calls of stage 0 over a `Host` (the kernel, or a fake one in the tests);
//! - [`cpio`]: the bundle (the program and its files, read-only), a cpio "newc" archive;
//! - [`stack`]: the stack a Linux program finds at its entry point (argc, argv, envp, the auxiliary vector).

#![no_std]
#![forbid(unsafe_code)]

pub mod bundle;
pub mod calls;
pub mod cpio;
pub mod stack;
