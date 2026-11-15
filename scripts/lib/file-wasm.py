#!/usr/bin/env python3
"""Write filetest.wasm, A2's test app (claude/nia-a2, NIA_A2_PLAN.md): files up to 16 KiB and saves on the disk.

  file-wasm.py DIR             writes DIR/filetest.wasm and prints the two checksums the suite expects
  file-wasm.py --rust          prints it as the Rust array nia.rs bundles in test images (feature test-apps)

The `file` permission is never given to a disk app, so the test runs the bundled copy; a disk copy is the
permission test. It asks for print, putc, input and file, and writes only its own names (big.txt, cut.txt). The first key
from getkey picks the mode; every result is printed:
  w  write 16,384 bytes of pattern A to big.txt, read them back (cap 16,384), print the write's and the read's
     results and the checksum (h * 31 + byte) of what was read: 16384 16384 SUM_A
  x  write 16,385 bytes to big.txt (refused, -3), then read big.txt (16384, the old file stays): -3 16384 SUM_A
  o  the edges: read a missing name (-1); read big.txt with cap 0 (0) and cap 600 (600); write afrisovera.conf (-4)
     and hello.wasm (-4); then a write of 10,000 bytes at 60,000: TRAP out of bounds, so 99 is never printed
  r  read big.txt and print the result and the checksum (16384 SUM_A after a reboot)
  v  write cut.txt again and again, pattern A, then B, then A..., for ever: the suite kills QEMU during it
  k  read cut.txt and print the result, then the checksum: -5 (damaged by the cut) or 16384 and SUM_A or SUM_B
  l  (A5) file_list with cap 600: print the result and put the listing out with putc; file_list with one byte too
     few: print the result (minus the size) and the marker byte (85: nothing written); then a list past the app's
     memory: TRAP (99 never printed)
"""
import sys

I32 = 0x7F
END = b'\x0B'
DROP = b'\x1A'


def uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def sleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if (n == 0 and not b & 0x40) or (n == -1 and b & 0x40):
            out.append(b)
            return bytes(out)
        out.append(b | 0x80)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(b):
    return uleb(len(b)) + b


def c(v):
    return b'\x41' + sleb(v)


def call(f):
    return b'\x10' + uleb(f)


def lg(i):
    return b'\x20' + uleb(i)


def ls(i):
    return b'\x21' + uleb(i)


def functype(params, results):
    return b'\x60' + uleb(len(params)) + bytes(params) + uleb(len(results)) + bytes(results)


BLOCK, LOOP, IF = b'\x02\x40', b'\x03\x40', b'\x04\x40'
BR, BR_IF, RET = b'\x0C', b'\x0D', b'\x0F'
ADD, MUL, AND, EQ, GE_U = b'\x6A', b'\x6C', b'\x71', b'\x46', b'\x4F'
LOAD8_U, STORE8 = b'\x2D\x00', b'\x3A\x00'  # align 1, then the offset

T_V_I, T_I, T_IIII_I, T_VV, T_III_V, T_II_I = range(6)
TYPES = [functype([I32], []), functype([], [I32]), functype([I32] * 4, [I32]), functype([], []),
         functype([I32] * 3, []), functype([I32] * 2, [I32])]
IMPORTS = [(b'print', T_V_I), (b'getkey', T_I), (b'file_write', T_IIII_I), (b'file_read', T_IIII_I),
           (b'putc', T_V_I), (b'file_list', T_II_I)]
PRINT, GETKEY, WRITE, READ, PUTC, LIST = range(6)
RUN, FILL, SUM = 6, 7, 8
PERMS = [b'print', b'putc', b'input', b'file']
LBUF, LBUF2 = 41984, 40960  # file_list's buffer, and a byte that a refused call must leave as it was
WBUF, RBUF = 4096, 24576  # the 16 KiB write buffer and the read-back buffer
# names in the data section at 0
NAMES = {b'big.txt': 0, b'cut.txt': 16, b'nofile': 32, b'afrisovera.conf': 48, b'hello.wasm': 80}
FULL = 16384


def nm(n):
    return c(NAMES[n]) + c(len(n))


def pattern(k, n):
    """Pattern k (1 = A, 2 = B): byte i is (i * 7 + k) & 255."""
    return bytes((i * 7 + k) & 255 for i in range(n))


def checksum(b):
    """The app's checksum: h = h * 31 + byte, wrapping at 32 bits, printed signed (Nia's print is an i32)."""
    h = 0
    for x in b:
        h = (h * 31 + x) & 0xFFFFFFFF
    return h - (1 << 32) if h >= 1 << 31 else h


SUM_A = checksum(pattern(1, FULL))
SUM_B = checksum(pattern(2, FULL))

# fill(at, n, k): pattern k into memory. Locals: 3 i.
FILL_BODY = (b'\x01\x01\x7F' + c(0) + ls(3) + BLOCK + LOOP
             + lg(3) + lg(1) + GE_U + BR_IF + uleb(1)
             + lg(0) + lg(3) + ADD + lg(3) + c(7) + MUL + lg(2) + ADD + c(255) + AND + STORE8 + uleb(0)
             + lg(3) + c(1) + ADD + ls(3) + BR + uleb(0) + END + END + END)
# sum(at, n) -> the checksum, s = s * 31 + byte. Locals: 2 i, 3 s.
SUM_BODY = (b'\x01\x02\x7F' + c(0) + ls(2) + c(0) + ls(3) + BLOCK + LOOP
            + lg(2) + lg(1) + GE_U + BR_IF + uleb(1)
            + lg(3) + c(31) + MUL + lg(0) + lg(2) + ADD + LOAD8_U + uleb(0) + ADD + ls(3)
            + lg(2) + c(1) + ADD + ls(2) + BR + uleb(0) + END + END + lg(3) + END)


def p(expr):
    return expr + call(PRINT)


def write(n, at, length):
    return nm(n) + c(at) + c(length) + call(WRITE)


def read(n, at, cap):
    return nm(n) + c(at) + c(cap) + call(READ)


def mode(key, body):
    return lg(0) + c(ord(key)) + EQ + IF + body + RET + END


READ_BACK = p(read(b'big.txt', RBUF, FULL)) + p(c(RBUF) + c(FULL) + call(SUM))
# l (A5): the listing with cap 600 (printed, then put out byte by byte), the same with one byte too few (minus the
# size, and the marker byte 85 untouched), then a list past the app's memory (TRAP: 99 never printed).
LIST_MODE = (c(LBUF2) + c(85) + STORE8 + uleb(0)
             + c(LBUF) + c(600) + call(LIST) + ls(1) + p(lg(1))
             + c(0) + ls(2) + BLOCK + LOOP + lg(2) + lg(1) + GE_U + BR_IF + uleb(1)
             + c(LBUF) + lg(2) + ADD + LOAD8_U + uleb(0) + call(PUTC) + lg(2) + c(1) + ADD + ls(2) + BR + uleb(0)
             + END + END
             + p(c(LBUF2) + lg(1) + c(1) + b'\x6B' + call(LIST)) + p(c(LBUF2) + LOAD8_U + uleb(0))
             + c(65530) + c(600) + call(LIST) + DROP + p(c(99)))
RUN_BODY = (b'\x01\x03\x7F' + call(GETKEY) + ls(0)
            + mode('w', c(WBUF) + c(FULL) + c(1) + call(FILL) + p(write(b'big.txt', WBUF, FULL)) + READ_BACK)
            + mode('x', p(write(b'big.txt', WBUF, FULL + 1)) + READ_BACK)
            + mode('o', p(read(b'nofile', RBUF, FULL)) + p(read(b'big.txt', RBUF, 0)) + p(read(b'big.txt', RBUF, 600))
                   + p(write(b'afrisovera.conf', WBUF, 10)) + p(write(b'hello.wasm', WBUF, 10))
                   + write(b'big.txt', 60000, 10000) + DROP + p(c(99)))
            + mode('r', READ_BACK)
            + mode('v', LOOP + c(WBUF) + c(FULL) + c(1) + call(FILL) + write(b'cut.txt', WBUF, FULL) + DROP
                   + c(WBUF) + c(FULL) + c(2) + call(FILL) + write(b'cut.txt', WBUF, FULL) + DROP + BR + uleb(0) + END)
            + mode('k', p(read(b'cut.txt', RBUF, FULL)) + p(c(RBUF) + c(FULL) + call(SUM)))
            + mode('l', LIST_MODE)
            + END)


def filetest():
    data = bytearray(96)
    for n, at in NAMES.items():
        data[at:at + len(n)] = n
    bodies = [RUN_BODY, FILL_BODY, SUM_BODY]
    ftypes = [T_VV, T_III_V, T_II_I]
    code = uleb(len(bodies)) + b''.join(uleb(len(x)) + x for x in bodies)
    return (b'\x00asm\x01\x00\x00\x00'
            + section(1, uleb(len(TYPES)) + b''.join(TYPES))
            + section(2, uleb(len(IMPORTS)) + b''.join(name(b'afri') + name(n) + b'\x00' + uleb(t)
                                                        for n, t in IMPORTS))
            + section(3, uleb(len(ftypes)) + b''.join(uleb(t) for t in ftypes))
            + section(5, b'\x01\x00\x01')
            + section(7, b'\x01' + name(b'run') + b'\x00' + uleb(RUN))
            + section(10, code)
            + section(11, b'\x01\x00' + c(0) + END + uleb(len(data)) + bytes(data))
            + section(0, name(b'afri.permissions') + uleb(len(PERMS)) + b''.join(name(x) for x in PERMS)))


def main():
    t = filetest()
    if sys.argv[1:] == ['--rust']:
        print('// scripts/lib/file-wasm.py --rust (%d bytes)' % len(t))
        print('static WASM_FILETEST: [u8; %d] = [' % len(t))
        for k in range(0, len(t), 16):
            print('    ' + ' '.join('0x%02X,' % b for b in t[k:k + 16]))
        print('];')
        return
    import os
    os.makedirs(sys.argv[1], exist_ok=True)
    open(os.path.join(sys.argv[1], 'filetest.wasm'), 'wb').write(t)
    print('filetest.wasm %d bytes SUM_A %d SUM_B %d' % (len(t), SUM_A, SUM_B))


if __name__ == '__main__':
    main()
