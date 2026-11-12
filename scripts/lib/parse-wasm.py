"""Write the nia-parse overflow apps (claude/nia-parse, item 8 step A2): every length the parser adds is checked.

  parse-wasm.py DIR       writes DIR/NAME.wasm for each app below, one name per line on stdout
  parse-wasm.py --rust    prints the same modules as Rust byte arrays (the copies in nia.rs fuzz(), wasm-fuzz)

Each one carries a 10-byte LEB (2^64 - 1) where the parser reads a length:
  ov1  a type's parameter count       (type section, `c += a + np`)      refused: truncated type
  ov2  a type's result count          (type section, `c += a + nr`)      refused: truncated type
  ov3  an export name's length        (export section, `c + nl`)         refused: truncated export name
  ov4  a code body's size             (code section, `c += bsz`)         refused: truncated code body
  ov5  run's body is one byte, 0x7F: 127 local groups, no bytes for them   TRAP: truncated code body
       (invoke skipped `c += a + 1` per group, then `&body[c..]` was out of range: a panic, not an overflow)
On af690af2 ov1 to ov4 overflowed an unchecked add and ov5 sliced past the body: each a kernel panic.
(The section size and the import name lengths, step A, are wasm-afri's af1 to af3.)
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


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


MAGIC = b'\x00asm\x01\x00\x00\x00'
BIG = b'\xff' * 9 + b'\x01'                       # a 10-byte LEB: 2^64 - 1
PAD = b'\x00' * 8
TYPES = section(1, b'\x01\x60\x00\x00')            # one type: () -> ()
FUNCS = section(3, b'\x01\x00')
EXPORT = section(7, b'\x01\x03run\x00\x00')

APPS = {
    'ov1': MAGIC + section(1, b'\x01\x60' + BIG + PAD),
    'ov2': MAGIC + section(1, b'\x01\x60\x00' + BIG + PAD),
    'ov3': MAGIC + TYPES + FUNCS + section(7, b'\x01' + BIG + PAD),
    'ov4': MAGIC + TYPES + FUNCS + EXPORT + section(10, b'\x01' + BIG + PAD),
    'ov5': MAGIC + TYPES + FUNCS + EXPORT + section(10, b'\x01\x01\x7f'),
}

if __name__ == '__main__':
    if sys.argv[1] == '--rust':
        for n, body in APPS.items():
            print('    let %s: &[u8] = &[%s];' % (n, ', '.join('0x%02X' % b for b in body)))
        sys.exit(0)
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(body)
        print(n)
