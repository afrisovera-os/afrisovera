#!/usr/bin/env bash
# Verify `board machine` (milestone 2.728) — the question underneath docs/15 item 4.
#
# 2.725 taught the kernel to DIFF its compile-time map against a device tree and
# 2.726/2.727 to let a live tree OVERRIDE it. Both assumed, without ever checking,
# that the tree is about the machine we compiled for. It need not be: a `board-rpi4`
# kernel handed a virt tree would diff five rows of cross-machine noise and, on a real
# live blob, rebase MMIO onto another SoC. This milestone makes each map state the
# root `compatible` names its machine answers to and reports them against the tree's.
#
# Checked from sources that are NOT board.rs:
#   * the names themselves come from the blueprint table in docs/15-hardware-bringup.md;
#   * the virt name is *also* confirmed against the bundled fixture kernel/src/virt.dtb,
#     i.e. against bytes QEMU produced, so "the tree says linux,dummy-virt" is the tree
#     talking and not the kernel quoting itself;
#   * the wrong-machine verdict — unreachable on a QEMU virt boot, and the whole point
#     of the milestone — is exercised by `board machine test` against synthetic root
#     nodes, so it is not a branch that ships unrun.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "BOARD-MACHINE TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4860,server,nowait \
    -qmp tcp:127.0.0.1:4861,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/board-machine.out"
python3 scripts/shell-demo.py 4860 4861 "${TMPDIR:-/tmp}/board-machine.png" \
    "echo warmup" "board machine" "board machine test" "board check" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "board machine|tree source|tree model|tree compatible|map compatible|verdict|\[ok\]|\[FAIL\]|rules held|agree, " "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md kernel/src/virt.dtb <<'PY'
import re, sys
s    = open(sys.argv[1], "rb").read().decode("latin1")
doc  = open(sys.argv[2], encoding="utf-8").read()
dtb  = open(sys.argv[3], "rb").read()

def fail(msg):
    print(f"BOARD-MACHINE TEST FAILED ({msg})"); sys.exit(1)

def need(*frags):
    for f in frags:
        if f not in s:
            fail(f"missing '{f}'")

# --- the expected names come from the blueprint, not from board.rs ----------------
m = re.search(r"^\|\s*Root `compatible`\s*\|(.+)$", doc, re.M)
if not m:
    fail("blueprint row 'Root `compatible`' not found in docs/15")
cols = [c.strip() for c in m.group(1).split("|")[:3]]        # virt | RPi4 | PINE64
names = [re.findall(r"`([^`]+)`", c) for c in cols]
virt, rpi4, a64 = names
if virt != ["linux,dummy-virt"]:
    fail(f"blueprint virt compatible moved: {virt}")
if "brcm,bcm2711" not in rpi4 or "allwinner,sun50i-a64" not in a64:
    fail(f"blueprint rpi4/a64 compatibles moved: {rpi4} {a64}")

# ...and the virt name is independently true of the fixture QEMU actually emitted.
if b"linux,dummy-virt" not in dtb:
    fail("the bundled virt.dtb does not contain 'linux,dummy-virt' — the printed "
         "tree identity would be the kernel quoting itself")

# --- part 1: the verdict on THIS machine ------------------------------------------
need("board machine: qemu-virt (aarch64) vs device tree",
     "tree source      bundled reference virt.dtb (firmware passed no pointer)",
     f"tree model       '{virt[0]}'",
     f"tree compatible  '{virt[0]}'",
     f"map compatible   '{virt[0]}'",
     "verdict          same machine")
if "OTHER MACHINE" in s.split("board machine test")[0]:
    fail("the virt kernel called its own virt tree a foreign machine")

# --- part 2: the wrong-machine verdict, against synthetic root nodes ---------------
rules = s.split("identity self-test against synthetic root nodes")[-1]
for phrase in ("a virt tree is the virt board's own machine",
               "an rpi4 kernel handed a virt tree says OTHER MACHINE (the milestone's case)",
               "and the reverse: a virt kernel handed a Pi 4 tree is just as wrong",
               "a name anywhere in the list counts, not only the first",
               "a tree naming itself only in `model` is still identifiable",
               "a tree that names no machine is unstated, not wrong",
               "a near-miss SoC number is a different machine, not a match",
               "whole names only: `pine64` is not satisfied by `pine64-plus`",
               "a wrong-machine tree can be named: the Pi 3 tree resolves to the Pi 3 map"):
    if f"[ok]   {phrase}" not in rules:
        fail(f"identity rule not proved: {phrase}")
if "[FAIL]" in rules:
    fail("an identity rule did not hold")
need("board machine: 9/9 identity rules held")

# --- part 3: 2.725's `board check` is undisturbed ---------------------------------
# The counted rows became six when 2.730 added DRAM base; the size row is printed
# and not counted, so this line still reads "0 rows the tree does not describe".
need("=> 6 agree, 0 disagree (0 rows the tree does not describe)")

print(f"BOARD-MACHINE TEST PASSED  (root identity from docs/15 + the bundled DTB: "
      f"tree and map both '{virt[0]}' => same machine; 9/9 identity rules held "
      f"against synthetic root nodes, including the rpi4-kernel-given-a-virt-tree "
      f"case and whole-name matching; board check still all-agree / 0 disagree)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "BOARD-MACHINE TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
