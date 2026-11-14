"""Write the wasm-textrun apps (claude/nia-a1a3: OS_REQUESTS item 10 A1, 128 functions and 32 types, and A3,
win_text_run).

  textrun-wasm.py DIR             writes DIR/NAME.wasm for each app below, one "NAME expected" line per app
  textrun-wasm.py --rust NAME     prints that app as a Rust byte array (f129 is wasm-fuzz's guard case)

A1, the limits. Imports and defined functions share one index space: print is function 0, run is 1, and the
other 127 defined functions are 2 to 128 (the last). Function k takes p = k % 4 i32 parameters (type 2 + k % 30,
so all 32 types are used) and returns k plus its parameters. run calls each of 2..128 in turn with every
argument 1, adds the results and prints the sum, so a function index off by the import count, or a wrong type or
code entry for any function, prints another number.
  f128  128 functions, 32 types: prints SUM_F128 and returns cleanly.
  f129  129 functions: refused at load, "too many functions (Nia holds 128)".
  t33   33 types: refused at load, "too many types (Nia holds 32)".

A3, win_text_run(win, x, y, ptr, len, color). Each tr app opens a 200 x 60 window, draws, presents and prints 1;
tr0 to tr2 then wait for a key, so the suite can take a screenshot while they wait.
  tr0  the window only (no text): the picture tr1 must differ from.
  tr1  "AFRI 123" with one win_text_run at (8, 8).
  tr2  the same eight characters with eight win_text calls at (8 + 8 i, 8): the same pixels as tr1.
  tr3  the edges, printing 2, 3, 4, 5 after each (then the 1), and returning cleanly:
       len 0 (draws nothing); len 1000 at ptr 65500 (cut to 30 before the memory check, so 65500 + 30 fits);
       x = 0xFFFFFFF0 (stops at the edge); window id 9 (draws nothing).
  tr4  ptr 65530, len 30: past the 64 KiB, TRAP "out of bounds memory access" before any print.
"""
import os
import sys

I32 = 0x7F
END, DROP = b'\x0B', b'\x1A'


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


def functype(params, results):
    return b'\x60' + uleb(len(params)) + bytes(params) + uleb(len(results)) + bytes(results)


def module(types, imports, bodies, func_types, run, perms, mem=False, data=b''):
    code = uleb(len(bodies)) + b''.join(uleb(len(x)) + x for x in bodies)
    out = (b'\x00asm\x01\x00\x00\x00'
           + section(1, uleb(len(types)) + b''.join(types))
           + section(2, uleb(len(imports)) + b''.join(name(b'afri') + name(n) + b'\x00' + uleb(t) for n, t in imports))
           + section(3, uleb(len(func_types)) + b''.join(uleb(t) for t in func_types)))
    if mem:
        out += section(5, b'\x01\x00\x01')
    out += section(7, b'\x01' + name(b'run') + b'\x00' + uleb(run)) + section(10, code)
    if data:
        out += section(11, b'\x01\x00' + c(0) + END + uleb(len(data)) + data)
    return out + section(0, name(b'afri.permissions') + uleb(len(perms)) + b''.join(name(p) for p in perms))


# --- A1 -----------------------------------------------------------------------------------------------------------
def limits_app(nfuncs, ntypes):
    """print (import 0), run (1), then nfuncs - 1 more; the result is (bytes, the sum run prints)."""
    types = [functype([I32], []), functype([], [])]
    types += [functype([I32] * ((t - 2) % 4), [I32]) for t in range(2, ntypes)]

    def tk(k):  # the type of function k (k >= 2): 2 + k % (ntypes - 2), so every type from 2 up is used
        return 2 + k % (ntypes - 2)

    def nparams(k):
        return (tk(k) - 2) % 4

    bodies, ftypes, total = [], [1], 0
    run = b'\x00'
    for k in range(2, nfuncs + 1):
        p = nparams(k)
        run += c(1) * p + call(k) + (b'\x6A' if k > 2 else b'')  # i32.add after the first
        total += k + p
        bodies.append(b'\x00' + c(k) + b''.join(b'\x20' + uleb(j) + b'\x6A' for j in range(p)) + END)
        ftypes.append(tk(k))
    run += call(0) + END
    return module(types, [(b'print', 0)], [run] + bodies, ftypes, 1, [b'print']), total


F128, SUM_F128 = limits_app(128, 32)
F129, _ = limits_app(129, 32)
T33, _ = limits_app(128, 33)

# --- A3 -----------------------------------------------------------------------------------------------------------
TEXT = b'AFRI 123'
WHITE = 0xFFFFFF
# types: 0 (i32)->(), 1 ()->(), 2 (i32,i32)->i32, 3 (i32 x6)->(), 4 (i32 x5)->(), 5 ()->i32
TR_TYPES = [functype([I32], []), functype([], []), functype([I32] * 2, [I32]), functype([I32] * 6, []),
            functype([I32] * 5, []), functype([], [I32])]
PRINT, WIN_CREATE, TEXT_RUN, WIN_TEXT, PRESENT, GETKEY = range(6)
TR_IMPORTS = [(b'print', 0), (b'win_create', 2), (b'win_text_run', 3), (b'win_text', 4), (b'win_present', 1),
              (b'getkey', 5)]
W = b'\x20\x00'  # local.get 0: the window id


def tr_app(draw, wait=True):
    """local 0 = win_create(200, 60); draw; win_present; print(1); getkey (when wait)."""
    body = (b'\x01\x01\x7F' + c(200) + c(60) + call(WIN_CREATE) + b'\x21\x00' + draw + call(PRESENT)
            + c(1) + call(PRINT) + ((call(GETKEY) + DROP) if wait else b'') + END)
    return module(TR_TYPES, TR_IMPORTS, [body], [1], len(TR_IMPORTS), [b'print', b'draw', b'input'], True, TEXT)


def text_run(win, x, y, ptr, n, color=WHITE):
    return win + c(x) + c(y) + c(ptr) + c(n) + c(color) + call(TEXT_RUN)


TR0 = tr_app(b'')
TR1 = tr_app(text_run(W, 8, 8, 0, len(TEXT)))
TR2 = tr_app(b''.join(W + c(8 + 8 * i) + c(8) + c(ch) + c(WHITE) + call(WIN_TEXT) for i, ch in enumerate(TEXT)))
TR3 = tr_app(text_run(W, 8, 8, 0, 0) + c(2) + call(PRINT)
             + text_run(W, 0, 30, 65500, 1000) + c(3) + call(PRINT)
             + text_run(W, -16, 8, 0, 8) + c(4) + call(PRINT)
             + text_run(c(9), 8, 8, 0, 8) + c(5) + call(PRINT), wait=False)
TR4 = tr_app(text_run(W, 8, 8, 65530, 30) + c(6) + call(PRINT), wait=False)

APPS = {
    'f128': (F128, 'print(%d), returned cleanly' % SUM_F128),
    'f129': (F129, 'parse error: too many functions (Nia holds 128)'),
    't33': (T33, 'parse error: too many types (Nia holds 32)'),
    'tr0': (TR0, 'print(1), waits for a key'),
    'tr1': (TR1, 'print(1), waits for a key; the same picture as tr2'),
    'tr2': (TR2, 'print(1), waits for a key; the same picture as tr1'),
    'tr3': (TR3, 'print 2 3 4 5 1, returned cleanly'),
    'tr4': (TR4, 'no print, TRAP: out of bounds memory access'),
}

if __name__ == '__main__':
    if sys.argv[1] == '--rust':
        b = APPS[sys.argv[2]][0]
        rows = [b[i:i + 16] for i in range(0, len(b), 16)]
        print('[u8; %d] = [' % len(b))
        for r in rows:
            print('    ' + ', '.join('0x%02X' % x for x in r) + ',')
        print('];')
    else:
        d = sys.argv[1]
        os.makedirs(d, exist_ok=True)
        for n, (b, want) in APPS.items():
            open(os.path.join(d, n + '.wasm'), 'wb').write(b)
            print('%s %s' % (n, want))
