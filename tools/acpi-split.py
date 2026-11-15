#!/usr/bin/env python3
"""Split a file saved by AfriSovera's `acpisave` (copied by report-from-stick as <time>-stick-<n>.bin) into its
ACPI tables, one file each, for the team to read with an AML disassembler (AML rung C2, 14 Nov 2026).

    python tools/acpi-split.py <saved .bin> <out folder>

The file is a title line, "AFRISOVERA ACPI TABLES 1", then whole tables one after another, each starting with its
4-letter signature and its length. Only DSDT, SSDT and FACP are ever in it (the kernel's fixed list); anything else,
a table running past the end, or a checksum that does not add up stops the split with a line. Writes DSDT.dat,
FACP.dat, SSDT1.dat, SSDT2.dat ... into the out folder (new files only).

These are the laptop maker's tables: keep them in the team's notes folder, never in git or a cloud job.
Standard library only.
"""
import os
import struct
import sys

TITLE = b"AFRISOVERA ACPI TABLES 1\n"
ALLOWED = (b"DSDT", b"SSDT", b"FACP")


def split(data):
    """The tables in `data`: a list of (signature, bytes); raises ValueError with a reason."""
    if not data.startswith(TITLE):
        raise ValueError("not an acpisave file (its title line is missing)")
    at, out = len(TITLE), []
    while at < len(data):
        if at + 8 > len(data):
            raise ValueError("a table header cut short at byte %d" % at)
        sig = data[at:at + 4]
        n = struct.unpack_from("<I", data, at + 4)[0]
        if sig not in ALLOWED:
            raise ValueError("a %r table at byte %d, which acpisave never saves" % (sig, at))
        if n < 36 or at + n > len(data):
            raise ValueError("the %s table at byte %d runs past the end" % (sig.decode(), at))
        t = data[at:at + n]
        if sum(t) % 256 != 0:
            raise ValueError("the %s table at byte %d does not add up (checksum)" % (sig.decode(), at))
        out.append((sig.decode(), t))
        at += n
    return out


def main(a):
    if len(a) != 2:
        print(__doc__)
        return 2
    try:
        tables = split(open(a[0], "rb").read())
    except (OSError, ValueError) as e:
        print("acpi-split: %s" % e)
        return 1
    os.makedirs(a[1], exist_ok=True)
    k = 0
    for sig, t in tables:
        if sig == "SSDT":
            k += 1
            name = "SSDT%d.dat" % k
        else:
            name = "%s.dat" % sig
        path = os.path.join(a[1], name)
        try:
            with open(path, "xb") as f:
                f.write(t)
        except FileExistsError:
            print("acpi-split: %s is already there; nothing overwritten" % path)
            return 1
        print("%s: %d bytes" % (path, len(t)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
