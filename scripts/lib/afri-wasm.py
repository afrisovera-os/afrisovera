"""Write the wasm-afri apps (claude/nia-afri, OS_REQUESTS item 8 step A): imports come from module `afri` (or
`env`, v0), and the parser's lengths are checked adds.

  afri-wasm.py DIR     writes DIR/NAME.wasm for each app below, one name per line on stdout

Each normal app imports `print` from the module named below and calls print(7).
  af1   magic, then section 1 whose size is a 10-byte LEB (2^64 - 1)        refused: truncated section
  af2   an import whose module-name length is a 10-byte LEB               refused: truncated import name
  af3   an import whose field-name length is a 10-byte LEB                refused: truncated import name
  af4   module 'wasi_snapshot_preview1'                                   refused, the name shown
  af5   module 'env'                                                       runs: print(7)
  af6   module 'afri'                                                      runs: print(7)
  af7   module 'AFRI'                                                      refused (exact bytes)
  af8   module 'afrix'                                                     refused (no prefix match)
  af9   module ESC '[2J' U+202E 'afri'                                     refused, shown as escaped bytes
  af10  module of 200 'a'                                                  refused, 32 bytes shown
On the old kernel af1 to af3 overflowed an unchecked add (a kernel panic) and af4 to af10 all ran.
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


def vec(items):
    return uleb(len(items)) + b''.join(items)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(bs):
    return uleb(len(bs)) + bs


MAGIC = b'\x00asm\x01\x00\x00\x00'
BIG = b'\xff' * 9 + b'\x01'            # a 10-byte LEB: 2^64 - 1
TYPES = section(1, vec([b'\x60\x00\x00', b'\x60\x01\x7F\x00']))   # 0: () -> (), 1: (i32) -> ()


def app(module):
    m = MAGIC + TYPES
    m += section(2, vec([name(module) + name(b'print') + b'\x00\x01']))
    m += section(3, vec([b'\x00']))
    m += section(7, vec([name(b'run') + b'\x00\x01']))
    body = vec([]) + b'\x41\x07\x10\x00\x0B'      # print(7)
    m += section(10, vec([uleb(len(body)) + body]))
    return m


APPS = {
    'af1': MAGIC + b'\x01' + BIG + b'\x00' * 4,
    'af2': MAGIC + TYPES + section(2, b'\x01' + BIG + b'\x00' * 8),
    'af3': MAGIC + TYPES + section(2, b'\x01' + name(b'afri') + BIG + b'\x00' * 8),
    'af4': app(b'wasi_snapshot_preview1'),
    'af5': app(b'env'),
    'af6': app(b'afri'),
    'af7': app(b'AFRI'),
    'af8': app(b'afrix'),
    'af9': app(b'\x1b[2J\xe2\x80\xaeafri'),
    'af10': app(b'a' * 200),
}

if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(body)
        print(n)
