"""Write the wasm-imports apps (claude/nia-imports): 16 imported functions instead of 8.

  imports-wasm.py DIR     writes DIR/NAME.wasm for each app below

  im16    16 imports; the LAST one (index 15) is print, and the app calls it: prints 16, then 99
  im17    17 imports: the kernel refuses it at load ("too many imported functions (Nia holds 16)")
  clock9  the real case: print, putc, win_create, win_fill, win_text, win_present, time, pollkey, sleep_ms
          (9 imports). Every 200 ms (sleep_ms drives it) it draws the clock's seconds digit, time() % 10, until
          any key, then putc('T'), print(ticks) and print(time()). (Until OS_REQUESTS item 7 it drew the tick count
          and never called `time`, the architect's condition while `time` was wrong on the PC.)
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


I, L = 0x7F, 0x7E
SIG = {
    'print': ([I], []), 'print64': ([L], []), 'putc': ([I], []),
    'win_create': ([I, I], [I]), 'win_fill': ([I] * 6, []), 'win_present': ([], []), 'win_text': ([I] * 5, []),
    'time': ([], [I]), 'getkey': ([], [I]), 'random': ([], [I]), 'pollkey': ([], [I]), 'sleep_ms': ([I], [I]),
}


def module(imports, body, nlocals=0):
    """imports: field names (module 'afri'); a name not in SIG is an unknown host, typed () -> ()."""
    types = []

    def tidx(sig):
        enc = b'\x60' + vec([bytes([t]) for t in sig[0]]) + vec([bytes([t]) for t in sig[1]])
        if enc not in types:
            types.append(enc)
        return types.index(enc)

    imps = [name('afri') + name(f) + b'\x00' + uleb(tidx(SIG.get(f, ([], [])))) for f in imports]
    run_t = tidx(([], []))
    func = (vec([uleb(nlocals) + b'\x7F']) if nlocals else vec([])) + body + b'\x0B'
    m = b'\x00asm\x01\x00\x00\x00'
    m += section(1, vec(types))
    m += section(2, vec(imps))
    m += section(3, vec([uleb(run_t)]))
    m += section(5, vec([b'\x00\x01']))
    m += section(7, vec([name('run') + b'\x00' + uleb(len(imports))]))
    m += section(10, vec([uleb(len(func)) + func]))
    return m


C = lambda v: b'\x41' + sleb(v)
CALL = lambda i: b'\x10' + uleb(i)
GET = lambda i: b'\x20' + uleb(i)
SET = lambda i: b'\x21' + uleb(i)
TEE = lambda i: b'\x22' + uleb(i)

# 15 imports before print: the sideload functions and four unknown names (allowed while not called)
FIFTEEN = ['putc', 'win_create', 'win_fill', 'win_present', 'win_text', 'time', 'getkey', 'random', 'pollkey',
           'sleep_ms', 'print64', 'nope1', 'nope2', 'nope3', 'nope4']
assert len(FIFTEEN) == 15

CLOCK = ['print', 'putc', 'win_create', 'win_fill', 'win_text', 'win_present', 'time', 'pollkey', 'sleep_ms']
P, PUTC, WCREATE, WFILL, WTEXT, WPRESENT, TIME, POLL, SLEEP = range(9)
# locals: 0 = window id, 1 = ticks
CLOCK_BODY = (C(120) + C(40) + CALL(WCREATE) + SET(0)
              + b'\x03\x40'                                                       # loop
              + GET(0) + C(0) + C(0) + C(120) + C(40) + C(0x202040) + CALL(WFILL)
              + GET(0) + C(8) + C(16) + C(48) + CALL(TIME) + C(10) + b'\x70' + b'\x6A' + C(0xFFFFFF) + CALL(WTEXT)
              + CALL(WPRESENT)
              + GET(1) + C(1) + b'\x6A' + SET(1)
              + C(200) + CALL(SLEEP) + b'\x1A'
              + CALL(POLL) + C(-1) + b'\x46' + b'\x0D\x00'                       # no key: br_if 0 (tick again)
              + b'\x0B'
              + C(ord('T')) + CALL(PUTC) + GET(1) + CALL(P) + CALL(TIME) + CALL(P))

APPS = {
    'im16': (FIFTEEN + ['print'], C(16) + CALL(15) + C(99) + CALL(15), 0),
    'im17': (FIFTEEN + ['nope5', 'print'], C(17) + CALL(16), 0),
    'clock9': (CLOCK, CLOCK_BODY, 2),
}

if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, (imps, body, nl) in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(module(imps, body, nl))
        print(n, len(imps))
