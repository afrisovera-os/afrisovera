#!/usr/bin/env bash
# Gap G2 (docs/security-model.md): an EL0 program passes KERNEL addresses to SYS_AI (as its reply buffer) and to
# SYS_SPEAK (as its text). The kernel must refuse both: the canary in kernel memory stays "G2-CANARY-INTACT" and
# its bytes are never spoken. A kernel that follows user pointers unchecked overwrites the canary.
# G2 part 2: the copies go through kernel/src/uaccess.rs (LDTRB/STTRB, a fault there refuses the call), and before the
# program runs the kernel walks its tables: every leaf EL0 can reach must lie in its user blocks (the [g2] line).
# `rtc` then spawns the userspace RTC driver: its tables must give EL0 exactly one 4 KiB device page (the PL031's),
# and it must still read the clock and serve it to its client.
# Then WIN_CREATE, WIN_FILL and WIN_TEXT get u64::MAX coordinates and sizes: no panic, and the program prints 0x5afe.
#   EXPECT=hole scripts/el0-ptr-test.sh   grades a BEFORE kernel: passes only if the write lands.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
EXPECT="${EXPECT:-fixed}"
# EL0PTR_CPU=max (suite el0-ptr-max): the same run on a CPU with PAN (architect, 22 Nov 2026), so the ordinary EL0
# paths meet PAN on every push; it also runs `pan` and requires PAN on. The default stays the Raspberry Pi 4's CPU.
EL0PTR_CPU="${EL0PTR_CPU:-cortex-a72}"
EXTRA=()
[ "$EL0PTR_CPU" = max ] && EXTRA=("pan")

qemu-system-aarch64 -M virt -cpu "$EL0PTR_CPU" -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4706,server,nowait \
    -qmp tcp:127.0.0.1:4707,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/el0-ptr.out"
python3 scripts/shell-demo.py 4706 4707 "${TMPDIR:-/tmp}/el0-ptr.png" "${EXTRA[@]}" "el0-ptr" "rtc" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "el0-ptr|@EL0|G2-CANARY|\[g2\]|\[rtc\]|EL0 fault|alive|PANIC" "$OUT" | sed 's/^/  /'

python3 - "$OUT" "$EXPECT" "$EL0PTR_CPU" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
expect = sys.argv[2]
cpu = sys.argv[3] if len(sys.argv) > 3 else "cortex-a72"
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
    # G2 (architect, 16 Nov 2026): a driver gets exactly its one 4 KiB device page (the PL031's), not the 2 MiB region
    # that also holds the UART and fw_cfg's DMA interface; and the driver still reads the clock through it
    d = re.search(r"\[g2\] P3 \(driver\) tables: (\d+) EL0-reachable leaf entries, (\d+) device page\(s\) of 4 KiB \(PA 0x9010000\), (\d+) outside", s)
    if not d:
        fail.append("missing: the driver's [g2] table walk line")
    elif int(d.group(2)) != 1 or int(d.group(3)) != 0:
        fail.append("the driver's walk: %s device page(s), %s outside (wanted exactly 1 and 0)" % (d.group(2), d.group(3)))
    if "EL0 fault in P3" in s:
        fail.append("the RTC driver faulted reading its device page")
    t = re.findall(r"\[P[34] @EL0\] (0x[0-9a-f]+)", s)
    if len(t) < 2 or t[-1] != t[-2] or int(t[-1], 16) < 0x60000000:
        fail.append("the RTC driver did not serve a plausible time to its client: %s" % t[-2:])
    if "[rtc] done" not in s:
        fail.append("missing: [rtc] done")
if cpu == "max":
    if "pan: on" not in s:
        fail.append("-cpu max: PAN was not on")
    if "PAN stopped" in s:
        fail.append("-cpu max: PAN stopped a kernel access to EL0 memory (a path outside kernel/src/uaccess.rs)")
if fail:
    print("EL0-PTR TEST FAILED (%s): %s" % (expect, "; ".join(fail)))
    sys.exit(1)
print("EL0-PTR TEST PASSED (%s)" % expect)
PY
