#!/usr/bin/env python3
"""jitter-log: check a PC serial log of `rng jitter dump <n>` and turn it into ea_non_iid input (ADR 0008 step 1).

  python3 scripts/lib/jitter-log.py serial.log              # check only
  python3 scripts/lib/jitter-log.py serial.log out/laptop   # check, then write out/laptop.u8.bin + .u64.bin

The kernel prints a header line (`rng jitter dump: n=<n> ...`), then `jd` lines of hex deltas (TSC ticks, 16 per
line), then `rng jitter end: n=.. min=.. max=.. mean=.. distinct-low-bytes=.. sha256=..`. The sha256 is over the
deltas as 8-byte little-endian values, computed by the kernel, so a dropped or garbled serial line shows up here.

Outputs: `<prefix>.u8.bin` holds the low byte of each delta (8-bit samples: `ea_non_iid <file> 8`); `<prefix>.u64.bin`
holds the full deltas as 8-byte little-endian values, for any later run on wider samples.
Exit 0: the log is whole and every check agrees. Exit 1: something is missing or does not match (each one printed).
"""
import hashlib
import re
import struct
import sys

ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
HEAD = re.compile(r"rng jitter dump: n=(\d+) ")
JD = re.compile(r"^jd((?: [0-9a-f]+)+)$")
END = re.compile(r"rng jitter end: n=(\d+) min=([0-9a-f]+) max=([0-9a-f]+) mean=(\d+) "
                 r"distinct-low-bytes=(\d+) sha256=([0-9a-f]{64})$")


def parse(text):
    """(n from the header, deltas, the end line's match) for the first dump in the log."""
    n, deltas, end = None, [], None
    for raw in text.splitlines():
        line = ANSI.sub("", raw).replace("\r", "").strip()
        if n is None:
            m = HEAD.search(line)
            if m:
                n = int(m.group(1))
            continue
        m = JD.match(line)
        if m:
            deltas += [int(x, 16) for x in m.group(1).split()]
            continue
        m = END.search(line)
        if m:
            end = m
            break
    return n, deltas, end


def check(n, deltas, end):
    bad = []
    if n is None:
        return ["no 'rng jitter dump: n=' header line"]
    if end is None:
        bad.append("no 'rng jitter end:' line (the dump did not finish, or the log was cut)")
    if len(deltas) != n:
        bad.append("the header says n=%d but the log holds %d deltas" % (n, len(deltas)))
    if end is not None and deltas:
        en, mn, mx, mean, distinct, sha = end.groups()
        h = hashlib.sha256(b"".join(struct.pack("<Q", d) for d in deltas)).hexdigest()
        checks = [
            ("n", int(en), n),
            ("min", int(mn, 16), min(deltas)),
            ("max", int(mx, 16), max(deltas)),
            ("mean", int(mean), sum(deltas) // len(deltas)),
            ("distinct-low-bytes", int(distinct), len({d & 0xFF for d in deltas})),
            ("sha256", sha, h),
        ]
        for name, said, got in checks:
            if said != got:
                bad.append("the kernel's %s is %s but the logged deltas give %s" % (name, said, got))
    return bad


def main(argv):
    if len(argv) not in (2, 3):
        raise SystemExit(__doc__)
    n, deltas, end = parse(open(argv[1], "rb").read().decode("latin1"))
    bad = check(n, deltas, end)
    for b in bad:
        print("jitter-log: FAIL, " + b)
    if bad:
        return 1
    print("jitter-log: OK, n=%d deltas, min %d max %d mean %d ticks, %d distinct low bytes, sha256 matches"
          % (n, min(deltas), max(deltas), sum(deltas) // n, len({d & 0xFF for d in deltas})))
    if len(argv) == 3:
        with open(argv[2] + ".u8.bin", "wb") as f:
            f.write(bytes(d & 0xFF for d in deltas))
        with open(argv[2] + ".u64.bin", "wb") as f:
            f.write(b"".join(struct.pack("<Q", d) for d in deltas))
        print("jitter-log: wrote %s.u8.bin (%d B, 8-bit samples for ea_non_iid) and %s.u64.bin (%d B)"
              % (argv[2], n, argv[2], 8 * n))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
