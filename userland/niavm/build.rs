use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // aarch64: linker.ld (0x8000_0000); x86_64: linker-x86_64.ld (the PC's USER_BASE)
    let script = if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64") {
        "linker-x86_64.ld"
    } else {
        "linker.ld"
    };
    println!("cargo:rustc-link-arg=-T{}", dir.join(script).display());
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=linker-x86_64.ld");
    println!("cargo:rerun-if-changed=build.rs");
}
