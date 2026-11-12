"""Write the wasm-poll apps (claude/nia-poll, OS_REQUESTS item 5): pollkey and sleep_ms.

  poll-wasm.py DIR     writes DIR/NAME.wasm for each app below

Every app imports afri.print (i32), afri.pollkey () -> i32 and afri.sleep_ms (i32) -> i32, declares one 64 KiB
memory and exports run. Locals: 0 = k (the last key), 1 = n (a counter).
  pk1  print(pollkey())                      no key typed: prints -1 at once
  pk2  loop: k = pollkey(); if k == -1 { sleep_ms(100); n += 1; continue }; if k == 'q' break; print(k)
       then print(n)                         typed a, b, q: prints 97, 98, then n > 0
  pk3  loop: k = pollkey(); if k != -1 print(k)          Ctrl+C must TRAP (never printed as 3)
  sl1  print(sleep_ms(1500))                 1500 to 1600
  sl2  print(sleep_ms(-1))                   the clamp: 10000 to 10100
  sl3  loop: sleep_ms(10000)                 Ctrl+C during the sleep must TRAP within about 1 s
  sl4  sleep_ms(3000); print(pollkey())      a key typed during the sleep is kept: 120 for x
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
PRINT, POLL, SLEEP = b'\x10\x00', b'\x10\x01', b'\x10\x02'
GET = lambda i: b'\x20' + uleb(i)
SET = lambda i: b'\x21' + uleb(i)
TEE = lambda i: b'\x22' + uleb(i)
BLOCK, LOOP, IF, END = b'\x02\x40', b'\x03\x40', b'\x04\x40', b'\x0B'
BR = lambda l: b'\x0C' + uleb(l)
BR_IF = lambda l: b'\x0D' + uleb(l)
DROP, ADD, EQ, NE = b'\x1A', b'\x6A', b'\x46', b'\x47'

APPS = {
    'pk1': POLL + PRINT,
    'pk2': (BLOCK + LOOP
            + POLL + TEE(0) + C(-1) + EQ + IF
            + C(100) + SLEEP + DROP + GET(1) + C(1) + ADD + SET(1) + BR(1)   # labels: if 0, loop 1, block 2
            + END
            + GET(0) + C(113) + EQ + BR_IF(1)                               # 'q': out of the block
            + GET(0) + PRINT + BR(0)
            + END + END
            + GET(1) + PRINT),
    'pk3': LOOP + POLL + TEE(0) + C(-1) + NE + IF + GET(0) + PRINT + END + BR(0) + END,
    'sl1': C(1500) + SLEEP + PRINT,
    'sl2': C(-1) + SLEEP + PRINT,
    'sl3': LOOP + C(10000) + SLEEP + DROP + BR(0) + END,
    'sl4': C(3000) + SLEEP + DROP + POLL + PRINT,
}


def module(body):
    func = vec([b'\x02\x7F']) + body + END  # two i32 locals
    m = b'\x00asm\x01\x00\x00\x00'
    m += section(1, vec([b'\x60\x01\x7F\x00', b'\x60\x00\x01\x7F', b'\x60\x01\x7F\x01\x7F', b'\x60\x00\x00']))
    m += section(2, vec([name('afri') + name('print') + b'\x00\x00',
                         name('afri') + name('pollkey') + b'\x00\x01',
                         name('afri') + name('sleep_ms') + b'\x00\x02']))
    m += section(3, vec([b'\x03']))
    m += section(5, vec([b'\x00\x01']))
    m += section(7, vec([name('run') + b'\x00\x03']))
    m += section(10, vec([uleb(len(func)) + func]))
    return m


if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(module(body))
        print(n)
