#!/usr/bin/env python3
"""Device firmware files (13 Nov 2026, FIRMWARE_PLAN.md and the architect's verdict of 13 Nov).

firmware/MANIFEST lists every firmware file AfriSovera may send to a device: name, size, sha256, licence file,
vendor, version and where it comes from. The binaries are not kept in git: `fetch` makes the folder that goes into
\\EFI\\afrisovera\\firmware\\ on the boot stick, downloading each file from its pinned source (or making it, for
AfriSovera's own test file) and stopping on any wrong size or hash.

    fwtool.py check              # every row has its licence text; fwdb.rs matches the MANIFEST; generated files hash right
    fwtool.py gen fwtest OUT     # write AfriSovera's own test file
    fwtool.py fetch DIR [CACHE]  # every MANIFEST file into DIR, its licence beside it, checked by hash; downloads kept in CACHE
"""
import hashlib
import os
import re
import shutil
import sys
import urllib.request

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
MANIFEST = os.path.join(ROOT, "firmware", "MANIFEST")
LICENCES = os.path.join(ROOT, "firmware", "licences")
FWDB = os.path.join(ROOT, "kernel", "src", "pc", "fwdb.rs")
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$")  # a plain file name: no folder, no "..", fits the long-name buffer


def gen_fwtest():
    """AfriSovera's own test file: 70000 bytes of SHA-256 blocks (not any device's firmware)."""
    out = bytearray()
    i = 0
    while len(out) < 70000:
        out += hashlib.sha256(b"afrisovera fwtest " + i.to_bytes(4, "little")).digest()
        i += 1
    return bytes(out[:70000])


def gen_btpatch(build):
    """AfriSovera's own Intel-legacy-shaped test patch for the test Bluetooth controller (pc-bt): eight commands
    (vendor opcode 0xFC52, four bytes each) and the Command Complete each must answer with. Build 2 expects an error
    status from its fifth command, which the test controller never gives (the "wrong event" case). Not any
    device's firmware."""
    out = bytearray()
    for k in range(1, 9):
        out += bytes([0x01, 0x52, 0xFC, 4, k, build, 0xA5, 0x5A])
        status = 1 if (build == 2 and k == 5) else 0
        out += bytes([0x02, 0x0E, 4, 1, 0x52, 0xFC, status])
    return bytes(out)


def bt_name(build):
    """The name the kernel forms from the test controller's Intel Read Version (bt.rs intel_name)."""
    return "ibt-hw-37.8.10-fw-1.1.%x.2c.13.bseq" % build


GENERATED = {
    "gen:fwtest": gen_fwtest,
    "gen:btpatch1": lambda: gen_btpatch(1),
    "gen:btpatch2": lambda: gen_btpatch(2),
    "gen:btpatch4": lambda: gen_btpatch(4),
}


def rows():
    """The MANIFEST's rows as dicts; exits with a line on any malformed row."""
    out = []
    with open(MANIFEST, encoding="utf-8") as f:
        for k, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            p = line.split()
            if len(p) != 7:
                sys.exit(f"fwtool: MANIFEST line {k}: 7 fields expected (name size sha256 licence vendor version source)")
            r = dict(zip(("name", "size", "sha256", "licence", "vendor", "version", "source"), p))
            if not NAME_RE.match(r["name"]) or ".." in r["name"]:
                sys.exit(f"fwtool: MANIFEST line {k}: {r['name']!r} is not a plain file name")
            if not r["size"].isdigit() or not 0 < int(r["size"]) <= 4 << 20:
                sys.exit(f"fwtool: MANIFEST line {k}: size must be 1 to 4194304 bytes")
            if not re.fullmatch(r"[0-9a-f]{64}", r["sha256"]):
                sys.exit(f"fwtool: MANIFEST line {k}: sha256 must be 64 lower-case hex digits")
            if not (r["source"] in GENERATED or r["source"].startswith("https://")):
                sys.exit(f"fwtool: MANIFEST line {k}: the source must be an https address or a generated test file")
            r["size"] = int(r["size"])
            out.append(r)
    return out


def fwdb_rows():
    """(name, size, sha256 hex) of every row of fwdb.rs's table (ROWS, which FIRMWARE is)."""
    src = open(FWDB, encoding="utf-8").read()
    m = re.search(r"const ROWS: \[Fw; \d+\] = \[(.*?)\n\];", src, re.S)
    if not m:
        sys.exit("fwtool: fwdb.rs has no ROWS table")
    return re.findall(r'name: "([^"]+)",\s*size: (\d+),\s*sha256: hex\("([0-9a-f]{64})"\)', m.group(1))


def check():
    bad = 0
    ms = rows()
    names = [r["name"] for r in ms]
    if len(set(n.lower() for n in names)) != len(names):
        print("fwtool: two MANIFEST rows have the same name"); bad += 1
    for r in ms:
        lic = os.path.join(LICENCES, r["licence"])
        if not os.path.isfile(lic) or os.path.getsize(lic) == 0:
            print(f"fwtool: {r['name']}: its licence text firmware/licences/{r['licence']} is missing or empty"); bad += 1
        if r["source"] in GENERATED:
            b = GENERATED[r["source"]]()
            if len(b) != r["size"] or hashlib.sha256(b).hexdigest() != r["sha256"]:
                print(f"fwtool: {r['name']}: the generated file does not match its size and sha256"); bad += 1
    db = [(n, int(s), h) for n, s, h in fwdb_rows()]
    ma = [(r["name"], r["size"], r["sha256"]) for r in ms]
    if sorted(db) != sorted(ma):
        print("fwtool: fwdb.rs's table and firmware/MANIFEST differ:")
        for x in sorted(set(db) - set(ma)):
            print(f"  only in fwdb.rs: {x}")
        for x in sorted(set(ma) - set(db)):
            print(f"  only in MANIFEST: {x}")
        bad += 1
    used = {r["licence"] for r in ms}
    for f in sorted(os.listdir(LICENCES)):
        if f not in used:
            print(f"fwtool: firmware/licences/{f} belongs to no MANIFEST row"); bad += 1
    print(f"fwtool: {len(ms)} firmware file(s) in the MANIFEST, {len(db)} in fwdb.rs, {bad} problem(s)")
    return bad == 0


def fetch(dest, cache=None):
    os.makedirs(dest, exist_ok=True)
    for r in rows():
        if r["source"] in GENERATED:
            b = GENERATED[r["source"]]()
        else:
            cached = os.path.join(cache, r["sha256"]) if cache else None
            if cached and os.path.isfile(cached):
                b = open(cached, "rb").read()
            else:
                with urllib.request.urlopen(r["source"], timeout=60) as u:
                    b = u.read(r["size"] + 1)
        if len(b) != r["size"] or hashlib.sha256(b).hexdigest() != r["sha256"]:
            sys.exit(f"fwtool: {r['name']}: wrong size or sha256 from {r['source']}; the build stops")
        if cache and r["source"] not in GENERATED:
            os.makedirs(cache, exist_ok=True)
            open(os.path.join(cache, r["sha256"]), "wb").write(b)
        open(os.path.join(dest, r["name"]), "wb").write(b)
        shutil.copyfile(os.path.join(LICENCES, r["licence"]), os.path.join(dest, r["licence"]))
    shutil.copyfile(MANIFEST, os.path.join(dest, "MANIFEST"))
    print(f"fwtool: firmware folder ready in {dest}")


def main(a):
    if a[:1] == ["check"]:
        return 0 if check() else 1
    if len(a) == 3 and a[0] == "gen" and "gen:" + a[1] in GENERATED:
        open(a[2], "wb").write(GENERATED["gen:" + a[1]]())
        return 0
    if a[:1] == ["fetch"] and len(a) in (2, 3):
        fetch(a[1], a[2] if len(a) == 3 else None)
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
