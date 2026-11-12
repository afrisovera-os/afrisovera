"""Write the wasm-oob modules (claude/nia-addr): memory accesses at and past the end of 64 KiB, a base of -1, the
offset immediate, and data segments at the edge.

  oob-wasm.py DIR      writes DIR/oobN.wasm and prints one line per module: NAME KIND EXPECT
                       KIND trap   -> "[nia] TRAP: out of bounds memory access", and print(99) never runs
                       KIND value  -> EXPECT (a print/print64 line), then print(99)
                       KIND parse  -> "[nia] parse error: EXPECT"
                       KIND runs   -> no TRAP, print(99) runs (the draw case: -1 arguments are harmless)

Every module imports afri.print (i32) and afri.print64 (i64), declares one 64 KiB memory, exports run, prints
its number first, and prints 99 at the end. Each case is what a standard WebAssembly engine does.
"""
import os
import sys


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


def vec(items):
    return uleb(len(items)) + b''.join(items)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(s):
    return uleb(len(s)) + s.encode()


I32C = lambda v: b'\x41' + sleb(v)
I64C = lambda v: b'\x42' + sleb(v)
PRINT, PRINT64 = b'\x10\x00', b'\x10\x01'


def mem(op, align, off=0):
    return bytes([op]) + uleb(align) + uleb(off)


# extra imports after print (0) and print64 (1): (name, params, results); their types follow types 0..2
DRAW = [('win_create', 2, 1), ('win_fill', 6, 0), ('win_text', 5, 0)]


def module(num, body, data=None, raw_data=None, extra=(), local_i32=0):
    code = I32C(num) + PRINT + body + I32C(99) + PRINT + b'\x0B'
    func = (vec([uleb(local_i32) + b'\x7F']) if local_i32 else vec([])) + code
    types = [b'\x60\x01\x7F\x00', b'\x60\x01\x7E\x00', b'\x60\x00\x00']
    imps = [name('afri') + name('print') + b'\x00\x00', name('afri') + name('print64') + b'\x00\x01']
    for n, np_, nr in extra:
        imps.append(name('afri') + name(n) + b'\x00' + uleb(len(types)))
        types.append(b'\x60' + vec([b'\x7F'] * np_) + vec([b'\x7F'] * nr))
    run_idx = len(imps)
    m = b'\x00asm\x01\x00\x00\x00'
    m += section(1, vec(types))
    m += section(2, vec(imps))
    m += section(3, vec([b'\x02']))
    m += section(5, vec([b'\x00\x01']))
    m += section(7, vec([name('run') + b'\x00' + uleb(run_idx)]))
    m += section(10, vec([uleb(len(func)) + func]))
    if data is not None:
        off, blob = data
        m += section(11, vec([b'\x00' + I32C(off) + b'\x0B' + vec([bytes([x]) for x in blob])]))
    if raw_data is not None:
        m += section(11, b'\x01' + raw_data)  # one segment, its bytes as given
    return m


# The draw case: win_create(-1, -1) (clamped to the largest window), then win_fill and win_text with -1
# arguments into it, and win_fill on window -1. Before nia-addr, -1 became usize::MAX and tonga's `ry + rh`
# overflowed (a debug-kernel panic from any app: draw is allowed to unverified apps).
CALL = lambda i: b'\x10' + uleb(i)
LGET, LSET = b'\x20\x00', b'\x21\x00'
DRAW_BODY = (I32C(-1) + I32C(-1) + CALL(2) + LSET
             + LGET + I32C(-1) + I32C(-1) + I32C(-1) + I32C(-1) + I32C(0xFF0000) + CALL(3)
             + LGET + I32C(0) + I32C(0) + I32C(-1) + I32C(-1) + I32C(0x00FF00) + CALL(3)
             + LGET + I32C(-1) + I32C(-1) + I32C(65) + I32C(0xFFFFFF) + CALL(4)
             + I32C(-1) + I32C(-1) + I32C(-1) + I32C(-1) + I32C(-1) + I32C(0) + CALL(3))


CASES = [
    # name, kind, expect, body, data
    ('oob1', 'trap', '', I32C(-1) + mem(0x28, 2, 4) + PRINT, None),            # base -1 plus offset 4
    ('oob2', 'trap', '', I32C(65535) + mem(0x2F, 1) + PRINT, None),            # load16_u, 1 byte left
    ('oob3', 'value', 'print(16706)',
     I32C(65534) + I32C(0x4142) + mem(0x3B, 1) + I32C(65534) + mem(0x2F, 1) + PRINT, None),
    ('oob4', 'trap', '', I32C(65529) + mem(0x29, 3) + PRINT64, None),          # i64.load, 7 bytes left
    ('oob5', 'value', 'print64(72623859790382856)',
     I32C(65528) + I64C(0x0102030405060708) + mem(0x37, 3) + I32C(65528) + mem(0x29, 3) + PRINT64, None),
    ('oob6', 'value', 'print(90)',
     I32C(65535) + I32C(0x5A) + mem(0x3A, 0) + I32C(65535) + mem(0x2D, 0) + PRINT, None),
    ('oob7', 'trap', '', I32C(0) + mem(0x2D, 0, 65536) + PRINT, None),         # the offset immediate counts
    ('oob8', 'parse', 'data segment past the end of memory', b'', (-1, b'AB')),
    ('oob9', 'value', 'print(16961)', I32C(65534) + mem(0x2F, 1) + PRINT, (65534, b'AB')),
    ('oob10', 'trap', '', I32C(65535) + I32C(0x4142) + mem(0x3B, 1), None),    # store16, 1 byte left
    ('oob11', 'trap', '', I32C(-1) + I64C(7) + mem(0x3C, 0), None),             # i64.store8 at 4294967295
    # addition B: a segment whose length is a 10-byte LEB near 2^64 (c + len would overflow)
    ('oob12', 'parse', 'truncated data segment', b'',
     dict(raw_data=b'\x00' + I32C(0) + b'\x0B' + b'\xFF' * 9 + b'\x01')),
    # addition A: draw arguments of -1 (needs win_create, win_fill, win_text and one i32 local)
    ('oob13', 'runs', '', DRAW_BODY, dict(extra=DRAW, local_i32=1)),
]

if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for i, (n, kind, expect, body, data) in enumerate(CASES, 1):
        kw = data if isinstance(data, dict) else {'data': data}
        open(os.path.join(d, n + '.wasm'), 'wb').write(module(i, body, **kw))
        print(n, kind, expect)
