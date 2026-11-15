#!/usr/bin/env python3
"""Write tone.wasm, the bundled sound demo and test app for N-A1a (claude/nia-audio, NIA_AUDIO_PLAN.md).

  audio-wasm.py DIR            writes DIR/tone.wasm
  audio-wasm.py --rust         prints tone.wasm as the Rust array nia.rs bundles (WASM_TONE)

A disk app never gets the audio permission (CAP_SIDELOAD), so the sound tests run the bundled copy; a disk copy of
the same bytes under another name is the permission test. tone.wasm asks for print, input, draw, time, voice and
audio. It plays square waves (amplitude 8000) and is driven by one key:

Without a desk (cell_open gives -4), the first key from getkey picks the mode:
  a  22,050 Hz mono: 0.5 s at 441 Hz, a 500 ms pause with nothing written, 0.5 s more, audio_close; prints 1
  b  48,000 Hz stereo: 1 s at 480 Hz on the left, the right silent, audio_close; prints 2
  e  the edges, each result printed: open(7999,1) open(48001,1) open(22050,0) open(22050,3) (all -3), open(22050,1)
     (1), a second open (-1), five writes of 100,000 frames (1378 four times: a quarter of 5512, then 0, full), a
     write to stream 9 (-1), the level (5512), audio_close (0), the level after it (-1)
  t  open, 1378 frames, then a write at ptr 65,530 of 30 frames: TRAP out of bounds (it never prints 99)
  c  open, 1378 frames, then a loop with no waits for ever (Ctrl+C stops it)
  r  open, 4410 frames (200 ms), then return with them queued; prints 4
  s  open, speak "busy" (the voice is busy), a second open (prints -1), audio_close, speak "free"; prints 5
  n  prints audio_open(22050, 1): -2 with no sound device; DENIED for a disk copy
With the desk (cell_open gives 1): the first key event picks the mode, then it keeps about 125 ms queued, polling
cell_event(20):
  w  a close request (kind 3) makes it return
  i  it ignores close requests; Close anyway stops it at its next loop step
"""
import sys

I32 = 0x7F
END = b'\x0B'
DROP = b'\x1A'


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


def name(b):
    return uleb(len(b)) + b


def c(v):
    return b'\x41' + sleb(v)


def call(f):
    return b'\x10' + uleb(f)


def lg(i):
    return b'\x20' + uleb(i)


def ls(i):
    return b'\x21' + uleb(i)


def functype(params, results):
    return b'\x60' + uleb(len(params)) + bytes(params) + uleb(len(results)) + bytes(results)


BLOCK, LOOP, IF, ELSE = b'\x02\x40', b'\x03\x40', b'\x04\x40', b'\x05'
BR, BR_IF, RET = b'\x0C', b'\x0D', b'\x0F'
ADD, SUB, MUL, REM_U, SHL, SHR_U, AND = b'\x6A', b'\x6B', b'\x6C', b'\x70', b'\x74', b'\x76', b'\x71'
EQ, LT_S, LT_U, GE_S, GE_U, LE_S = b'\x46', b'\x48', b'\x49', b'\x4E', b'\x4F', b'\x4C'
STORE16 = b'\x3B\x01'  # align 2^1, then the offset
GGET, GSET = b'\x23\x00', b'\x24\x00'

# types
T_V_I, T_I, T_II_I, T_III_I, T_I_I, T_II_V, T_VV, T_III_V, T_IIII_V = range(9)
TYPES = [functype([I32], []), functype([], [I32]), functype([I32] * 2, [I32]), functype([I32] * 3, [I32]),
         functype([I32], [I32]), functype([I32] * 2, []), functype([], []), functype([I32] * 3, []),
         functype([I32] * 4, [])]
IMPORTS = [(b'print', T_V_I), (b'getkey', T_I), (b'audio_open', T_II_I), (b'audio_write', T_III_I),
           (b'audio_level', T_I_I), (b'audio_close', T_I_I), (b'sleep_ms', T_I_I), (b'speak', T_II_V),
           (b'cell_open', T_II_I), (b'cell_event', T_I_I)]
PRINT, GETKEY, OPEN, WRITE, LEVEL, CLOSE, SLEEP, SPEAK, CELL_OPEN, CELL_EVENT = range(10)
RUN, FILL, PLAY, DESK = 10, 11, 12, 13
BUF = 1024  # the sample buffer; "busy" at 0 and "free" at 8 (the data section)
PERMS = [b'print', b'input', b'draw', b'time', b'voice', b'audio']


def p(expr):
    return expr + call(PRINT)


# fill(n, period, ch): n frames of the square wave at BUF (the right channel silent when ch is 2); global 0 is the
# phase. Locals: 3 i, 4 v, 5 addr.
FILL_BODY = (b'\x01\x03\x7F' + c(0) + ls(3) + BLOCK + LOOP
             + lg(3) + lg(0) + GE_U + BR_IF + uleb(1)
             + c(-8000) + ls(4)
             + GGET + lg(1) + REM_U + lg(1) + c(1) + SHR_U + LT_U + IF + c(8000) + ls(4) + END
             + c(BUF) + lg(3) + lg(2) + MUL + c(1) + SHL + ADD + ls(5)
             + lg(5) + lg(4) + STORE16 + uleb(0)
             + lg(2) + c(2) + EQ + IF + lg(5) + c(0) + STORE16 + uleb(2) + END
             + GGET + c(1) + ADD + GSET
             + lg(3) + c(1) + ADD + ls(3)
             + BR + uleb(0) + END + END + END)

# play(id, total, period, ch): write `total` frames in 512-frame parts; sleep_ms(10) whenever the stream is full.
# Locals: 4 n, 5 off, 6 t.
PLAY_BODY = (b'\x01\x03\x7F' + BLOCK + LOOP
             + lg(1) + c(0) + LE_S + BR_IF + uleb(1)
             + c(512) + ls(4) + lg(1) + c(512) + LT_S + IF + lg(1) + ls(4) + END
             + lg(4) + lg(2) + lg(3) + call(FILL)
             + c(0) + ls(5) + BLOCK + LOOP
             + lg(5) + lg(4) + GE_S + BR_IF + uleb(1)
             + lg(0) + c(BUF) + lg(5) + lg(3) + MUL + c(1) + SHL + ADD + lg(4) + lg(5) + SUB + call(WRITE) + ls(6)
             + lg(5) + lg(6) + ADD + ls(5)
             + lg(6) + b'\x45' + IF + c(10) + call(SLEEP) + DROP + END
             + BR + uleb(0) + END + END
             + lg(1) + lg(4) + SUB + ls(1)
             + BR + uleb(0) + END + END + END)

# desk(): the stream, the first key event picks w or i, then keep about 125 ms queued while polling cell_event(20).
# Locals: 0 id, 1 mode, 2 e, 3 kind.
DESK_BODY = (b'\x01\x04\x7F' + c(22050) + c(1) + call(OPEN) + ls(0)
             + c(-1) + call(CELL_EVENT) + c(0x0FFFFFFF) + AND + ls(1)
             + LOOP
             + c(20) + call(CELL_EVENT) + ls(2)
             + lg(2) + c(28) + SHR_U + c(7) + AND + ls(3)
             + lg(3) + c(3) + EQ + lg(1) + c(ord('w')) + EQ + AND + IF + RET + END
             + lg(0) + call(LEVEL) + c(2756) + LT_S + IF + lg(0) + c(1378) + c(50) + c(1) + call(PLAY) + END
             + BR + uleb(0) + END + END)


def mode(key, body):
    return lg(0) + c(ord(key)) + EQ + IF + body + RET + END


ID = 1  # run's local 1: the stream id
OPEN_22 = c(22050) + c(1) + call(OPEN) + ls(ID)
RUN_BODY = (b'\x01\x02\x7F'
            + c(20) + c(4) + call(CELL_OPEN) + c(1) + EQ + IF + call(DESK) + RET + END
            + call(GETKEY) + ls(0)
            + mode('a', OPEN_22 + lg(ID) + c(11025) + c(50) + c(1) + call(PLAY) + c(500) + call(SLEEP) + DROP
                   + lg(ID) + c(11025) + c(50) + c(1) + call(PLAY) + lg(ID) + call(CLOSE) + DROP + p(c(1)))
            + mode('b', c(48000) + c(2) + call(OPEN) + ls(ID) + lg(ID) + c(48000) + c(100) + c(2) + call(PLAY)
                   + lg(ID) + call(CLOSE) + DROP + p(c(2)))
            + mode('e', p(c(7999) + c(1) + call(OPEN)) + p(c(48001) + c(1) + call(OPEN))
                   + p(c(22050) + c(0) + call(OPEN)) + p(c(22050) + c(3) + call(OPEN))
                   + OPEN_22 + p(lg(ID)) + p(c(22050) + c(1) + call(OPEN))
                   + p(lg(ID) + c(BUF) + c(100000) + call(WRITE)) * 5
                   + p(c(9) + c(BUF) + c(10) + call(WRITE))
                   + p(lg(ID) + call(LEVEL)) + p(lg(ID) + call(CLOSE)) + p(lg(ID) + call(LEVEL)))
            + mode('t', OPEN_22 + lg(ID) + c(1378) + c(50) + c(1) + call(PLAY)
                   + lg(ID) + c(65530) + c(30) + call(WRITE) + DROP + p(c(99)))
            + mode('c', OPEN_22 + lg(ID) + c(1378) + c(50) + c(1) + call(PLAY) + LOOP + BR + uleb(0) + END)
            + mode('r', OPEN_22 + lg(ID) + c(4410) + c(50) + c(1) + call(PLAY) + p(c(4)))
            + mode('s', OPEN_22 + c(0) + c(4) + call(SPEAK) + p(c(22050) + c(1) + call(OPEN))
                   + lg(ID) + call(CLOSE) + DROP + c(8) + c(4) + call(SPEAK) + p(c(5)))
            + mode('n', p(c(22050) + c(1) + call(OPEN)))
            + END)


def tone():
    bodies = [RUN_BODY, FILL_BODY, PLAY_BODY, DESK_BODY]
    ftypes = [T_VV, T_III_V, T_IIII_V, T_VV]
    code = uleb(len(bodies)) + b''.join(uleb(len(x)) + x for x in bodies)
    data = b'busy\x00\x00\x00\x00free'
    return (b'\x00asm\x01\x00\x00\x00'
            + section(1, uleb(len(TYPES)) + b''.join(TYPES))
            + section(2, uleb(len(IMPORTS)) + b''.join(name(b'afri') + name(n) + b'\x00' + uleb(t)
                                                        for n, t in IMPORTS))
            + section(3, uleb(len(ftypes)) + b''.join(uleb(t) for t in ftypes))
            + section(5, b'\x01\x00\x01')
            + section(6, b'\x01\x7F\x01' + c(0) + END)
            + section(7, b'\x01' + name(b'run') + b'\x00' + uleb(RUN))
            + section(10, code)
            + section(11, b'\x01\x00' + c(0) + END + uleb(len(data)) + data)
            + section(0, name(b'afri.permissions') + uleb(len(PERMS)) + b''.join(name(x) for x in PERMS)))


def main():
    t = tone()
    if sys.argv[1:] == ['--rust']:
        print('// scripts/lib/audio-wasm.py --rust (%d bytes)' % len(t))
        print('static WASM_TONE: [u8; %d] = [' % len(t))
        for k in range(0, len(t), 16):
            print('    ' + ' '.join('0x%02X,' % b for b in t[k:k + 16]))
        print('];')
        return
    import os
    os.makedirs(sys.argv[1], exist_ok=True)
    open(os.path.join(sys.argv[1], 'tone.wasm'), 'wb').write(t)
    print('tone.wasm %d bytes' % len(t))


if __name__ == '__main__':
    main()
