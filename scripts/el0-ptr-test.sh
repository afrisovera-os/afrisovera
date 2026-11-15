#!/usr/bin/env bash
# Gap G2 (docs/security-model.md): an EL0 program passes KERNEL addresses to SYS_AI (as its reply buffer) and to
# SYS_SPEAK (as its text). The kernel must refuse both: the canary in kernel memory stays "G2-CANARY-INTACT" and
# its bytes are never spoken. A kernel that follows user pointers unchecked overwrites the canary.
# G2 part 2: the copies go through kernel/src/uaccess.rs (LDTRB/STTRB, a fault there refuses the call), and before the
# program runs the kernel walks its tables: every leaf EL0 can reach must lie in its user blocks (the [g2] line).
# Then WIN_CREATE, WIN_FILL and WIN_TEXT get u64::MAX coordinates and sizes: no panic, and the program prints 0x5afe.
#   EXPECT=hole scripts/el0-ptr-test.sh   grades a BEFORE kernel: passes only if the write lands.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
EXPECT="${EXPECT:-fixed}"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4706,server,nowait \
    -qmp tcp:127.0.0.1:4707,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/el0-ptr.out"
python3 scripts/shell-demo.py 4706 4707 "${TMPDIR:-/tmp}/el0-ptr.png" "el0-ptr" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "el0-ptr|@EL0|G2-CANARY|\[g2\]|alive|PANIC" "$OUT" | sed 's/^/  /'

python3 - "$OUT" "$EXPECT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
expect = sys.argv[2]
fail = []
if "KERNEL PANIC" in s:
    fail.append("the kernel panicked")
if "alive" not in s:
    fail.append("the shell did not answer after the probe")
if expect == "hole":
    if "G2-CANARY OVERWRITTEN" not in s:
        fail.append("BEFORE kernel: the canary was not overwritten (expected the unchecked write to land)")
else:
    for want in ["AI refused: a buffer is outside the process's memory",
                 "SPEAK refused: the text is outside the process's memory",
                 "G2-CANARY intact",
                 "@EL0] 0x5afe"]:
        if want not in s:
            fail.append("missing: " + want)
    if "G2-CANARY OVERWRITTEN" in s or "speaking: \"G2-CANARY" in s:
        fail.append("kernel memory was written or spoken through an EL0 pointer")
    # G2 part 2, condition 1: the process's own tables let EL0 reach nothing but its user blocks
    import re
    m = re.search(r"\[g2\] P3 tables: (\d+) EL0-reachable leaf entr(?:y|ies), (\d+) outside its user blocks", s)
    if not m:
        fail.append("missing: the [g2] table walk line")
    elif int(m.group(1)) == 0 or int(m.group(2)) != 0:
        fail.append("the table walk: %s EL0-reachable, %s outside the user blocks" % (m.group(1), m.group(2)))
if fail:
    print("EL0-PTR TEST FAILED (%s): %s" % (expect, "; ".join(fail)))
    sys.exit(1)
print("EL0-PTR TEST PASSED (%s)" % expect)
PY
