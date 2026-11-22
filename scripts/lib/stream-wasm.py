#!/usr/bin/env python3
"""Write streamtest.wasm, A6's test app (claude/nia-a6, NIA_A6_PLAN.md and the architect's OK of 16 Nov): files read
in pieces and written through one stream that is committed once.

  stream-wasm.py DIR           writes DIR/streamtest.wasm and prints the two checksums the suite expects
  stream-wasm.py --rust        prints it as the Rust array nia.rs bundles in test images (feature test-apps)

It asks for print, putc, input and file, and writes only its own names (s.txt, a.txt, b.txt). The first key from
getkey picks the mode; every result is printed:
  w  pattern A into 16 KiB; file_open_write(s.txt) (1); 16 appends of 1 KiB (the last returns 16384); commit
     (16384); file_size (16384); 16 file_read_at of 1 KiB (their sum 16384); the checksum of what was read (SUM_A);
     file_read_at at offset 16384 (0); at offset -1 (-3)
  x  pattern B; open s.txt (1); 16 appends (16384); one more byte (-3, the stream is spoiled); commit (-3); then
     file_read s.txt (16384) and its checksum (SUM_A: the old file stays)
  c  pattern B; open s.txt (1); append 1 KiB (1024); then waits for keys until Ctrl+C stops it: the run's end drops
     the stream with its line, and s.txt stays as it was
  r  file_size s.txt, file_read s.txt and the checksum (16384 16384 SUM_A)
  d  open a.txt (1), append 10 (10), open b.txt (1: a.txt's stream dropped with its line), append 5 (5), commit (5);
     file_size a.txt (-1: never written) and b.txt (5)
  s  open afrisovera.conf (-4) and hello.wasm (-4); append and commit with no stream open (-1, -1)
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
LOAD8_U, STORE8 = b'\x2D\x00', b'\x3A\x00'

T_V_I, T_I, T_IIII_I, T_VV, T_III_V, T_II_I, T_I_I, T_III_I, T_IIIII_I = range(9)
TYPES = [functype([I32], []), functype([], [I32]), functype([I32] * 4, [I32]), functype([], []),
         functype([I32] * 3, []), functype([I32] * 2, [I32]), functype([I32], [I32]), functype([I32] * 3, [I32]),
         functype([I32] * 5, [I32])]
IMPORTS = [(b'print', T_V_I), (b'getkey', T_I), (b'file_read', T_IIII_I), (b'file_size', T_II_I),
           (b'file_read_at', T_IIIII_I), (b'file_open_write', T_II_I), (b'file_append', T_III_I),
           (b'file_commit', T_I_I)]
PRINT, GETKEY, READ, SIZE, READ_AT, OPEN, APPEND, COMMIT = range(8)
RUN, FILL, SUM, APP16, RD16 = 8, 9, 10, 11, 12
PERMS = [b'print', b'putc', b'input', b'file']
WBUF, RBUF = 4096, 24576
NAMES = {b's.txt': 0, b'a.txt': 16, b'b.txt': 32, b'afrisovera.conf': 48, b'hello.wasm': 80}
FULL, PIECE = 16384, 1024


def nm(n):
    return c(NAMES[n]) + c(len(n))


def pattern(k, n):
    return bytes((i * 7 + k) & 255 for i in range(n))


def checksum(b):
    h = 0
    for x in b:
        h = (h * 31 + x) & 0xFFFFFFFF
    return h - (1 << 32) if h >= 1 << 31 else h


SUM_A = checksum(pattern(1, FULL))
SUM_B = checksum(pattern(2, FULL))

FILL_BODY = (b'\x01\x01\x7F' + c(0) + ls(3) + BLOCK + LOOP
             + lg(3) + lg(1) + GE_U + BR_IF + uleb(1)
             + lg(0) + lg(3) + ADD + lg(3) + c(7) + MUL + lg(2) + ADD + c(255) + AND + STORE8 + uleb(0)
             + lg(3) + c(1) + ADD + ls(3) + BR + uleb(0) + END + END + END)
SUM_BODY = (b'\x01\x02\x7F' + c(0) + ls(2) + c(0) + ls(3) + BLOCK + LOOP
            + lg(2) + lg(1) + GE_U + BR_IF + uleb(1)
            + lg(3) + c(31) + MUL + lg(0) + lg(2) + ADD + LOAD8_U + uleb(0) + ADD + ls(3)
            + lg(2) + c(1) + ADD + ls(2) + BR + uleb(0) + END + END + lg(3) + END)
# app16(at) -> the last append's answer: 16 appends of 1 KiB from at. Locals: 1 i, 2 r.
APP16_BODY = (b'\x01\x02\x7F' + c(0) + ls(1) + BLOCK + LOOP
              + lg(1) + c(16) + GE_U + BR_IF + uleb(1)
              + c(1) + lg(0) + lg(1) + c(PIECE) + MUL + ADD + c(PIECE) + call(APPEND) + ls(2)
              + lg(1) + c(1) + ADD + ls(1) + BR + uleb(0) + END + END + lg(2) + END)
# rd16(at) -> the sum of 16 file_read_at(s.txt, i * 1 KiB, at + i * 1 KiB, 1 KiB). Locals: 1 i, 2 total.
RD16_BODY = (b'\x01\x02\x7F' + c(0) + ls(1) + c(0) + ls(2) + BLOCK + LOOP
             + lg(1) + c(16) + GE_U + BR_IF + uleb(1)
             + lg(2) + nm(b's.txt') + lg(1) + c(PIECE) + MUL + lg(0) + lg(1) + c(PIECE) + MUL + ADD + c(PIECE)
             + call(READ_AT) + ADD + ls(2)
             + lg(1) + c(1) + ADD + ls(1) + BR + uleb(0) + END + END + lg(2) + END)


def p(expr):
    return expr + call(PRINT)


def mode(key, body):
    return lg(0) + c(ord(key)) + EQ + IF + body + RET + END


OPEN_S = nm(b's.txt') + call(OPEN)
RUN_BODY = (b'\x01\x01\x7F' + call(GETKEY) + ls(0)
            + mode('w', c(WBUF) + c(FULL) + c(1) + call(FILL) + p(OPEN_S) + p(c(WBUF) + call(APP16))
                   + p(c(1) + call(COMMIT)) + p(nm(b's.txt') + call(SIZE)) + p(c(RBUF) + call(RD16))
                   + p(c(RBUF) + c(FULL) + call(SUM))
                   + p(nm(b's.txt') + c(FULL) + c(RBUF) + c(100) + call(READ_AT))
                   + p(nm(b's.txt') + c(-1) + c(RBUF) + c(10) + call(READ_AT)))
            + mode('x', c(WBUF) + c(FULL) + c(2) + call(FILL) + p(OPEN_S) + p(c(WBUF) + call(APP16))
                   + p(c(1) + c(WBUF) + c(1) + call(APPEND)) + p(c(1) + call(COMMIT))
                   + p(nm(b's.txt') + c(RBUF) + c(FULL) + call(READ)) + p(c(RBUF) + c(FULL) + call(SUM)))
            + mode('c', c(WBUF) + c(PIECE) + c(2) + call(FILL) + p(OPEN_S) + p(c(1) + c(WBUF) + c(PIECE) + call(APPEND))
                   + LOOP + call(GETKEY) + DROP + BR + uleb(0) + END)
            + mode('r', p(nm(b's.txt') + call(SIZE)) + p(nm(b's.txt') + c(RBUF) + c(FULL) + call(READ))
                   + p(c(RBUF) + c(FULL) + call(SUM)))
            + mode('d', p(nm(b'a.txt') + call(OPEN)) + p(c(1) + c(WBUF) + c(10) + call(APPEND))
                   + p(nm(b'b.txt') + call(OPEN)) + p(c(1) + c(WBUF) + c(5) + call(APPEND)) + p(c(1) + call(COMMIT))
                   + p(nm(b'a.txt') + call(SIZE)) + p(nm(b'b.txt') + call(SIZE)))
            + mode('s', p(nm(b'afrisovera.conf') + call(OPEN)) + p(nm(b'hello.wasm') + call(OPEN))
                   + p(c(1) + c(WBUF) + c(1) + call(APPEND)) + p(c(1) + call(COMMIT)))
            + END)


def streamtest():
    data = bytearray(96)
    for n, at in NAMES.items():
        data[at:at + len(n)] = n
    bodies = [RUN_BODY, FILL_BODY, SUM_BODY, APP16_BODY, RD16_BODY]
    ftypes = [T_VV, T_III_V, T_II_I, T_I_I, T_I_I]
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
    t = streamtest()
    if sys.argv[1:] == ['--rust']:
        print('// scripts/lib/stream-wasm.py --rust (%d bytes)' % len(t))
        print('static WASM_STREAMTEST: [u8; %d] = [' % len(t))
        for k in range(0, len(t), 16):
            print('    ' + ' '.join('0x%02X,' % b for b in t[k:k + 16]))
        print('];')
        return
    import os
    os.makedirs(sys.argv[1], exist_ok=True)
    open(os.path.join(sys.argv[1], 'streamtest.wasm'), 'wb').write(t)
    print('streamtest.wasm %d bytes SUM_A %d SUM_B %d' % (len(t), SUM_A, SUM_B))


if __name__ == '__main__':
    main()
