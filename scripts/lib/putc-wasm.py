"""Write the wasm-putc apps (claude/nia-putc, OS_REQUESTS item 6): putc text shown at once, safely.

  putc-wasm.py DIR     writes DIR/NAME.wasm for each app below

Imports (module afri): putc (0), pollkey (1), sleep_ms (2), print (3), time (4), random (5).
  pu1  putc "Hi", wait for a key (pollkey + sleep_ms 50), putc "!\\n"      "Hi" is on screen BEFORE the key
  pu2  FF 'A' C3 '\\n' | C3 A9 1B "[2J" 07 '\\n' | 'x' E2 80 AE C2 85 'y' '\\n'
       lines: U+FFFD A U+FFFD | e-acute ?[2J? | x??y  (invalid and cut UTF-8; ESC, BEL; bidi U+202E; C1 U+0085)
  pu3  putc "before", then unreachable                                      the text survives the TRAP
  pu4  300 x '.', then '\\n'                                                 all 300 shown
  pu5  putc "ok", then E2 82 (a cut 3-byte character), return              "ok" U+FFFD, the line closed
  pu6  putc 'a'; print(5); putc 'b'                                         three lines, in order
  pu7  putc "t="; time(); random(); pollkey(); sleep_ms(1); putc "x\\n"     one line: quiet calls keep it open
  sa   putc "abcde" FF, then unreachable      (before boots only: leaves an invalid byte in the old buffer)
  sb   putc "hi", return                      (before boots only: the old kernel panics on sa's stale byte)
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
PUTC, POLL, SLEEP, PRINT, TIME, RAND = (b'\x10\x00', b'\x10\x01', b'\x10\x02', b'\x10\x03', b'\x10\x04',
                                        b'\x10\x05')


def puts(bs):
    return b''.join(C(b) + PUTC for b in bs)


WAIT_KEY = (b'\x02\x40\x03\x40'                                   # block, loop
            + POLL + C(-1) + b'\x47' + b'\x0D\x01'                # key: br_if 1 (out)
            + C(50) + SLEEP + b'\x1A' + b'\x0C\x00'               # sleep 50; br 0
            + b'\x0B\x0B')
DOTS = (b'\x03\x40' + C(ord('.')) + PUTC
        + b'\x20\x00' + C(1) + b'\x6A' + b'\x22\x00' + C(300) + b'\x48' + b'\x0D\x00'   # i += 1; br_if i < 300
        + b'\x0B' + C(10) + PUTC)

APPS = {
    'pu1': puts(b'Hi') + WAIT_KEY + puts(b'!\n'),
    'pu2': puts(b'\xffA\xc3\n' + b'\xc3\xa9\x1b[2J\x07\n' + b'x\xe2\x80\xae\xc2\x85y\n'),
    'pu3': puts(b'before') + b'\x00',
    'pu4': DOTS,
    'pu5': puts(b'ok\xe2\x82'),
    'pu6': puts(b'a') + C(5) + PRINT + puts(b'b'),
    'pu7': puts(b't=') + TIME + b'\x1A' + RAND + b'\x1A' + POLL + b'\x1A' + C(1) + SLEEP + b'\x1A' + puts(b'x\n'),
    'sa': puts(b'abcde\xff') + b'\x00',
    'sb': puts(b'hi'),
}


def module(body):
    func = vec([b'\x01\x7F']) + body + b'\x0B'   # one i32 local (pu4's counter)
    m = b'\x00asm\x01\x00\x00\x00'
    m += section(1, vec([b'\x60\x01\x7F\x00', b'\x60\x00\x01\x7F', b'\x60\x01\x7F\x01\x7F', b'\x60\x00\x00']))
    m += section(2, vec([name('afri') + name('putc') + b'\x00\x00',
                         name('afri') + name('pollkey') + b'\x00\x01',
                         name('afri') + name('sleep_ms') + b'\x00\x02',
                         name('afri') + name('print') + b'\x00\x00',
                         name('afri') + name('time') + b'\x00\x01',
                         name('afri') + name('random') + b'\x00\x01']))
    m += section(3, vec([b'\x03']))
    m += section(5, vec([b'\x00\x01']))
    m += section(7, vec([name('run') + b'\x00\x06']))   # 6 imports, so the function is index 6
    m += section(10, vec([uleb(len(func)) + func]))
    return m


if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(module(body))
        print(n)
