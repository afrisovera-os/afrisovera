#!/usr/bin/env python3
"""Write keytest.wasm, A4's test app (claude/nia-a4, NIA_A4_PLAN.md): it prints every key it gets.

  keys-wasm.py DIR             writes DIR/keytest.wasm
  keys-wasm.py --rust          prints it as the Rust array nia.rs bundles in test images (feature test-apps)

It asks for print, input and draw. With the desk showing (cell_open gives 1) it waits on cell_event and prints each
key event's payload; a close request prints -3 and returns. Without a desk it prints each getkey result. Either way
`q` (113) ends it after printing.
"""
import sys

I32 = 0x7F
END = b'\x0B'


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


LOOP, IF, BR, RET = b'\x03\x40', b'\x04\x40', b'\x0C', b'\x0F'
EQ, SHR_U, AND = b'\x46', b'\x76', b'\x71'
T_V_I, T_I, T_II_I, T_I_I, T_VV = range(5)
TYPES = [functype([I32], []), functype([], [I32]), functype([I32] * 2, [I32]), functype([I32], [I32]), functype([], [])]
IMPORTS = [(b'print', T_V_I), (b'getkey', T_I), (b'cell_open', T_II_I), (b'cell_event', T_I_I)]
PRINT, GETKEY, CELL_OPEN, CELL_EVENT = range(4)
RUN = 4
PERMS = [b'print', b'input', b'draw']
Q = ord('q')

# locals: 0 e, 1 kind, 2 p
DESK = (LOOP
        + c(-1) + call(CELL_EVENT) + ls(0)
        + lg(0) + c(28) + SHR_U + c(7) + AND + ls(1)
        + lg(1) + c(3) + EQ + IF + c(-3) + call(PRINT) + RET + b'\x0B'
        + lg(1) + c(4) + EQ + IF + RET + b'\x0B'
        + lg(1) + c(1) + EQ + IF
        + lg(0) + c(0x0FFFFFFF) + AND + ls(2) + lg(2) + call(PRINT)
        + lg(2) + c(Q) + EQ + IF + RET + b'\x0B'
        + b'\x0B'
        + BR + uleb(0) + END)
SHELL = (LOOP
         + call(GETKEY) + ls(0) + lg(0) + call(PRINT)
         + lg(0) + c(Q) + EQ + IF + RET + b'\x0B'
         + BR + uleb(0) + END)
RUN_BODY = (b'\x01\x03\x7F'
            + c(20) + c(4) + call(CELL_OPEN) + c(1) + EQ + IF + DESK + RET + b'\x0B'
            + SHELL + END)


def keytest():
    code = uleb(1) + uleb(len(RUN_BODY)) + RUN_BODY
    return (b'\x00asm\x01\x00\x00\x00'
            + section(1, uleb(len(TYPES)) + b''.join(TYPES))
            + section(2, uleb(len(IMPORTS)) + b''.join(name(b'afri') + name(n) + b'\x00' + uleb(t)
                                                        for n, t in IMPORTS))
            + section(3, uleb(1) + uleb(T_VV))
            + section(7, b'\x01' + name(b'run') + b'\x00' + uleb(RUN))
            + section(10, code)
            + section(0, name(b'afri.permissions') + uleb(len(PERMS)) + b''.join(name(x) for x in PERMS)))


def main():
    t = keytest()
    if sys.argv[1:] == ['--rust']:
        print('// scripts/lib/keys-wasm.py --rust (%d bytes)' % len(t))
        print('static WASM_KEYTEST: [u8; %d] = [' % len(t))
        for k in range(0, len(t), 16):
            print('    ' + ' '.join('0x%02X,' % b for b in t[k:k + 16]))
        print('];')
        return
    import os
    os.makedirs(sys.argv[1], exist_ok=True)
    open(os.path.join(sys.argv[1], 'keytest.wasm'), 'wb').write(t)
    print('keytest.wasm %d bytes' % len(t))


if __name__ == '__main__':
    main()
