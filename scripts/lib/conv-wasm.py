"""Write conv.wasm for the wasm-conv suite (claude/nia-conv, OS_REQUESTS item 4).

  conv-wasm.py OUT.wasm      writes the module and prints, one per line, the lines the kernel must show

Every value is what a standard WebAssembly engine prints. (The 64 KiB edge moved to wasm-oob with claude/nia-addr:
a store16 at 65535 is now a TRAP, as WebAssembly says.)

The module imports afri.print (i32) and afri.print64 (i64), declares one 64 KiB memory and exports run, which
tests i32.wrap_i64 (0xA7), i64.extend_i32_s (0xAC), i64.extend_i32_u (0xAD), i32.load16_s (0x2E),
i32.load16_u (0x2F) and i32.store16 (0x3B) and prints every result, ending with print(7).
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


def vec(items):
    return uleb(len(items)) + b''.join(items)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(s):
    return uleb(len(s)) + s.encode()


I32C = lambda v: b'\x41' + sleb(v)
I64C = lambda v: b'\x42' + sleb(v)
PRINT = b'\x10\x00'
PRINT64 = b'\x10\x01'
WRAP, EXT_S, EXT_U = b'\xA7', b'\xAC', b'\xAD'
ADD = b'\x6A'


def mem(op, align, off=0):
    return bytes([op]) + uleb(align) + uleb(off)


LOAD, STORE = mem(0x28, 2), mem(0x36, 2)
L16S, L16U, S16 = mem(0x2E, 1), mem(0x2F, 1), mem(0x3B, 1)

body = b''
want = []
# wrap: low 32 bits only
body += I64C(0x1_0000_002A) + WRAP + PRINT; want.append('print(42)')
body += I64C(0x7FFF_FFFF_8000_0000) + WRAP + PRINT; want.append('print(-2147483648)')
body += I64C(0x1_0000_0001) + WRAP + I32C(1) + ADD + PRINT; want.append('print(2)')
# extend
body += I32C(-5) + EXT_S + PRINT64; want.append('print64(-5)')
body += I32C(-1) + EXT_U + PRINT64; want.append('print64(4294967295)')
body += I32C(0x7FFF_FFFF) + EXT_S + PRINT64; want.append('print64(2147483647)')
body += I32C(0x7FFF_FFFF) + EXT_U + PRINT64; want.append('print64(2147483647)')
body += I32C(-1) + EXT_S + PRINT64; want.append('print64(-1)')
# store16 / load16: bytes 8..15 = 0xFF, store16 0x1234_8765 at 10 writes 65 87 only
body += I32C(8) + I32C(-1) + STORE
body += I32C(12) + I32C(-1) + STORE
body += I32C(10) + I32C(0x1234_8765) + S16
body += I32C(8) + LOAD + PRINT; want.append('print(-2023358465)')
body += I32C(12) + LOAD + PRINT; want.append('print(-1)')
body += I32C(10) + L16S + PRINT; want.append('print(-30875)')
body += I32C(10) + L16U + PRINT; want.append('print(34661)')
body += I32C(8) + mem(0x2F, 1, 2) + PRINT; want.append('print(34661)')
body += I32C(10) + L16U + EXT_U + PRINT64; want.append('print64(34661)')
body += I32C(7) + PRINT; want.append('print(7)')
body += b'\x0B'

func = vec([]) + body  # no locals
mod = b'\x00asm\x01\x00\x00\x00'
mod += section(1, vec([b'\x60\x01\x7F\x00', b'\x60\x01\x7E\x00', b'\x60\x00\x00']))
mod += section(2, vec([name('afri') + name('print') + b'\x00\x00',
                       name('afri') + name('print64') + b'\x00\x01']))
mod += section(3, vec([b'\x02']))
mod += section(5, vec([b'\x00\x01']))
mod += section(7, vec([name('run') + b'\x00\x02']))
mod += section(10, vec([uleb(len(func)) + func]))

open(sys.argv[1], 'wb').write(mod)
print('\n'.join(want))
