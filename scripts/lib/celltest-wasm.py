"""Write celltest.wasm, the cell window test app (afriui.md section 25, stage 2; the desk-cell-app suite).

  celltest-wasm.py OUT.wasm

Hand-assembled here so the suite needs no Rust toolchain in the job and the app is the same bytes on every run.
It asks for print, draw, time and input in its afri.permissions section; run from the disk it is unsigned.

  start  print(cell_open(64, 24)); a page of 16 rows, row r in foreground role r on background role 15 - r;
         cell_cursor(1, 0, 0, 2)
  loop   e = cell_event(-1); print(e); then by kind (bits 28 to 30):
    1 key  q return; s spin for ever with no host calls; i ignore close requests from now on; w sleep_ms(3000);
           n print cell_put(1,0,-1,p,1), cell_cursor(1,0,0,7), cell_cursor(1,-1,0,1), cell_put(2,0,0,p,1),
             cell_open(64,24) (expected -5 -5 -5 -1 -3);
           p print(cell_event(200)) (expected 0); t print(cell_event(-5)) (waits like -1);
           g print(getkey()), print(pollkey()) (expected -2 -1);
           d call a chain of 15 functions, so the last is 16 call levels deep counting `run`, put a row there and
             print(cell_event(0)) (expected 0; Nia prints the stack left at depth 16). A chain, not recursion, so
             nia-apps' `nia.py check` (which refuses recursion) passes it too
    3 close request: return, unless i
    4 closed: print cell_put(1,0,0,p,1), cell_size(1), cell_event(0) (expected -2 -2 and kind 4), return
"""
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


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(b):
    return uleb(len(b)) + b


def c(v):  # i32.const
    return b'\x41' + sleb(v)


def get(i):
    return b'\x20' + uleb(i)


def set_(i):
    return b'\x21' + uleb(i)


def call(f):
    return b'\x10' + uleb(f)


IF, ELSE, END, LOOP, BR0, BRIF0, RET, DROP = b'\x04\x40', b'\x05', b'\x0B', b'\x03\x40', b'\x0C\x00', b'\x0D\x00', b'\x0F', b'\x1A'
EQ, EQZ, AND, ADD, SUB, MUL, LTU, SHRU = b'\x46', b'\x45', b'\x71', b'\x6A', b'\x6B', b'\x6C', b'\x49', b'\x76'

# types
T_V_I, T_II_I, T_5I_I, T_4I_I, T_I_I, T__I, T__V = range(7)
TYPES = [([0x7F], []), ([0x7F] * 2, [0x7F]), ([0x7F] * 5, [0x7F]), ([0x7F] * 4, [0x7F]), ([0x7F], [0x7F]),
         ([], [0x7F]), ([], [])]
# imports, in function index order
IMPORTS = [(b'print', T_V_I), (b'cell_open', T_II_I), (b'cell_put', T_5I_I), (b'cell_cursor', T_4I_I),
           (b'cell_size', T_I_I), (b'cell_event', T_I_I), (b'sleep_ms', T_I_I), (b'getkey', T__I), (b'pollkey', T__I)]
PRINT, OPEN, PUT, CURSOR, SIZE, EVENT, SLEEP, GETKEY, POLLKEY = range(9)
RUN, DEEP = 9, 10  # DEEP is the first of the chain, functions 10 to 24

ROW = 32  # cells per page row; 2 bytes each, rows at 64-byte steps from address 0
DEPTH = 16  # Nia's call depth limit: `run` is level 1, so the 15th function of the chain is at level 16


def page():
    data = bytearray()
    for r in range(16):
        text = ('role %2d  ' % r + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ')[:ROW].encode()
        attr = r | ((15 - r) << 4)
        for ch in text:
            data += bytes([ch, attr])
    return bytes(data)


def key(ch, body):
    """if (k == ch) { body }"""
    return get(1) + c(ord(ch)) + EQ + IF + body + END


def run_body():
    # locals: 0 e, 1 k (the key code), 2 ignore, 3 r, 4 kind
    b = c(64) + c(24) + call(OPEN) + call(PRINT)
    b += c(0) + set_(3) + LOOP
    b += c(1) + c(0) + get(3) + get(3) + c(2 * ROW) + MUL + c(ROW) + call(PUT) + DROP
    b += get(3) + c(1) + ADD + b'\x22\x03' + c(16) + LTU + BRIF0 + END
    b += c(1) + c(0) + c(0) + c(2) + call(CURSOR) + DROP
    b += LOOP
    b += c(-1) + call(EVENT) + b'\x22\x00' + call(PRINT)
    b += get(0) + c(28) + SHRU + set_(4)
    b += get(0) + c((1 << 28) - 1) + AND + set_(1)
    keys = (key('q', RET)
            + key('s', LOOP + BR0 + END)
            + key('i', c(1) + set_(2))
            + key('w', c(3000) + call(SLEEP) + DROP)
            + key('n', c(1) + c(0) + c(-1) + c(0) + c(1) + call(PUT) + call(PRINT)
                  + c(1) + c(0) + c(0) + c(7) + call(CURSOR) + call(PRINT)
                  + c(1) + c(-1) + c(0) + c(1) + call(CURSOR) + call(PRINT)
                  + c(2) + c(0) + c(0) + c(0) + c(1) + call(PUT) + call(PRINT)
                  + c(64) + c(24) + call(OPEN) + call(PRINT))
            + key('p', c(200) + call(EVENT) + call(PRINT))
            + key('t', c(-5) + call(EVENT) + call(PRINT))
            + key('g', call(GETKEY) + call(PRINT) + call(POLLKEY) + call(PRINT))
            + key('d', call(DEEP)))
    b += get(4) + c(1) + EQ + IF + keys + END
    b += get(4) + c(3) + EQ + get(2) + EQZ + AND + IF + RET + END
    b += get(4) + c(4) + EQ + IF
    b += c(1) + c(0) + c(0) + c(0) + c(1) + call(PUT) + call(PRINT)
    b += c(1) + call(SIZE) + call(PRINT)
    b += c(0) + call(EVENT) + call(PRINT) + RET + END
    b += BR0 + END
    return b'\x01\x05\x7F' + b + END


def deep_body(i):
    """Function DEEP + i of the chain (i = 0 to DEPTH - 2); the last puts a row and waits for no event."""
    if i < DEPTH - 2:
        return b'\x00' + call(DEEP + i + 1) + END
    return b'\x00' + c(1) + c(0) + c(20) + c(0) + c(ROW) + call(PUT) + DROP + c(0) + call(EVENT) + call(PRINT) + END


def module():
    types = b''.join(b'\x60' + uleb(len(p)) + bytes(p) + uleb(len(r)) + bytes(r) for p, r in TYPES)
    imports = b''.join(name(b'afri') + name(n) + b'\x00' + uleb(t) for n, t in IMPORTS)
    bodies = [run_body()] + [deep_body(i) for i in range(DEPTH - 1)]
    code = uleb(len(bodies)) + b''.join(uleb(len(x)) + x for x in bodies)
    data = page()
    perms = name(b'afri.permissions') + uleb(4) + name(b'print') + name(b'draw') + name(b'time') + name(b'input')
    return (b'\x00asm\x01\x00\x00\x00'
            + section(1, uleb(len(TYPES)) + types)
            + section(2, uleb(len(IMPORTS)) + imports)
            + section(3, uleb(len(bodies)) + bytes([T__V] * len(bodies)))
            + section(5, b'\x01\x00\x01')
            + section(7, b'\x01' + name(b'run') + b'\x00' + uleb(RUN))
            + section(10, code)
            + section(11, b'\x01\x00' + c(0) + END + uleb(len(data)) + data)
            + section(0, perms))


if __name__ == '__main__':
    open(sys.argv[1], 'wb').write(module())
