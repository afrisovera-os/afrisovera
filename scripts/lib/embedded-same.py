#!/usr/bin/env python3
"""Check that a test app's bytes compiled into the kernel are what its generator writes today.

  embedded-same.py GENERATOR.py KERNEL_FILE      prints "same: N bytes" and exits 0, or names the difference and
                                                  exits 1

The generator's `--rust` output (one `static NAME: [u8; N] = [...];` array) must appear byte for byte in the kernel
file. A suite runs this first, so a generator changed without the kernel's copy (A5's first the cloud runner run: mode l was
in the generator, not in the kernel) fails at once with a plain reason instead of a mode that prints nothing.
"""
import re
import subprocess
import sys


def array(text):
    m = re.search(r"static (\w+): \[u8; (\d+)\] = \[(.*?)\];", text, re.S)
    if not m:
        return None, None
    return m.group(1), bytes(int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{2})", m.group(3)))


def main():
    gen = subprocess.run([sys.executable, sys.argv[1], "--rust"], capture_output=True, text=True, check=True).stdout
    name, want = array(gen)
    src = open(sys.argv[2], encoding="utf-8").read()
    i = src.find("static %s:" % name)
    if name is None or i < 0:
        print("embedded-same: %s not found in %s" % (name, sys.argv[2]))
        sys.exit(1)
    _, got = array(src[i:])
    if got != want:
        print("embedded-same: %s in %s (%d bytes) is not what %s writes (%d bytes): run it with --rust and paste"
              % (name, sys.argv[2], len(got or b""), sys.argv[1], len(want)))
        sys.exit(1)
    print("same: %s, %d bytes" % (name, len(want)))


if __name__ == "__main__":
    main()
