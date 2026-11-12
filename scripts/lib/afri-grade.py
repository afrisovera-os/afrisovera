"""Grade the wasm-afri log (claude/nia-afri, OS_REQUESTS item 8 step A). Usage: afri-grade.py LOG

Cuts the serial log per `nia NAME.wasm` command and checks each app's lines (see afri-wasm.py). Prints one line
per check and exits 1 on any failure.
"""
import re
import sys

REFUSED = '[nia] parse error: import from a module Nia does not offer'
OFFERS = " (Nia offers only 'afri')"
WANT = {
    'af1': ['[nia] parse error: truncated section'],
    'af2': ['[nia] parse error: truncated import name'],
    'af3': ['[nia] parse error: truncated import name'],
    'af4': ["[nia] import from module 'wasi_snapshot_preview1'" + OFFERS, REFUSED],
    'af5': ['[wasm app] print(7) = 0x7'],
    'af6': ['[wasm app] print(7) = 0x7'],
    'af7': ["[nia] import from module 'AFRI'" + OFFERS, REFUSED],
    'af8': ["[nia] import from module 'afrix'" + OFFERS, REFUSED],
    'af9': ["[nia] import from module '\\x1b[2J\\xe2\\x80\\xaeafri'" + OFFERS, REFUSED],
    'af10': ["[nia] import from module '" + 'a' * 32 + "' and 168 more bytes" + OFFERS, REFUSED],
}
KEEP = re.compile(r'\[nia\] (parse error|import from module)|\[wasm app\]|TRAP')


def main():
    raw = open(sys.argv[1], 'rb').read()
    text = raw.decode('utf-8', errors='replace').replace('\r', '')
    lines = text.split('\n')
    bad = 0

    def seg(name):
        out, on = [], False
        for ln in lines:
            if 'afrisovera>' in ln:
                if on:
                    break
                if re.search(r'nia %s\.wasm\s*$' % re.escape(name), ln):
                    on = True
                continue
            if on:
                out.append(ln)
        return out

    for name, want in WANT.items():
        got = [ln.strip() for ln in seg(name) if KEEP.search(ln)]
        ok = got == want
        bad |= not ok
        show = lambda xs: [x.encode('ascii', 'backslashreplace').decode() for x in xs]
        print('  %s %s: %s%s' % ('ok ' if ok else 'BAD', name, show(got), '' if ok else '  want %s' % show(want)))

    start = raw.find(b'nia af9.wasm')
    end = raw.find(b'afrisovera>', start + 1)
    if start < 0 or b'\x1b' in raw[start:end] or b'\xe2\x80\xae' in raw[start:end]:
        print('  BAD af9: a raw ESC or bidi byte from the module name reached the console')
        bad = 1
    else:
        print('  ok  af9: the module name reached the console only as escaped bytes')
    if 'KERNEL PANIC' in text or 'panicked' in text:
        print('  BAD kernel panic')
        bad = 1
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
