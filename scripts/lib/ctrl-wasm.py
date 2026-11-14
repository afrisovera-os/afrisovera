"""Write the control-frame apps (claude/nia-ctrl: typed loop labels, and fuzz finding 2, `else` inside a loop).

  ctrl-wasm.py DIR       writes DIR/NAME.wasm for each app below, one name per line on stdout
  ctrl-wasm.py --rust    prints each app as a Rust byte array (for the fuzz() table in nia.rs)

A loop's label is its start and carries no values (WebAssembly 1.0), whatever its result type. Before the fix
Nia gave a `loop (result i32)` label arity 1, so a branch back moved one value to the loop's base.
  lr1  loop (result i32) { n += 1; br_if 0 (n < 3) with nothing above the base; i32.const 7 } print
       -> print(7). Before the fix: TRAP "value stack underflow at branch" (a valid app stopped).
  lr2  i32.const 5 under the loop, then lr1's loop; print, print -> print(7), print(5). Before the fix the
       branch copied the 5 from under the loop's base into the loop; the value under the loop must come out intact.
  lr3  loop (result i32) { i32.const 42 } falls through its end; print -> print(42), before and after (the
       architect's lr3: a typed loop's result stays when it falls through `end`).

`else` (0x05) jumped to the top frame's cont_pc - 1 whatever the frame was. In a loop that is the loop's start,
so it went back into the body without `branch()`, the only place the Ctrl+C poll runs.
  el1  the fuzzer's own 119-byte input (security-fuzz-findings/nia-else-in-loop.wasm): before, a freeze that
       Ctrl+C cannot stop; after, TRAP "else outside an if".
  el2  loop { else } end: before, a jump back onto the loop's block type byte; after, the same TRAP.
  el3  block { else } end: before, a jump to the block's end (runs on, not valid WebAssembly); after, the TRAP.
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

EL1 = bytes.fromhex(
    '0061736d0100000001080260017f00600000020c0103656e76047075746300000302010105030100010707010372756e0001'
    '0a29012701027f410021000240033220002d000021012001050d0120011000200041016a21000c000b0b0b0b16020c012a41'
    '000b1048454c4c4f2046524f4d205741534d00')


def app(code, locals_i32=0):
    """run: () -> (), imports afri.print (i32) -> () as function 0; `code` is run's body before its final end."""
    loc = (b'\x01' + uleb(locals_i32) + b'\x7F') if locals_i32 else b'\x00'
    body = loc + code + b'\x0B'
    return (MAGIC
            + section(1, b'\x02' + b'\x60\x01\x7F\x00' + b'\x60\x00\x00')
            + section(2, b'\x01' + uleb(4) + b'afri' + uleb(5) + b'print' + b'\x00\x00')
            + section(3, b'\x01\x01')
            + section(7, b'\x01' + uleb(3) + b'run' + b'\x00\x01')
            + section(10, b'\x01' + uleb(len(body)) + body))


PRINT = b'\x10\x00'
# loop (result i32) { local0 += 1; br_if 0 (local0 < 3); i32.const 7 } end
LOOP3 = (b'\x03\x7F'
         + b'\x20\x00\x41\x01\x6A\x22\x00'   # local.get 0; i32.const 1; i32.add; local.tee 0
         + b'\x41\x03\x49\x0D\x00'           # i32.const 3; i32.lt_u; br_if 0
         + b'\x41\x07' + b'\x0B')            # i32.const 7; end

APPS = {
    'lr1': app(LOOP3 + PRINT, 1),
    'lr2': app(b'\x41\x05' + LOOP3 + PRINT + PRINT, 1),
    'lr3': app(b'\x03\x7F\x41\x2A\x0B' + PRINT),
    'el1': EL1,
    'el2': app(b'\x03\x40\x05\x0B'),
    'el3': app(b'\x02\x40\x05\x0B'),
}
assert len(EL1) == 119

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
