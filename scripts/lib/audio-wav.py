#!/usr/bin/env python3
"""Read a QEMU wav capture and describe its sound (claude/nia-audio, the wasm-audio suite).

  audio-wav.py FILE      prints "wav: rate=R ch=C frames=N" then one line per stretch of sound:
                         "seg: start_ms=S ms=D hz=H peak=P" (the left channel; a stretch ends at 5 ms of quiet)
                         and "quiet: ms=Q" between stretches; or "error=<why>"

Sound is any sample with |s| > 1000. A square wave at +-8000 (or +-4000) never stays quiet for 5 ms, so a stretch
of quiet that long inside a tone is a gap the player left. The frequency is zero crossings / 2 / seconds.
"""
import struct
import sys

LOUD = 1000


def main():
    try:
        d = open(sys.argv[1], "rb").read()
    except OSError as e:
        print("error=%s" % e)
        return
    if d[:4] != b"RIFF" or d[8:12] != b"WAVE":
        print("error=not-a-wav")
        return
    i, rate, ch, bits, data = 12, 0, 0, 0, None
    while i + 8 <= len(d):
        cid, n = d[i:i + 4], struct.unpack("<I", d[i + 4:i + 8])[0]
        if cid == b"data" and (n == 0 or i + 8 + n > len(d)):
            n = len(d) - i - 8  # QEMU writes the data size at exit: read what is in the file
        body = d[i + 8:i + 8 + n]
        if cid == b"fmt ":
            _, ch, rate, _, _, bits = struct.unpack("<HHIIHH", body[:16])
        elif cid == b"data":
            data = body
        i += 8 + n + (n & 1)
    if data is None or bits != 16 or ch < 1:
        print("error=unexpected-format")
        return
    frames = len(data) // (2 * ch)
    left = struct.unpack("<%dh" % (frames * ch), data[:frames * 2 * ch])[::ch]
    print("wav: rate=%d ch=%d frames=%d" % (rate, ch, frames))
    gap = rate * 5 // 1000
    segs, start, quiet, last_loud = [], None, 0, -1
    for k, s in enumerate(left):
        if abs(s) > LOUD:
            if start is None:
                start = k
            last_loud = k
        elif start is not None and k - last_loud >= gap:
            segs.append((start, last_loud + 1))
            start = None
    if start is not None:
        segs.append((start, last_loud + 1))
    prev_end = None
    for a, b in segs:
        if prev_end is not None:
            print("quiet: ms=%d" % ((a - prev_end) * 1000 // rate))
        part = left[a:b]
        cross = sum(1 for x, y in zip(part, part[1:]) if (x < 0) != (y < 0))
        secs = len(part) / rate
        print("seg: start_ms=%d ms=%d hz=%d peak=%d" % (a * 1000 // rate, len(part) * 1000 // rate,
                                                        round(cross / 2 / secs) if secs else 0,
                                                        max(abs(x) for x in part)))
        prev_end = b


if __name__ == "__main__":
    main()
