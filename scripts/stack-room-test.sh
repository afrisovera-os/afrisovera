#!/usr/bin/env bash
# Verify the heap's room in the unit that spends it, and the refusal that reaches the
# caller (milestone 2.735) — what 2.734's named bound actually COSTS.
#
# 2.734 gave the arena a named bound: exhaustion stopped being an unattributed null and
# became "[heap] REFUSED ... the arena has no room left". That exposed the next gap. The
# heap's largest customer by three orders of magnitude is the 16 KiB kernel stack — one
# per `sched::spawn` thread, one per `process::spawn` EL0 process — and both call sites
# answered a null with `assert!(!stack.is_null(), ...)`, i.e. a PANIC. So:
#   * the published ceiling was "1 MiB free", a figure nobody can act on. The number that
#     decides whether the next spawn survives is "room for N more threads", and it is not
#     free-bytes/16 KiB either, because the arena charges alignment padding per grant;
#   * and crossing the ceiling took the whole machine down instead of failing the one
#     spawn that asked for the stack that was not there.
#
# Checked here:
#   * `heap` and `ps` both state the room in stacks — served, and how many more fit;
#   * the figure is CONSISTENT with the bytes beside it (never larger than free/16 KiB)
#     and it MOVES: spawning an EL0 process takes exactly one off it. That is the check
#     that says the number comes from the live arena rather than from a constant;
#   * `heap test` proves `capacity()` against synthetic arenas, including the case a
#     division gets wrong (padding) and the zero-size case that would otherwise hang;
#   * and in the source: neither spawn path asserts on a null stack any more, both return
#     the bound to a caller, and every call site goes through a path that handles it.
#     That last part cannot be observed from a shell — the live kernel allocates a few
#     dozen KiB into a mebibyte and never comes near the bound — which is exactly why it
#     had never run before this milestone.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "STACK ROOM TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4866,server,nowait \
    -qmp tcp:127.0.0.1:4867,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/stackroom.out"
# `ps` before and after a spawn: the room must move by exactly the stacks taken.
python3 scripts/shell-demo.py 4866 4867 "${TMPDIR:-/tmp}/stackroom.png" \
    "echo warmup" "ps" "rustapp" "ps" "heap" "heap test" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "in stacks|stacks:|on a refusal|room for|NOT spawned|\[ok\]   the|\[FAIL\]|bump checks held" \
    "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/heap.rs kernel/src/sched.rs kernel/src/process.rs kernel/src/main.rs <<'PY'
import re, sys
s     = open(sys.argv[1], "rb").read().decode("latin1")
heap  = open(sys.argv[2], encoding="utf-8").read()
sched = open(sys.argv[3], encoding="utf-8").read()
proc  = open(sys.argv[4], encoding="utf-8").read()
main  = open(sys.argv[5], encoding="utf-8").read()

def fail(msg):
    print(f"STACK ROOM TEST FAILED ({msg})"); sys.exit(1)

def code(src):
    """The source with its line comments removed.

    The checks below ask what the kernel DOES. This milestone's doc-comments quote the
    very `assert!` they replaced — that is the point of them, and a grep that cannot tell
    a citation from a call would force the code to be changed silently or the history to
    go unwritten. Strip `//`, `///` and `//!` lines and ask the remainder."""
    return "\n".join(ln for ln in src.splitlines() if not ln.lstrip().startswith("//"))

heap, sched, proc, main = (code(x) for x in (heap, sched, proc, main))

# --- part 1: the panic is gone from BOTH spawn paths --------------------------------
# The whole point of the milestone. A single surviving assert on a null stack puts the
# machine back to dying on a bound the caller could have absorbed.
for name, src in (("sched.rs", sched), ("process.rs", proc)):
    for pat in (r"assert!\(\s*!\w*stack\.is_null\(\)", r"\.expect\(\"no frame for L[12]\"\)"):
        if re.search(pat, src):
            fail(f"{name} still panics on an allocation bound (/{pat}/) — the refusal has "
                 f"to reach a caller, that is the milestone")
    if "alloc::alloc::alloc" in src:
        fail(f"{name} still allocates a stack behind heap::alloc_stack()'s back, so its "
             f"refusal is neither counted nor named")

# One door for the heap's largest customer, and it hands back a reason.
if "pub fn alloc_stack() -> Result<*mut u8, &'static str>" not in heap:
    fail("heap::alloc_stack is not the single named-refusal door for kernel stacks")
if "pub const STACK_SIZE: usize" not in heap or "STACK_SIZE: usize = heap::STACK_SIZE" not in proc:
    fail("the 16 KiB stack figure is not declared once in the allocator that pays for it "
         "— a second copy is a second opinion the moment either moves")
if "crate::heap::STACK_SIZE" not in sched:
    fail("sched.rs does not take its stack size from the heap")

# Both spawns return the bound rather than swallowing or asserting on it.
if "pub fn spawn(entry: extern \"C\" fn(usize) -> !, arg: usize) -> Result<(), &'static str>" not in sched:
    fail("sched::spawn does not return its bound to the caller")
if not re.search(r"unsafe fn spawn\((?:[^)]*\n)*?\) -> Result<\(\), &'static str> \{", proc):
    fail("process::spawn does not return its bound to the caller")
for name, src in (("sched.rs", sched), ("process.rs", proc)):
    if "#[must_use" not in src:
        fail(f"{name}'s spawn is not #[must_use] — a Result nobody reads is the assert "
             f"back again, only quieter")

# --- part 2: every call site handles it ---------------------------------------------
# A refusal that reaches a caller who drops it on the floor is not a refusal that reached
# anyone. `spawn` itself is private to process.rs; the demos go through spawn_or_refuse.
if "unsafe fn spawn_or_refuse(" not in proc:
    fail("process.rs has no spawn_or_refuse — the demos would each have to unwrap")
bare = [ln.strip() for ln in proc.splitlines()
        if re.search(r"(?<![_a-z])spawn\(\s*\d", ln) and "spawn_or_refuse" not in ln
        and not ln.strip().startswith("//")]
bare = [ln for ln in bare if "match spawn(" not in ln]
if bare:
    fail(f"{len(bare)} process spawn call site(s) still bypass spawn_or_refuse: {bare[:2]}")
if re.search(r"^\s*sched::spawn\([^)]*\);\s*$", main, re.M):
    fail("main.rs drops sched::spawn's Result on the floor")
if "NOT spawned" not in proc or "NOT spawned" not in main:
    fail("a refusal is not reported on the console by the caller that absorbed it")

# --- part 3: the room, as the live kernel reports it --------------------------------
# `heap`: the free bytes restated in the unit that spends them.
m = re.search(r"^  in stacks      (\d+) served, room for (\d+) more at (\d+) KiB", s, re.M)
if not m:
    fail("`heap` does not state its room in stacks — the bytes are back to being the "
         "only published figure")
served, room, kib = int(m.group(1)), int(m.group(2)), int(m.group(3))
if kib != 16:
    fail(f"the stack unit is {kib} KiB, not the 16 KiB both spawn paths take")
if served == 0:
    fail("the kernel reports no stack served, yet it ran EL0 processes at boot — the "
         "counter is not wired to the live allocator")
if room == 0:
    fail("the live arena reports room for no further thread — this kernel is one spawn "
         "from the failure 2.735 exists to make survivable")

# Consistency with the bytes printed beside it: padding can only ever make the count
# SMALLER than the naive division, never larger.
m = re.search(r"^  used           (\d+) B \((\d+) B of it alignment padding\), (\d+) B free",
              s, re.M)
if not m:
    fail("`heap` no longer states its use, so the stack figure cannot be cross-checked")
used, free = int(m.group(1)), int(m.group(3))
if room > free // (kib * 1024):
    fail(f"room for {room} stacks over-states {free} B free — free/{kib} KiB is {free // (kib*1024)}, "
         f"and alignment padding only ever costs more, never less")
if room < free // (kib * 1024) - 1:
    fail(f"room for {room} stacks under-states {free} B free by more than one grant of "
         f"padding — the figure is not being computed from the live arena")

# --- part 4: and it MOVES ------------------------------------------------------------
# The check that separates a live figure from a constant: `rustapp` spawns exactly one
# EL0 process between the two `ps` calls, so served must be +1 and room -1.
rows = re.findall(r"^    stacks: (\d+) served since boot, room for (\d+) more at (\d+) KiB",
                  s, re.M)
if len(rows) < 2:
    fail(f"`ps` printed {len(rows)} stack rows, expected 2 (before and after a spawn) — "
         f"the process table's other ceiling is not reported beside its slots")
(s0, r0, _), (s1, r1, _) = (tuple(map(int, r)) for r in rows[:2])
if s1 != s0 + 1:
    fail(f"one EL0 process was spawned between the two `ps` calls, but served went "
         f"{s0} -> {s1}; the counter does not track real stacks")
if r1 != r0 - 1:
    fail(f"served rose by one but the room went {r0} -> {r1}; the remaining figure is "
         f"not recomputed from the arena the spawn just spent")
if s1 != served or r1 != room:
    fail(f"`ps` says {s1} served / {r1} left, `heap` says {served} / {room} — two "
         f"opinions about one arena")
if "NOT spawned" in s:
    fail("a spawn was refused on an ordinary boot")
if not re.search(r"^  on a refusal   the spawn fails and says which bound", s, re.M):
    fail("`heap` does not state what now happens when the bound fires")

# --- part 5: the counting itself, against synthetic arenas ---------------------------
# capacity() is the arithmetic behind every figure above, and the live arena exercises
# exactly one point of it (a mostly-empty one). These are the rest.
cases = s.split("kernel heap: bump self-test against synthetic requests")[-1]
for phrase in (
        "an empty arena is counted in whole grants of the size that will be asked for",
        "each grant already served takes exactly one off the count; a full arena serves 0",
        # The case a division gets wrong: padding is charged per grant, so counting and
        # dividing disagree, and the allocator will do what the counting says.
        "PADDING is counted, so the answer is smaller than free-bytes/size would claim",
        "a request no arena state can serve counts as no room, rather than looping",
        "a zero-byte request consumes nothing, so the room for it is unbounded, not a hang",
        "the count is exact: every grant it promised is served, and the next one refuses",
        "the live arena's room in stacks is non-zero and never over-states the free bytes"):
    if f"[ok]   {phrase}" not in cases:
        fail(f"room not proved: {phrase}")
if "[FAIL]" in cases:
    fail("a bump check did not hold")
m = re.search(r"kernel heap: (\d+)/(\d+) bump checks held", cases)
if not m:
    fail("`heap test` prints no tally")
if m.group(1) != m.group(2):
    fail(f"only {m.group(1)} of {m.group(2)} bump checks held")
checks = int(m.group(2))

print(f"STACK ROOM TEST PASSED  (the 16 KiB kernel stack is declared once, in the "
      f"allocator that pays for it, and taken through the single door heap::alloc_stack, "
      f"which refuses BY NAME with 2.734's bound instead of returning a null; neither "
      f"sched::spawn nor process::spawn asserts on that null any more — both return the "
      f"bound to a caller, both are #[must_use], and every call site absorbs it and says "
      f"so on the console, so a full arena now costs one demo rather than the machine; "
      f"the arena's remaining room is published in the unit that spends it — `heap` and "
      f"`ps` agree at {served} served / room for {room} more at 16 KiB, never over-stating "
      f"the {free} B free beside it, and it MOVED by exactly one grant when one EL0 "
      f"process was spawned between two `ps` calls, so the figure is the live arena's and "
      f"not a constant; {checks}/{checks} bump checks held, including the padding case a "
      f"free-bytes/size division gets wrong and the zero-size case that would otherwise "
      f"count forever)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "STACK ROOM TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
