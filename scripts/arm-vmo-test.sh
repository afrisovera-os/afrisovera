#!/usr/bin/env bash
# arm-vmo: process memory for EL0 on ARM (AFR-0001 7.6, calls 32 to 34; Dira M3 S5a), the ARM side of pc-vmo.
#
# `vmotest` runs two EL0 programs (kernel/src/process.rs, user_vmo and user_vmox):
#   vmo:  vmo_create(1 MiB) gives handle 0; vmo_map puts it at the map area's start, 0x40_0000_0000 (its own L1
#         slot, above every board's RAM); 0xbeef and 0xcafe read back from its first and last 8 bytes; a map with
#         EXECUTE is E_PERM; vmo_create(33 MiB) is E_NOMEM (the 32 MiB cap); a 4 KiB object maps after a guard page
#         (0x40_0010_1000), unmaps (0), and touching it then faults (a data abort from EL0: the TLB copy was dropped);
#   vmox: a 4 KiB object with a `ret` written in it; branching there faults (an instruction abort: UXN).
# Every successful map prints the G2 walk of the process's tables (architect's S5 verdict, G2 part 2 condition 1):
# every EL0-accessible entry inside its code, stack or map area. Then the frames check: the map area holds no frame
# after both programs, and the reservation is back where it was. `echo alive`: the shell answers.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -2
[ -r "$BIN" ] || { echo "ARM VMO TEST FAILED (harness: $BIN was not built)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4716,server,nowait \
    -qmp tcp:127.0.0.1:4717,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/arm-vmo.out"
python3 scripts/shell-demo.py 4716 4717 "${TMPDIR:-/tmp}/arm-vmo.png" "vmotest" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "@EL0\]|\[vmo\]|EL0 fault|alive|PANIC|panicked" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import re, sys
s = open(sys.argv[1], "rb").read().decode("latin1")
# only what `vmotest` printed: the boot's own EL0 demos also print from slots 3 and 4 (the cloud runner, 15 Nov)
at = s.rfind("vmotest")
s = s[at:] if at >= 0 else s
fail = []
if "KERNEL PANIC" in s or "panicked" in s:
    fail.append("the kernel panicked")
if "alive" not in s:
    fail.append("the shell did not answer after the test")
p3 = re.findall(r"\[P3 @EL0\] (0x[0-9a-f]+)", s)
want3 = ["0x0", "0x4000000000", "0xbeef", "0xcafe", "0xffffffffffffffff", "0xfffffffffffffffa", "0x4000101000", "0x0"]
if p3 != want3:
    fail.append("vmo printed %s, not %s" % (p3, want3))
p4 = re.findall(r"\[P4 @EL0\] (0x[0-9a-f]+)", s)
if p4 != ["0x4000000000"]:
    fail.append("vmox printed %s, not ['0x4000000000']" % p4)
faults = {int(m.group(1)): int(m.group(2), 16) for m in re.finditer(r"EL0 fault in P(\d+): ESR=(0x[0-9a-f]+)", s)}
if faults.get(3, 0) >> 26 != 0x24:
    fail.append("vmo's touch of the unmapped page was not a data abort from EL0 (ESR %s)" % hex(faults.get(3, 0)))
if faults.get(4, 0) >> 26 != 0x20:
    fail.append("vmox's jump into its object was not an instruction abort from EL0 (ESR %s)" % hex(faults.get(4, 0)))
walks = re.findall(r"\[vmo\] walk P\d: (\d+) EL0 entries, (\d+) outside .* \((ok|WRONG)\)", s)
if len(walks) != 3 or any(w[2] != "ok" for w in walks):
    fail.append("the G2 walks were %s, not three ok" % walks)
m = re.search(r"\[vmo\] test: map area frames held (\d+), reserved pages (\d+) before and (\d+) after \((ok|WRONG)\)", s)
if not m or m.group(4) != "ok":
    fail.append("the frames check: %s" % (m.group(0) if m else "missing"))
if fail:
    print("ARM VMO TEST FAILED: " + "; ".join(fail))
    sys.exit(1)
print("ARM VMO TEST PASSED")
PY
