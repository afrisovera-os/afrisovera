"""Write the br_table count apps (claude/nia-brtable, fuzz finding 1 from the security engineer's fuzz-host).

  brtable-wasm.py DIR       writes DIR/NAME.wasm for each app below, one name per line on stdout
  brtable-wasm.py --rust    prints each app as a Rust byte array (for the fuzz() table in nia.rs)

br_table (0x0E) carries a label count read from the app. Before the fix two loops ran that many times reading
labels past the body's end without moving, so a huge count froze the kernel (no loop poll, Ctrl+C could not stop
it), and a 10-byte count of 2^64 - 1 overflowed `count + 1` (a debug-kernel panic).
  bt1  the fuzzer's own 113-byte input: print(5), then br_table with a count of about 2^42 (the interpreter's loop)
  bt2  run = block { i32.const 0; br_table with a 10-byte count 2^64 - 1 } (instr_end, entering the block)
  bt3  run = block { i32.const 0; br_table with a count of about 2^42 } (instr_end's loop)
Each must end in `[nia] TRAP: truncated br_table`, and the shell must answer after it.
"""
import os
import sys


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


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


MAGIC = b'\x00asm\x01\x00\x00\x00'
BIG = bytes.fromhex('98a8c0e0808101')            # the fuzzer's count, about 2^42
HUGE = bytes.fromhex('ffffffffffffffffff01')      # 2^64 - 1, 10 bytes

BT1 = bytes.fromhex(
    '0061736d01000000010c0360017f0060017e00600000021d020461667269057072696e74000004616672690770'
    '72696e74363400010302010205030100010707010372756e00020a270125004105100041f8ff0342883d0e98a8'
    'c0e080810137030041f8ff03290300100141e30010000b')


def blocked(count):
    """run: () -> (), no imports; body = block { i32.const 0; br_table COUNT, label 0; } end."""
    body = b'\x00' + b'\x02\x40' + b'\x41\x00' + b'\x0E' + count + b'\x00' + b'\x0B' + b'\x0B'
    return (MAGIC + section(1, b'\x01\x60\x00\x00') + section(3, b'\x01\x00')
            + section(7, b'\x01' + uleb(3) + b'run' + b'\x00\x00')
            + section(10, b'\x01' + uleb(len(body)) + body))


APPS = {'bt1': BT1, 'bt2': blocked(HUGE), 'bt3': blocked(BIG)}
assert len(BT1) == 113

if __name__ == '__main__':
    if sys.argv[1] == '--rust':
        for n, b in APPS.items():
            print('    let %s: &[u8] = &[%s];' % (n, ', '.join('0x%02X' % x for x in b)))
    else:
        d = sys.argv[1]
        os.makedirs(d, exist_ok=True)
        for n, b in APPS.items():
            open(os.path.join(d, n + '.wasm'), 'wb').write(b)
            print(n)
