"""Grade the wasm-time log (claude/nia-time, OS_REQUESTS item 7). Usage: time-grade.py LOG

The suite types, in one boot whose clock QEMU started at 2026-01-01 12:34:56 UTC (epoch 1767270896):
  date, nia tm1.wasm, date, ask the time, mem
and this script checks, against the same boot's own `date` (the architect's condition 1):
  - the first `date` is within 900 s after 1767270896 (the -rtc base took effect);
  - tm1's first `time` is 0 to 10 s after the first `date`;
  - tm1's second `time`, after sleep_ms(2000), is 2 to 5 s after its first (condition 2);
  - the clock answer of `ask the time` is 0 to 10 s after the second `date`;
  - no TRAP, DENIED or panic.
Prints one line per check and exits 1 on any failure.
"""
import re
import sys

BASE = 1767270896


def main():
    text = open(sys.argv[1], 'rb').read().decode('utf-8', errors='replace').replace('\r', '')
    segs = []   # (command, [lines])
    for ln in text.split('\n'):
        m = re.search(r'afrisovera> (\S.*?)\s*$', ln)
        if m:
            segs.append((m.group(1), []))
        elif segs:
            segs[-1][1].append(ln)

    def find(cmd, nth=0):
        hits = [ls for c, ls in segs if c == cmd]
        return hits[nth] if len(hits) > nth else []

    def ints(lines, pat):
        return [int(m.group(1)) for ln in lines for m in [re.search(pat, ln)] if m]

    bad = 0

    def check(ok, what):
        nonlocal bad
        print('  %s %s' % ('ok ' if ok else 'BAD', what))
        bad |= not ok

    d1 = ints(find('date', 0), r'\(epoch (\d+)\)')
    d2 = ints(find('date', 1), r'\(epoch (\d+)\)')
    tm = find('nia tm1.wasm')
    t = ints(tm, r'\[wasm app\] print\((-?\d+)\)')
    a = ints(find('ask the time'), r'THE ON-DEVICE CLOCK READS (-?\d+) SECONDS')
    print('  date %s, tm1 %s, date %s, ask %s' % (d1, t, d2, a))

    check(len(d1) == 1 and BASE <= d1[0] <= BASE + 900, 'the first date is within 900 s after %d' % BASE)
    check(len(t) == 2, 'tm1 printed two values')
    if len(t) == 2 and d1:
        check(0 <= t[0] - d1[0] <= 10, 'tm1 time() is 0 to 10 s after date (%d s)' % (t[0] - d1[0]))
        check(2 <= t[1] - t[0] <= 5, 'tm1 second time() is 2 to 5 s after the first (%d s)' % (t[1] - t[0]))
    check(len(a) == 1 and len(d2) == 1 and 0 <= a[0] - d2[0] <= 10,
          'ask the time is 0 to 10 s after the second date')
    check(not any('TRAP' in ln or 'DENIED' in ln for ln in tm), 'tm1 ran without TRAP or DENIED')
    check('KERNEL PANIC' not in text and 'panicked' not in text, 'no kernel panic')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
