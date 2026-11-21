#!/usr/bin/env python3
"""The twelve bundled demos' own permission lists (item 8, step C1).

Each demo array in kernel/src/nia.rs ends with an `afri.permissions` custom section that lists exactly the host
calls it makes. A list only narrows what the app is given, so the demos that test a refusal (hello with no caps,
net without net, rand without random) still get it from the caps they are run with.

  demo-perms.py --check   every demo ends with exactly its list (exit 1 and a line per fault if not)
  demo-perms.py --write   add the list to any demo that lacks one (run once; then sign-app.py WASM_SUM)

Reads nia.rs from the repo root (run from there). Light: no build.
"""
import re
import sys

NIA = 'kernel/src/nia.rs'
DEMOS = [
    ('hello.wasm', 'WASM_STR', ['putc']),
    ('sum.wasm', 'WASM_SUM', ['print']),
    ('draw.wasm', 'WASM_DRAW', ['draw']),
    ('fileapp.wasm', 'WASM_FILE', ['file', 'putc']),
    ('clock.wasm', 'WASM_CLOCK', ['time', 'print']),
    ('type.wasm', 'WASM_TYPE', ['draw', 'input']),
    ('ai.wasm', 'WASM_AI', ['ai', 'putc']),
    ('chat.wasm', 'WASM_CHAT', ['input', 'putc', 'ai']),
    ('notify.wasm', 'WASM_NOTIFY', ['voice']),
    ('notify2.wasm', 'WASM_NOTIFY2', ['voice']),
    ('net.wasm', 'WASM_NET', ['net', 'print']),
    ('rand.wasm', 'WASM_RAND', ['random', 'print']),
]
TOK = re.compile(r"0[xX][0-9A-Fa-f]+|b'\\.'|b'.'|\d+")
ESC = {'n': 10, 't': 9, 'r': 13, '0': 0, '\\': 92, "'": 39}
MARK = '// afri.permissions (item 8, C1): '


def uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def name(s):
    return uleb(len(s)) + s


def section(perms):
    body = name(b'afri.permissions') + uleb(len(perms)) + b''.join(name(p.encode()) for p in perms)
    return b'\x00' + uleb(len(body)) + body


def array(src, arr):
    m = re.search(r"static\s+" + arr + r"\s*:\s*&\[u8\]\s*=\s*&\[(.*?)\n\];", src, re.S)
    if not m:
        raise SystemExit(f'{arr} not found in {NIA}')
    return m


def parse(body):
    out = bytearray()
    for line in body.split('\n'):
        for t in TOK.findall(line.split('//')[0]):
            if t[:2] in ('0x', '0X'):
                out.append(int(t, 16))
            elif t.startswith("b'\\"):
                out.append(ESC[t[3]])
            elif t.startswith("b'"):
                out.append(ord(t[2]))
            else:
                out.append(int(t))
    return bytes(out)


def rust_line(sec):
    parts = []
    i = 0
    while i < len(sec):
        b = sec[i]
        parts.append(f"b'{chr(b)}'" if 0x61 <= b <= 0x7A or b == 0x2E else f'0x{b:02X}')
        i += 1
    return '    ' + ','.join(parts) + ','


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else '--check'
    src = open(NIA, encoding='utf-8', newline='').read()
    bad = 0
    for app, arr, perms in DEMOS:
        m = array(src, arr)
        b = parse(m.group(1))
        sec = section(perms)
        has = b.endswith(sec)
        if mode == '--write' and not has:
            if b.count(b'afri.permissions'):
                raise SystemExit(f'{app}: has a different afri.permissions section; fix it by hand')
            add = '\n    ' + MARK + ', '.join(perms) + '\n' + rust_line(sec)
            src = src[:m.end(1)] + add + src[m.end(1):]
            b = parse(array(src, arr).group(1))
            has = b.endswith(sec)
        if not has:
            print(f'{app} ({arr}): does not end with its list ({", ".join(perms)})')
            bad += 1
        elif b.count(b'afri.permissions') != 1:
            print(f'{app} ({arr}): more than one afri.permissions section')
            bad += 1
        elif len(b) > 512:
            print(f'{app} ({arr}): {len(b)} bytes, over the 512 a copied app may have')
            bad += 1
        else:
            print(f'{app}: {len(b)} bytes, lists {", ".join(perms)}')
    if mode == '--write':
        open(NIA, 'w', encoding='utf-8', newline='').write(src)
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
