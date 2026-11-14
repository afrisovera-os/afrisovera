#!/usr/bin/env bash
# Verify that an allocation the arena refuses is ATTRIBUTED, and that the boot path no
# longer asks the arena for anything (milestone 2.736) — the arena's small customers.
#
# 2.734 named the heap's bound; 2.735 routed its largest customer (the 16 KiB kernel
# stack) through a door that hands that bound back to a caller, so a full arena costs one
# demo instead of the machine. What that left is everything else the kernel allocates:
# every `Vec`, `String` and `format!` goes through the INFALLIBLE `alloc` API, which has
# no way to return a bound. A null there goes to Rust's own allocation error handler and
# out as `panic!("memory allocation of N bytes failed")` — a line that names a byte count
# and nothing else. Not which allocator refused. Not how full it was. Not whether the
# machine was one allocation over the line or nowhere near it.
#
# `#[alloc_error_handler]` would intercept it earlier, but it is unstable and
# rust-toolchain.toml pins stable, so the two moves available are the ones checked here:
#   * the `#[panic_handler]` reads the message before printing it, recognises the
#     allocation failure, and states 2.734's bound plus the live arena beside it —
#     used/free/padding and the room left in 16 KiB stacks;
#   * `config::persist` — which runs on EVERY disk boot, before there is a shell — stops
#     building its text with `format!`. It renders into a fixed buffer and refuses BY
#     NAME, so a full arena costs the config write rather than the boot. `vfs::write`'s
#     return value is checked for the same reason.
#
# Neither path can be reached on a live machine (the kernel puts ~80 KiB into a mebibyte
# and renders ~90 bytes into 512), which is exactly why they had never run. So the proof
# is the same shape as `heap test`: pure functions — `heap::alloc_failure_size` and
# `config::render` — driven against synthetic inputs, plus source checks for the parts a
# shell cannot observe.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "ALLOC BOUND TEST FAILED (build)"; exit 1; }

# A disk and a NIC, because the path this milestone changed is the BOOT path: without a
# virtio-blk device `config::boot_apply` returns before it ever renders, and the run would
# prove the new code correct in a self-test while never once executing it for real. With
# the disk attached the kernel renders and writes afrisovera.conf through the fixed buffer
# on its way up, and `config` below reads back what that write actually stored.
IMG="${TMPDIR:-/tmp}/afrisovera-allocbound.img"
rm -f "$IMG"
head -c 4194304 /dev/zero > "$IMG" # fresh 4 MiB blank disk

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:4870,server,nowait \
    -qmp tcp:127.0.0.1:4871,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/allocbound.out"
python3 scripts/shell-demo.py 4870 4871 "${TMPDIR:-/tmp}/allocbound.png" \
    "echo warmup" "config test" "config" "config save" "heap test" "heap" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "render checks held|bump checks held|\[ok\]   (the|a|an|EVERY)|\[FAIL\]|on an alloc panic|NOT saved|\[cfg\]" \
    "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/config.rs kernel/src/fmtbuf.rs kernel/src/heap.rs kernel/src/main.rs <<'PY'
import re, sys
s    = open(sys.argv[1], "rb").read().decode("latin1")
cfg  = open(sys.argv[2], encoding="utf-8").read()
fmtb = open(sys.argv[3], encoding="utf-8").read()
heap = open(sys.argv[4], encoding="utf-8").read()
main = open(sys.argv[5], encoding="utf-8").read()

def fail(msg):
    print(f"ALLOC BOUND TEST FAILED ({msg})"); sys.exit(1)

def dash(t):
    """Every way an em dash can reach this file, folded to one ASCII marker.

    The sibling suites give up on em-dashed case names and compare only the prefix
    before the dash; here the REASON is what follows it ("... a wrong name is worse
    than none"), so the prefix is the half not worth checking. Guessing a decoder does
    not work either: the capture is whatever `shell-demo.py`'s stdout encoding gave it,
    which is cp1252 on this host (a lone 0x97) and UTF-8 elsewhere. Reading the capture
    byte-preserving and folding BOTH sides is the version that does not depend on the
    machine the test runs on.

    The needles are built from code points rather than typed, so re-saving this file in
    another encoding cannot quietly turn one of them into the thing it was meant to
    find."""
    for needle in (chr(0x2014),                          # a real em dash: the phrases here
                   chr(0x97),                            # cp1252, through the latin1 read
                   chr(0xE2) + chr(0x80) + chr(0x94)):   # UTF-8, through the same
        t = t.replace(needle, "-")
    return t

def code(src):
    """The source minus its line comments — 2.735's helper, for 2.735's reason: this
    milestone's doc-comments quote the very `format!` and panic message they replaced,
    and a grep that cannot tell a citation from a call would force the history to go
    unwritten to satisfy it."""
    return "\n".join(ln for ln in src.splitlines() if not ln.lstrip().startswith("//"))

def nostr(src):
    """The source minus its string literals, for the checks that look for a BANNED CALL.
    Same problem as `code()`, one layer in: this milestone's own test descriptions name
    `format!` in prose ("byte-identical to what format! built before it"), and a bare
    substring search cannot tell that from an actual call. Not used for the checks that
    look for a printed message — those live in string literals on purpose."""
    return re.sub(r'"(?:[^"\\]|\\.)*"', '""', src)

cfg, fmtb, heap, main = (code(x) for x in (cfg, fmtb, heap, main))

# --- part 1: the boot path no longer allocates its text ------------------------------
# The milestone's first half. `persist` runs on every disk boot; a `format!` there is an
# infallible allocation on the one path with nobody at a console to read the panic.
if "format!" in nostr(cfg) or "use alloc::" in nostr(cfg):
    fail("config.rs still builds its text with format! — the boot path is back to dying "
         "of a full arena for a 90-byte string")
# (13 Nov 2026: persist also takes the desk's language, LANGUAGES_PLAN.md; the bound is unchanged)
if "fn persist(boots: u32, lang: &str) -> Result<(), &'static str>" not in cfg:
    fail("config::persist does not return its bound — a refusal nobody is handed is a "
         "config that silently was not written")
if not re.search(r"if !crate::vfs::write\(CONF, conf\.as_bytes\(\)\)", cfg):
    fail("config::persist drops vfs::write's return value, so a full file table loses "
         "the config without saying so")
# Both callers absorb it rather than unwrapping or ignoring it.
for caller in ("boot_apply", "save"):
    body = cfg.split(f"pub fn {caller}()")[-1].split("\npub fn ")[0]
    if "Err(why)" not in body or "NOT saved" not in body:
        fail(f"config::{caller} does not report the refusal it was handed")
if re.search(r"^\s*persist\([^)]*\);\s*$", cfg, re.M):
    fail("a config::persist call site drops the Result on the floor")
# The render itself is pure and allocation-free.
if "pub fn render<'a>(" not in cfg:
    fail("config::render is not a pure function, so its boundary cannot be tested")
if "const CONF_MAX: usize = crate::vfs::MAX_FILE_BYTES" not in cfg:
    fail("the render buffer is not tied to the VFS file limit — one of the two bounds "
         "would then truncate silently past the other")
for bad in ("alloc::", "Vec<", "String"):
    if bad in nostr(fmtb):
        fail(f"fmtbuf.rs uses {bad} — the fallible replacement for format! must not "
             f"itself need the allocator")

# --- part 2: the panic names the bound and states the arena --------------------------
ph = main.split("#[panic_handler]")[-1]
if "fmtbuf::render_lossy" not in ph:
    fail("the panic handler prints the message straight out, so it never reads it — an "
         "allocation failure cannot be recognised, only echoed")
if "heap::alloc_failure_size" not in ph or "heap::report_exhaustion" not in ph:
    fail("the panic handler does not consult the heap — 2.734's last_bound() and 2.735's "
         "stacks_left() sit one call away, and an allocation panic still names only a "
         "byte count")
if "format!" in nostr(ph) or "alloc::" in nostr(ph):
    fail("the panic handler allocates — on a full arena that is a second panic inside "
         "the first")
if "pub fn alloc_failure_size(text: &str) -> Option<usize>" not in heap:
    fail("heap::alloc_failure_size is not a pure classifier, so it cannot be tested "
         "against a message the live kernel never produces")
rep = heap.split("pub fn report_exhaustion")[-1].split("\npub fn ")[0]
for needed in ("last_bound()", "stacks_left()", "used()", "fails()"):
    if needed not in rep:
        fail(f"report_exhaustion does not state {needed} — the operator still cannot "
             f"tell a machine one allocation over the line from one nowhere near it")

# --- part 3: the render's boundary, against buffers the kernel never gives it --------
cases = s.split("config: allocation-free render against synthetic buffers")[-1]
for phrase in (
        "the rendered config is byte-identical to what format! built before it",
        "a buffer of exactly the rendered length is served, to the last byte",
        "one byte short is REFUSED by name — never written short, never half a key=value",
        "EVERY buffer below the fit refuses; none reports a partial render as a success",
        # The claim that keeps the refusal theoretical rather than a new failure mode.
        "the WIDEST config the kernel can produce (255.255.255.255 x3, u32::MAX boots, an 8-letter language) fits",
        "the buffer is no larger than the VFS file limit, so nothing rendered is truncated",
        "a lossy render stops on a CHAR boundary, keeping the prefix and counting the rest",
        "a zero-length buffer writes nothing and overruns nothing, rather than faulting",
        "text that fits is rendered whole, with nothing reported dropped"):
    if dash(f"[ok]   {phrase}") not in dash(cases):
        fail(f"render not proved: {phrase}")
m = re.search(r"config: (\d+)/(\d+) render checks held \((\d+) B buffer, (\d+) B rendered, "
              r"(\d+) B widest possible\)", cases)
if not m:
    fail("`config test` prints no tally")
if m.group(1) != m.group(2):
    fail(f"only {m.group(1)} of {m.group(2)} render checks held")
rchecks, cbuf, rendered, widest = (int(m.group(i)) for i in (2, 3, 4, 5))
if not 0 < rendered <= widest <= cbuf:
    fail(f"the rendered ({rendered} B) / widest ({widest} B) / buffer ({cbuf} B) figures "
         f"are not ordered — the headroom claim is not what was measured")

# --- part 4: the classifier, against messages the live kernel never prints -----------
bump = s.split("kernel heap: bump self-test against synthetic requests")[-1]
for phrase in (
        "the stock allocation-failure message is recognised, alone or inside a panic line",
        "an ordinary panic is NOT attributed to the heap — a wrong name is worse than none",
        "a malformed count refuses rather than guessing, and a zero-byte one still parses"):
    if dash(f"[ok]   {phrase}") not in dash(bump):
        fail(f"attribution not proved: {phrase}")
if "[FAIL]" in s:
    fail("a check did not hold")
m = re.search(r"kernel heap: (\d+)/(\d+) bump checks held", bump)
if not m or m.group(1) != m.group(2):
    fail("the heap's bump checks did not all hold")
bchecks = int(m.group(2))

# --- part 5: the live kernel, which must be untouched by any of it -------------------
# The whole point is that a healthy boot never sees these paths. If the config stopped
# being written, or a refusal fired on an ordinary boot, the fix broke what it protected.
if "[cfg] NOT saved" in s:
    fail("the config was refused on an ordinary boot — the fixed buffer is too small for "
         "the state this kernel actually produces")
if not re.search(r"^  hostname=afrisovera", s, re.M):
    fail("`config` shows no hostname — the allocation-free render did not reach the file")
if not re.search(r"^  boots=\d+", s, re.M):
    fail("`config` shows no boot count")
if not re.search(r"^\[cfg\] saved to afrisovera\.conf", s, re.M):
    fail("`config save` did not report a save")
if not re.search(r"^  on an alloc panic  the handler recognises it, names this bound", s, re.M):
    fail("`heap` does not state what now happens when an allocation panics — a promise "
         "the operator cannot read before it fires is 2.732's complaint again")
if "KERNEL PANIC" in s:
    fail("the kernel panicked on an ordinary boot")

print(f"ALLOC BOUND TEST PASSED  (the arena's SMALL customers now fail the way its "
      f"largest one does: the #[panic_handler] renders the message into a fixed stack "
      f"buffer instead of printing it blind, recognises Rust's own "
      f"'memory allocation of N bytes failed' — which names a byte count and nothing "
      f"else — and answers it with 2.734's named bound plus the live arena beside it, so "
      f"an operator reads which bound refused, how full the arena was, and how many "
      f"16 KiB stacks were left rather than a bare N; the classifier is pure and refuses "
      f"to misattribute an ordinary panic; and config::persist, which runs on EVERY disk "
      f"boot before there is a shell, no longer builds its text with format! — it renders "
      f"into a {cbuf} B fixed buffer ({rendered} B used, {widest} B for the widest config "
      f"this kernel can ever produce, so the new bound cannot fire in production), refuses "
      f"BY NAME instead of writing a truncated config that would parse and lie, checks "
      f"vfs::write's return value, and both callers report the refusal and carry on — a "
      f"full arena now costs the config write, not the boot; {rchecks}/{rchecks} render "
      f"checks and {bchecks}/{bchecks} bump checks held)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "ALLOC BOUND TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
