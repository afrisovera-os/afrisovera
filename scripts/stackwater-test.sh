#!/usr/bin/env bash
# stackwater: how deep the two boot stacks really go (F7, step 1: measure before building a guard).
#
# 6e00ee98 raised the boot stack from 512 KiB to 1 MiB after the debug shell overran it (wasmops and
# Asili voice ran into the top of .bss). Nobody has measured the real depth. The kernel now paints both
# stacks at boot (kernel/src/stackwater.rs); `stack` reports the lowest word that is no longer paint.
#
# Boot with two cores (so core 1 runs on its own stack), run the commands known to be deepest, then:
#   - `stack` must report both stacks, neither OVERRUN, each under 80% of its size (an early warning, the
#     architect's line: past it, shrink shell::exec before anything else);
#   - no stack frame WITHOUT a stack probe may exceed 32 KiB (half the 64 KiB guard), or a frame could step
#     over the guard without touching it (checked when llvm-objdump is in the toolchain, as on Actions);
#   - the numbers are printed so the guard can be sized from them, together with the largest single
#     stack frame in the binary (from llvm-objdump when the toolchain has it): a guard smaller than one
#     frame can be stepped over without touching it.
# The paint is a LOWER bound (a reserved but never-written frame does not show), so the 80% limit is
# deliberately loose; the margin the guard needs comes from the largest-frame figure, not from this.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "STACKWATER TEST FAILED (build)"; exit 1; }

T="${TMPDIR:-/tmp}"
OUT="$T/stackwater.out"
FRAMES="$T/stackwater.frames"
rm -f "$FRAMES"
. "$(dirname "$0")/lib/afrish-boot.sh"

SND_ARGS=()
CMDS=(wasmops "nia sum.wasm" "ask 20% of 150" "mesh selftest" stack)
if qemu-system-aarch64 -device help 2>/dev/null | grep -q "virtio-sound-device"; then
    SND_ARGS=(-audiodev none,id=snd0 -device virtio-sound-device,audiodev=snd0)
    # Voice last but one: it was one of the two overruns; the second `stack` is best effort (smoke notes
    # that `say` can block on the sound control queue after printing).
    CMDS+=("say HELLO" stack)
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -device virtio-rng-device "${SND_ARGS[@]}" \
    -serial tcp:127.0.0.1:4412,server,nowait \
    -qmp tcp:127.0.0.1:4413,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 4412 4413 "$T/stackwater.png" "${CMDS[@]}" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[stack\]|selftest [0-9]+/[0-9]+|print\(32\)|asili voice\] synth" "$OUT" | sed 's/^/  /'

# Largest stack frames in the image, per function (an estimate, printed for sizing, not graded):
#   - small frames: `sub sp, sp, #imm[, lsl #12]` in the prologue;
#   - large frames: LLVM computes the new SP in x9 (`sub x9, sp, #a, lsl #12` then `sub x9, x9, #b`) and
#     walks down to it one 4 KiB page at a time (`sub sp, sp, #1, lsl #12` + `str xzr, [sp]`): an inline
#     STACK PROBE. Such a frame touches every page it reserves, so a 4 KiB guard cannot be stepped over.
OBJDUMP="$(ls "$(rustc --print sysroot 2>/dev/null)"/lib/rustlib/*/bin/llvm-objdump 2>/dev/null | head -1)"
if [ -n "$OBJDUMP" ] && [ -x "$OBJDUMP" ]; then
    "$OBJDUMP" -d --no-show-raw-insn "$BIN" 2>/dev/null | python3 -c '
import re, sys
fn, tot, probed = None, {}, set()
num = r"#(0x[0-9a-f]+|\d+)(,\s*lsl\s*#12)?"
sp_sub = re.compile(r"\bsub\s+sp,\s*sp,\s*" + num)
x9_sub = re.compile(r"\bsub\s+x9,\s*(sp|x9),\s*" + num)
def val(m, g):
    v = int(m.group(g), 0)
    return v << 12 if m.group(g + 1) else v
for line in sys.stdin:
    m = re.match(r"^[0-9a-f]+ <(.+)>:$", line.strip())
    if m:
        fn = m.group(1); continue
    if not fn:
        continue
    x = x9_sub.search(line)
    if x:
        tot[fn] = tot.get(fn, 0) + val(x, 2); probed.add(fn); continue
    s = sp_sub.search(line)
    if s:
        v = val(s, 1)
        if fn in probed and v == 4096:
            continue  # the probe loop step, already counted through x9
        tot[fn] = tot.get(fn, 0) + v
big = [(n, v) for n, v in tot.items() if n not in probed and v > 32768]
for name, v in big:
    print("  UNPROBED-OVER-32K %d B %s" % (v, name[:90]))
top = sorted(tot.items(), key=lambda x: -x[1])[:6]
for name, v in top:
    print("  FRAME %8d B %s %s" % (v, "probed" if name in probed else "      ", name[:90]))
print("  FRAME functions with an inline stack probe: %d" % len(probed))
' | tee "$FRAMES"
else
    echo "  FRAME (llvm-objdump not in this toolchain; largest-frame estimate skipped)"
fi

python3 - "$OUT" "$FRAMES" <<'PY'
import re, sys
t = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
bad = []
rows = re.findall(r"\[stack\] (boot|secondary) 0x[0-9a-f]+\.\.0x[0-9a-f]+ size=(\d+) KiB used=(\d+) B \((\d+)%\)", t)
first = {}
for name, size, used, pct in rows:
    first.setdefault(name, (int(size), int(used), int(pct)))
for name in ("boot", "secondary"):
    if name not in first:
        bad.append("no `stack` line for the %s stack" % name)
        continue
    size, used, pct = first[name]
    if used <= 0:
        bad.append("%s stack shows no use at all (paint never written, or never scanned)" % name)
    if pct >= 80:
        bad.append("%s stack %d%% used (%d B of %d KiB)" % (name, pct, used, size))
try:
    fr = open(sys.argv[2], "rb").read().decode("utf-8", "replace")
except OSError:
    fr = ""
for line in fr.splitlines():
    if "UNPROBED-OVER-32K" in line:
        bad.append("a stack frame with no stack probe is over 32 KiB:" + line.split("UNPROBED-OVER-32K", 1)[1])
if "OVERRUN" in t:
    bad.append("a stack reported OVERRUN (no paint left at its bottom)")
if re.search(r"KERNEL PANIC|SYNC EXCEPTION|unhandled exception", t):
    bad.append("kernel fault during the run")
if not re.search(r"print\(32\)", t):
    bad.append("wasmops did not finish (it is one of the commands that overran 512 KiB)")
deep = max((int(u) for n, s, u, p in rows if n == "boot"), default=0)
if bad:
    print("STACKWATER TEST FAILED")
    for b in bad:
        print("  - " + b)
    sys.exit(1)
print("STACKWATER TEST PASSED  (boot deepest %d B of %d KiB, secondary %d B of %d KiB)"
      % (deep, first["boot"][0], first["secondary"][1], first["secondary"][0]))
PY
