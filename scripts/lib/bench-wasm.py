"""Write the Nia speed bench apps (claude/nia-fast, the language and runtime engineer's step 1).

  bench-wasm.py DIR       writes DIR/NAME.wasm for each app below, one name per line on stdout
  bench-wasm.py --rust    prints each app as a Rust byte array (for nia.rs, `wasmbench`)
  bench-wasm.py --expect  prints each app's expected print value

The shapes are what rustc and clang emit for ordinary app code: an `if` and a `block` inside a loop, and a small
function with an `if` called from a loop. Before step 1, Nia scanned forward to the end of every block, loop and
if each time it entered one, so the cost of a loop grew with the size of the blocks inside it.

  b1  loop 20000 times { if (i & 1) { acc += (i * 3) ^ 5 } else { acc -= i }
                         block { br_if 0 ((i & 7) == 0); acc += 1 } }   print(acc)
  b2  loop 5000 times { acc += f(i) }, f(x) = if (x & 1) { x + 3 } else { x - 1 }   print(acc)
"""
import os
import sys

B1_N = 20000
B2_N = 5000


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


def const(v):
    return b'\x41' + sleb(v)


MAGIC = b'\x00asm\x01\x00\x00\x00'
GET, SET = b'\x20', b'\x21'
I, ACC = b'\x00', b'\x01'


def module(funcs):
    """funcs: list of (type index, body with locals, before its end). Type 0 is (i32)->(), the afri.print
    import; type 1 is ()->(); type 2 is (i32)->(i32). The last function is `run`."""
    code = b''.join(uleb(len(b) + 1) + b + b'\x0B' for _, b in funcs)
    run = 1 + len(funcs) - 1
    return (MAGIC
            + section(1, b'\x03' + b'\x60\x01\x7F\x00' + b'\x60\x00\x00' + b'\x60\x01\x7F\x01\x7F')
            + section(2, b'\x01' + uleb(4) + b'afri' + uleb(5) + b'print' + b'\x00\x00')
            + section(3, uleb(len(funcs)) + b''.join(uleb(t) for t, _ in funcs))
            + section(7, b'\x01' + uleb(3) + b'run' + b'\x00' + uleb(run))
            + section(10, uleb(len(funcs)) + code))


def loop_of(n, body):
    # i = 0; block { loop { br_if 1 (i >= n); body; i += 1; br 0 } }
    return (const(0) + SET + I
            + b'\x02\x40' + b'\x03\x40'
            + GET + I + const(n) + b'\x4E' + b'\x0D\x01'
            + body
            + GET + I + const(1) + b'\x6A' + SET + I
            + b'\x0C\x00' + b'\x0B' + b'\x0B'
            + GET + ACC + b'\x10\x00')


def b1():
    body = (GET + I + const(1) + b'\x71'
            + b'\x04\x40'
            + GET + ACC + GET + I + const(3) + b'\x6C' + const(5) + b'\x73' + b'\x6A' + SET + ACC
            + b'\x05'
            + GET + ACC + GET + I + b'\x6B' + SET + ACC
            + b'\x0B'
            + b'\x02\x40'
            + GET + I + const(7) + b'\x71' + b'\x45' + b'\x0D\x00'
            + GET + ACC + const(1) + b'\x6A' + SET + ACC
            + b'\x0B')
    return module([(1, b'\x01\x02\x7F' + loop_of(B1_N, body))])


def b2():
    f = (b'\x00' + GET + b'\x00' + const(1) + b'\x71'
         + b'\x04\x7F' + GET + b'\x00' + const(3) + b'\x6A'
         + b'\x05' + GET + b'\x00' + const(1) + b'\x6B' + b'\x0B')
    body = GET + ACC + GET + I + b'\x10\x01' + b'\x6A' + SET + ACC
    return module([(2, f), (1, b'\x01\x02\x7F' + loop_of(B2_N, body))])


def i32(v):
    v &= 0xFFFFFFFF
    return v - (1 << 32) if v & 0x80000000 else v


def expect_b1():
    acc = 0
    for i in range(B1_N):
        acc = i32(acc + ((i * 3) ^ 5)) if i & 1 else i32(acc - i)
        if i & 7:
            acc = i32(acc + 1)
    return acc


def expect_b2():
    acc = 0
    for i in range(B2_N):
        acc = i32(acc + (i + 3 if i & 1 else i - 1))
    return acc


APPS = {'b1': (b1, expect_b1), 'b2': (b2, expect_b2)}


def main():
    if len(sys.argv) == 2 and sys.argv[1] == '--rust':
        for name, (make, _) in APPS.items():
            data = make()
            print('#[rustfmt::skip]')
            print('static WASM_BENCH_%s: [u8; %d] = [' % (name.upper(), len(data)))
            for k in range(0, len(data), 16):
                print('    ' + ', '.join('0x%02X' % b for b in data[k:k + 16]) + ',')
            print('];')
        return
    if len(sys.argv) == 2 and sys.argv[1] == '--expect':
        for name, (_, exp) in APPS.items():
            print(name, exp())
        return
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    os.makedirs(sys.argv[1], exist_ok=True)
    for name, (make, _) in APPS.items():
        with open(os.path.join(sys.argv[1], name + '.wasm'), 'wb') as fh:
            fh.write(make())
        print(name)


if __name__ == '__main__':
    main()
