"""Grade the wasm-putc log (claude/nia-putc). Usage: putc-grade.py LOG

Reads the serial log as UTF-8, cuts it per `nia NAME.wasm` command (up to the next prompt), and compares each
app's output lines (the lines that start with "[wasm app] " and the [nia] TRAP / returned lines) with what the
new putc printer must print. Prints one line per check and exits 1 on any failure.
"""
import re
import sys

FFFD = '�'
WANT = {
    'pu1': ['Hi!'],
    'pu2': [FFFD + 'A' + FFFD, 'é?[2J?', 'x??y'],
    'pu3': ['before', 'TRAP'],
    'pu4': ['.' * 300],
    'pu5': ['ok' + FFFD],
    'pu6': ['a', 'print(5) = 0x5', 'b'],
    'pu7': ['t=x'],
    'sa': ['abcde' + FFFD, 'TRAP'],
    'sb': ['hi'],
}


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
        got = []
        for ln in seg(name):
            m = re.match(r'^ {4}\[wasm app\] (.*)$', ln)
            if m:
                got.append(m.group(1))
            elif '[nia] TRAP' in ln:
                got.append('TRAP')
        ok = got == want
        bad |= not ok
        show = lambda xs: [x.encode('ascii', 'backslashreplace').decode() for x in xs]
        print('  %s %s: %s%s' % ('ok ' if ok else 'BAD', name, show(got), '' if ok else '  want %s' % show(want)))

    # pu2's segment holds no raw ESC or BEL byte (the printer turned them into ?)
    start = raw.find(b'nia pu2.wasm')
    end = raw.find(b'afrisovera>', start + 1)
    if start < 0 or b'\x1b' in raw[start:end] or b'\x07' in raw[start:end]:
        print('  BAD pu2: a raw ESC or BEL byte reached the console')
        bad = 1
    else:
        print('  ok  pu2: no raw ESC or BEL byte in its output')
    # pu1: "Hi" was on the console before the key was typed (poll-keys' ?Hi step)
    if "poll-keys: saw 'Hi': yes" in text:
        print('  ok  pu1: "Hi" was shown before the key')
    else:
        print('  BAD pu1: "Hi" was not shown before the key')
        bad = 1
    if 'KERNEL PANIC' in text or 'panicked' in text:
        print('  BAD kernel panic')
        bad = 1
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
