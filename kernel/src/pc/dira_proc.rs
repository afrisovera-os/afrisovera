//! Dira on the PC (Dira M3 S4, DIRA_S4_PLAN.md, the architect's verdict of 14 Nov): the `dira` command starts
//! userland/dira as a ring-3 process. The kernel no longer parses any web byte on the PC: kernel/src/dira is built
//! for ARM only (until S5), and this file is the PC's `crate::dira`, so the shared shell's `dira` line is unchanged.
//!
//!   dira <address>        the page in a desk window (on the console when the desk is not showing)
//!   dira text <address>   the page on the console
//!   dira file <name>      an .html or .txt file from the file store
//!
//! What the kernel gives Dira, and nothing else (AFR-0001 7.6's spawn table): a CELL handle titled "Dira (built-in)"
//! (not in text mode), a NET handle for classes 0 to 2 (bit 3, this machine, only under the test rig's flag), the
//! address as typed (at most 512 bytes; the kernel never reads it as an address), and for `dira file` the store
//! file's bytes, mapped read-only. The binary is userland/dira built by scripts/build-dira.sh; dira_bin.rs (written
//! by that script) holds its SHA-256, the source tree it was built from and the toolchain, and the kernel refuses to
//! start bytes whose hash differs.

include!("dira_bin.rs");

/// The program itself (scripts/build-dira.sh: never edited by hand; the dira-bin suite rebuilds it and compares).
static DIRA_BIN: &[u8] = include_bytes!("dira-x86_64.bin");

/// The name Dira's hash is recorded under in the integrity manifest.
pub const NAME: &str = "dira (built-in)";

const USAGE: &str = "dira: usage: dira <address>   (a page in a desk window, e.g. dira http://10.0.2.2:8000/)\n      dira text <address>   (the page on the console)\n      dira file <name>      (an .html or .txt file from the store)";

/// The boot's integrity record (pc/main.rs, beside the bundled apps): the hash of the compiled-in bytes, and a line
/// if they are not the bytes dira_bin.rs names.
pub fn record() {
    crate::integrity::record(NAME, DIRA_BIN);
    crate::kprintln!(
        "[dira] built-in Dira: {} bytes, source sha256 {}..., {}",
        DIRA_BYTES,
        &DIRA_SOURCE[..16.min(DIRA_SOURCE.len())],
        DIRA_TOOLCHAIN
    );
    if !intact() || DIRA_BIN.len() != DIRA_BYTES {
        crate::kprintln!("[dira] the built-in Dira does not match its recorded hash: it will not be started");
    }
}

fn intact() -> bool {
    crate::sha256::digest(DIRA_BIN) == DIRA_SHA256
}

fn heap_line(when: &str) {
    crate::kprintln!(
        "dira: heap {}: {} bytes in use, high-water {} bytes",
        when,
        crate::heap::used(),
        crate::heap::high_water()
    );
}

/// The shell command.
pub fn run(args: &str) {
    let args = args.trim();
    let (text, rest) = match args.split_once(' ') {
        Some(("text", r)) => (true, r.trim()),
        _ if args == "text" => (true, ""),
        _ => (false, args),
    };
    if rest.is_empty() || rest == "file" {
        return crate::kprintln!("{}", USAGE);
    }
    // the argument Dira gets: the words as typed, with `file NAME` written as `file:NAME`
    let mut arg = [0u8; 512];
    let mut n = 0;
    let mut add = |b: &[u8]| -> bool {
        if n + b.len() > arg.len() {
            return false;
        }
        arg[n..n + b.len()].copy_from_slice(b);
        n += b.len();
        true
    };
    let fits = match rest.split_once(' ') {
        Some(("file", name)) => add(b"file:") && add(name.trim().as_bytes()),
        _ => add(rest.as_bytes()),
    };
    if !fits {
        return crate::kprintln!("dira: the address is longer than 512 bytes, so Dira was not started (nothing was cut)");
    }
    let arg = &arg[..n];
    // `dira file`: the store file, if there is one by that name (Dira says so itself when there is not)
    let file = arg.strip_prefix(b"file:").and_then(|name| core::str::from_utf8(name).ok()).and_then(|name| crate::vfs::read_full(name.trim()));
    if !intact() {
        return crate::kprintln!("dira: the built-in Dira does not match its recorded hash, so it was not started");
    }
    let window = !text && crate::desk::shown();
    if !text && !window {
        crate::kprintln!("dira: the desk is not showing, so the page comes here (open the desk with: desk)");
    }
    if !window {
        heap_line("before");
    }
    let frames0 = crate::frame::used_frames();
    if crate::proc::spawn_dira(DIRA_BIN, window, arg, file).is_none() {
        return crate::kprintln!("dira: Dira could not be started (no process slot or memory)");
    }
    let _ = crate::proc::run_all();
    if crate::qemu::fw_cfg_file(b"opt/afrisovera/test-grants") {
        // suite pc-dira-proc: everything Dira took (code, stack, its memory object, the file) came back
        let frames1 = crate::frame::used_frames();
        let ok = if frames0 == frames1 { "ok" } else { "WRONG" };
        crate::kprintln!("[dira] frames in use {} before and {} after ({})", frames0, frames1, ok);
    }
    if !window {
        heap_line("after");
    }
}
