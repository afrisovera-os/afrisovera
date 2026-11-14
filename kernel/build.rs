// Feed our linker script to the linker with an absolute path, so the build
// works no matter which directory `cargo` is invoked from (workspace root, CI,
// or the crate dir). Relative -T paths are fragile across those; this isn't.
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Pick the linker script for the selected board: they differ in load base
    // (QEMU virt loads at 0x4020_0000; the raspi3b/BCM2837 build links at
    // 0x20_0000 for QEMU's raw-Image path; the RPi4/BCM2711 flat-binary target
    // (docs/15-hardware-bringup.md "Suggested order" item 1) links at 0x8_0000,
    // the fixed address the VideoCore firmware loads `kernel8.img` at).
    // Precedence matches board.rs's ACTIVE selection (raspi > rpi4 > pine64 >
    // virt) so a build enabling both `board-raspi` and `board-rpi4` never links
    // BCM2711 code at the Pi 3's boot address.
    let raspi = std::env::var("CARGO_FEATURE_BOARD_RASPI").is_ok();
    let rpi4 = std::env::var("CARGO_FEATURE_BOARD_RPI4").is_ok();
    let name = if raspi {
        "linker-raspi.ld"
    } else if rpi4 {
        "linker-rpi4.ld"
    } else {
        "linker.ld"
    };
    let script = manifest.join(name);
    // Scoped to the ARM kernel binary: `mansa-pc` (the x86-64 UEFI kernel) is linked
    // by the UEFI target's own PE linker and takes no linker script.
    println!("cargo:rustc-link-arg-bin=mansa-kernel=-T{}", script.display());
    // The PC kernel, a repeatable link (docs/design-pc-repro-link.md): /Brepro puts a hash of the output where
    // lld writes a timestamp, and /DEBUG:NONE drops the PDB, whose object names and temp paths change every build
    // and would otherwise move that hash. pc-boot fails if a CodeView (PDB) entry comes back.
    println!("cargo:rustc-link-arg-bin=mansa-pc=/Brepro");
    println!("cargo:rustc-link-arg-bin=mansa-pc=/DEBUG:NONE");
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=linker-raspi.ld");
    println!("cargo:rerun-if-changed=linker-rpi4.ld");
    println!("cargo:rerun-if-changed=build.rs");
    stamp_commit();
}

/// The commit the kernel is built from, for the PC's `report` (pc/report.rs) -- read by
/// `option_env!` there and nowhere in the ARM kernel, so the ARM image does not change with
/// it. AFRI_COMMIT in the environment wins; no git gives "unknown". Re-run when HEAD moves.
fn stamp_commit() {
    use std::process::Command;
    println!("cargo:rerun-if-env-changed=AFRI_COMMIT");
    let git = |args: &[&str]| -> Option<String> {
        let o = Command::new("git").args(args).output().ok()?;
        if !o.status.success() {
            return None;
        }
        let s = String::from_utf8(o.stdout).ok()?.trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    };
    let commit = std::env::var("AFRI_COMMIT").ok().or_else(|| {
        let c = git(&["rev-parse", "--short=8", "HEAD"])?;
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some();
        Some(if dirty { c + "+changes" } else { c })
    });
    if let Some(c) = commit {
        println!("cargo:rustc-env=AFRI_COMMIT={}", c);
    }
    // AFRI_COMMIT_DATE in the environment wins too, so a build with no git (a the cloud runner job) can stamp the same
    // date as a git checkout of that commit, and the two PC images can be compared byte for byte.
    println!("cargo:rerun-if-env-changed=AFRI_COMMIT_DATE");
    let date = std::env::var("AFRI_COMMIT_DATE").ok().or_else(|| git(&["log", "-1", "--format=%cI"]));
    if let Some(d) = date {
        println!("cargo:rustc-env=AFRI_COMMIT_DATE={}", d);
    }
    for p in ["HEAD", "logs/HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", p]) {
            if std::path::Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={}", path);
            }
        }
    }
}
