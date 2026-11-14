"""Grade the wasm-perms log (claude/nia-perms, OS_REQUESTS item 8 step B). Usage: perms-grade.py LOG

Cuts the serial log per `nia NAME.wasm` command and compares each app's permission, parse, print, DENIED and TRAP
lines with what the kernel must print (see perms-wasm.py). Prints one line per check, exits 1 on any failure.
"""
import re
import sys


def listed(asked, granted, withheld, *rest):
    return ['[nia] permissions asked: ' + asked, '[nia] permissions granted: ' + granted,
            '[nia] permissions withheld: ' + withheld] + list(rest)


P = lambda n: '[wasm app] print(%d) = 0x%x' % (n, n)
DENIED = lambda h: "[nia] DENIED: app called host '%s' without the capability" % h
TRAP = '[nia] TRAP: capability denied'
WANT = {
    'p1': listed('print', 'print', 'nothing', P(1), DENIED('time'), TRAP),
    'p2': listed('print, time', 'print, time', 'nothing', P(2), P(3)),
    'p3': listed('print, net', 'print', 'net', P(4), DENIED('net'), TRAP),
    'p4': ["[nia] unknown permission 'teleport'", '[nia] parse error: unknown permission'],
    'p5': ['[nia] parse error: two afri.permissions sections'],
    'p6': ['[nia] parse error: bad afri.permissions section'],
    'p7': ['[nia] permissions: none listed; granted by kind (v0)', P(7)],
    'p8': listed('print', 'print', 'nothing', P(8)),
    'p9': listed('print', 'print', 'nothing', P(9), DENIED('time'), TRAP),
    'p10': ['[nia] parse error: too many permissions (Nia holds 16)'],
    'p11': ['[nia] parse error: bad afri.permissions section'],
}
KEEP = re.compile(r'\[nia\] (permissions|parse error|unknown permission|DENIED|TRAP)|\[wasm app\]')


def main():
    text = open(sys.argv[1], 'rb').read().decode('utf-8', errors='replace').replace('\r', '')
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
        print('  %s %s: %s%s' % ('ok ' if ok else 'BAD', name, got, '' if ok else '  want %s' % want))
    if 'KERNEL PANIC' in text or 'panicked' in text:
        print('  BAD kernel panic')
        bad = 1
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
