"""Write the F1 test app (claude/nia-fast): an app that rewrites its own file while it runs.

  selfwrite-wasm.py DIR       writes DIR/sw1.wasm, prints its name
  selfwrite-wasm.py --rust    prints it as a Rust byte array (fuzz-host/tests/selfwrite.rs)

Finding F1 (NIA_FAST_DESIGN.md): run_file ran an app straight out of the VFS file table, and file_write writes that
table in place. sw1 holds `file` (a bundled app's grant) and does:
  file_write("fuzz.wasm", 512 zero bytes)   its own file, as the fuzz-host harness names the app
  print(42)
  block { print(7) }
After the write, the code the kernel was reading is all zeros (0x00 is `unreachable`). Before the fix: the run
ends in "TRAP: unreachable" right after the write. After the fix (Nia runs its own copy, the one parse checked):
print(42), print(7), "'run' returned cleanly".
"""
import os
import sys

NAME = b'fuzz.wasm'


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


def imp(name, t):
    return uleb(4) + b'afri' + uleb(len(name)) + name + b'\x00' + uleb(t)


def sw1():
    types = (b'\x03' + b'\x60\x01\x7F\x00'                    # 0: (i32) -> ()        print
             + b'\x60\x04\x7F\x7F\x7F\x7F\x01\x7F'             # 1: (i32 x4) -> i32    file_write
             + b'\x60\x00\x00')                                # 2: () -> ()           run
    imports = b'\x02' + imp(b'print', 0) + imp(b'file_write', 1)
    code = (b'\x41\x00' + b'\x41' + uleb(len(NAME)) + b'\x41\xC0\x00' + b'\x41\x80\x04'  # 0, len, 64, 512
            + b'\x10\x01' + b'\x1A'                            # call file_write; drop
            + b'\x41\x2A' + b'\x10\x00'                        # print(42)
            + b'\x02\x40' + b'\x41\x07' + b'\x10\x00' + b'\x0B'  # block { print(7) }
            + b'\x0B')
    body = b'\x00' + code
    data = b'\x01' + b'\x00' + b'\x41\x00\x0B' + uleb(len(NAME)) + NAME
    return (b'\x00asm\x01\x00\x00\x00'
            + section(1, types)
            + section(2, imports)
            + section(3, b'\x01\x02')
            + section(5, b'\x01\x00\x01')
            + section(7, b'\x01' + uleb(3) + b'run' + b'\x00\x02')
            + section(10, b'\x01' + uleb(len(body)) + body)
            + section(11, data))


def main():
    data = sw1()
    if len(sys.argv) == 2 and sys.argv[1] == '--rust':
        print('const SW1: [u8; %d] = [' % len(data))
        for k in range(0, len(data), 16):
            print('    ' + ', '.join('0x%02X' % b for b in data[k:k + 16]) + ',')
        print('];')
        return
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    os.makedirs(sys.argv[1], exist_ok=True)
    with open(os.path.join(sys.argv[1], 'sw1.wasm'), 'wb') as fh:
        fh.write(data)
    print('sw1')


if __name__ == '__main__':
    main()
