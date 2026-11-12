"""Write the wasm-time app (claude/nia-time, OS_REQUESTS item 7): `time` reads the board's clock on both kernels.

  time-wasm.py DIR     writes DIR/tm1.wasm

Imports (module afri): time (0), sleep_ms (1), print (2).
  tm1  print(time()); sleep_ms(2000); print(time())       two epoch seconds, at least 2 s apart
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


C = lambda v: b'\x41' + sleb(v)
TIME, SLEEP, PRINT = b'\x10\x00', b'\x10\x01', b'\x10\x02'
DROP = b'\x1A'

APPS = {
    'tm1': TIME + PRINT + C(2000) + SLEEP + DROP + TIME + PRINT,
}


def module(body):
    func = vec([]) + body + b'\x0B'
    m = b'\x00asm\x01\x00\x00\x00'
    # types: 0 () -> i32, 1 (i32) -> i32, 2 (i32) -> (), 3 () -> ()
    m += section(1, vec([b'\x60\x00\x01\x7F', b'\x60\x01\x7F\x01\x7F', b'\x60\x01\x7F\x00', b'\x60\x00\x00']))
    m += section(2, vec([name('afri') + name('time') + b'\x00\x00',
                         name('afri') + name('sleep_ms') + b'\x00\x01',
                         name('afri') + name('print') + b'\x00\x02']))
    m += section(3, vec([b'\x03']))
    m += section(5, vec([b'\x00\x01']))
    m += section(7, vec([name('run') + b'\x00\x03']))   # 3 imports, so the function is index 3
    m += section(10, vec([uleb(len(func)) + func]))
    return m


if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(module(body))
        print(n)
