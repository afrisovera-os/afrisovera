#!/usr/bin/env python3
"""pc-compat0's judge (Daraja stage 0 step 3, COMPAT_STAGE0_PLAN section 7): a Linux test program run through the
bridge on AfriSovera against the same binary on Linux (scripts/compat-linux-ref-test.sh's build/compat-ref/).

    compat0-compare.py <serial log> <ref dir> <name>...     compare each named program
    compat0-compare.py --selftest                           the judge on canned logs, both directions

From the serial log, per program (between "[daraja] linux <name>: begin" and "...: end"):
- out1 and out2: the bytes of every "[daraja] out1 <hex>" / "out2" line, in order, joined;
- the exit code: the "[daraja] exit <n>" line;
- the calls: the "[daraja] call <nr>" lines, in order.
Against the reference: <name>.out1, <name>.out2, <name>.exit and <name>.calls (strace's call names, in order). Output
and exit must be equal byte for byte (bench0's output holds a timing, so only its exit is compared); the call numbers
must equal strace's list, by name, through the x86-64 numbers below (from the Linux manual pages' syscall list).
"""
import sys

# x86-64 Linux call numbers for the names strace may print for these programs (syscalls(2), the x86-64 table)
NUMBERS = {
    "read": 0, "write": 1, "open": 2, "close": 3, "fstat": 5, "lseek": 8, "mmap": 9, "brk": 12,
    "rt_sigaction": 13, "rt_sigprocmask": 14, "ioctl": 16, "readv": 19, "writev": 20, "exit": 60,
    "arch_prctl": 158, "set_tid_address": 218, "exit_group": 231, "openat": 257, "newfstatat": 262,
    "execve": 59, "getpriority": 140,
}
NO_OUTPUT_COMPARE = {"bench0"}  # its stdout is a cycle count


def section(log, name):
    begin, end = f"[daraja] linux {name}: begin", f"[daraja] linux {name}: end"
    out, inside = [], False
    for line in log.splitlines():
        line = line.rstrip("\r")
        if line.startswith(begin):
            inside, out = True, []
            continue
        if line.startswith(end):
            return out if inside else None
        if inside:
            out.append(line)
    return None


def parse(lines):
    out1, out2, exitc, calls = bytearray(), bytearray(), None, []
    for l in lines:
        if l.startswith("[daraja] out1 "):
            out1 += bytes.fromhex(l[len("[daraja] out1 "):].strip())
        elif l.startswith("[daraja] out2 "):
            out2 += bytes.fromhex(l[len("[daraja] out2 "):].strip())
        elif l.startswith("[daraja] exit "):
            exitc = int(l.split()[2])
        elif l.startswith("[daraja] call "):
            calls.append(int(l.split()[2]))
    return bytes(out1), bytes(out2), exitc, calls


def ref_calls(text):
    """strace's names, in order, after the program's own execve (the last one: the reference starts it through a small
    execve wrapper, whose calls come first); the execve itself is the start, not one of the program's calls."""
    names = [n.strip() for n in text.splitlines() if n.strip()]
    if "execve" in names:
        names = names[len(names) - 1 - names[::-1].index("execve") + 1:]
    return names


def judge(log, ref, name):
    """(ok, list of lines) for one program; `ref` maps file name to bytes."""
    msgs, ok = [], True
    sec = section(log, name)
    if sec is None:
        return False, [f"{name}: no complete run in the log (begin and end lines)"]
    o1, o2, ex, calls = parse(sec)
    want_exit = int(ref[f"{name}.exit"].decode().strip())
    if ex != want_exit:
        ok = False
        msgs.append(f"{name}: exit {ex}, Linux {want_exit}")
    if name not in NO_OUTPUT_COMPARE:
        for k, got in (("out1", o1), ("out2", o2)):
            want = ref[f"{name}.{k}"]
            if got != want:
                ok = False
                msgs.append(f"{name}: {k} differs: AfriSovera {got!r}, Linux {want!r}")
    names = ref_calls(ref[f"{name}.calls"].decode())
    unknown = [n for n in names if n not in NUMBERS]
    if unknown:
        ok = False
        msgs.append(f"{name}: strace names this judge has no number for: {unknown}")
    else:
        want_calls = [NUMBERS[n] for n in names]
        if calls != want_calls:
            ok = False
            msgs.append(f"{name}: calls {calls}, Linux {want_calls} ({' '.join(names)})")
    if ok:
        msgs.append(f"{name}: equal to Linux: exit {ex}, stdout {len(o1)} bytes, stderr {len(o2)} bytes, "
                    f"{len(calls)} calls ({' '.join(names)})")
    return ok, msgs


def selftest():
    ref = {
        "hello0.exit": b"7\n", "hello0.out1": b"hi\nHabari\n", "hello0.out2": b"err\n",
        "hello0.calls": b"execve\narch_prctl\nset_tid_address\nioctl\nwritev\nexit_group\n",
    }
    good = "\n".join([
        "afrisovera> linux hello0",
        "[daraja] linux hello0: begin (bridge 30000 bytes, bundle 9000 bytes)",
        "[daraja] call 158", "[daraja] call 218", "[daraja] call 16", "[daraja] call 20",
        "[daraja] out1 68690a", "[daraja] out1 4861626172690a", "[daraja] out2 6572720a",
        "[daraja] call 231", "[daraja] exit 7",
        "[daraja] linux hello0: end",
    ])
    cases = {
        "good": (good, True),
        "exit": (good.replace("[daraja] exit 7", "[daraja] exit 0"), False),
        "byte": (good.replace("4861626172690a", "4861626172790a"), False),
        "stderr": (good.replace("[daraja] out2 6572720a\n", ""), False),
        "call": (good.replace("[daraja] call 16\n", ""), False),
        "noend": (good.replace("[daraja] linux hello0: end", ""), False),
    }
    bad = 0
    for k, (log, want) in cases.items():
        ok, _ = judge(log, ref, "hello0")
        if ok != want:
            print(f"  FAIL selftest: the '{k}' log was {'accepted' if ok else 'refused'}")
            bad += 1
    print(f"  judge selftest: {len(cases) - bad} ok / {bad} FAIL")
    return bad == 0


def main():
    if sys.argv[1:] == ["--selftest"]:
        sys.exit(0 if selftest() else 1)
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    log = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
    refdir = sys.argv[2]
    all_ok = True
    for name in sys.argv[3:]:
        ref = {}
        for k in ("exit", "out1", "out2", "calls"):
            try:
                ref[f"{name}.{k}"] = open(f"{refdir}/{name}.{k}", "rb").read()
            except OSError:
                ref[f"{name}.{k}"] = b""
        ok, msgs = judge(log, ref, name)
        all_ok &= ok
        for m in msgs:
            print(("  " if ok else "  FAIL: ") + m)
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
