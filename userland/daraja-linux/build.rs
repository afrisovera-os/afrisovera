use std::path::PathBuf;

// The bridge program is linked at the PC's user code address (linker-x86_64.ld, as rustapp), but only when it is
// built for the bare-metal target: the library's host tests link as an ordinary host program.
fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg-bins=-T{}", dir.join("linker-x86_64.ld").display());
    }
    println!("cargo:rerun-if-changed=linker-x86_64.ld");
    println!("cargo:rerun-if-changed=build.rs");
}
