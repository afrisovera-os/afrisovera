#!/usr/bin/env python3
"""Assemble Nia demo WASM modules and print them as Rust byte arrays.

Generated so multi-arg host calls, 24-bit colour constants, and control flow get
correct LEB128 encoding without hand-assembly.
"""


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
        if (n == 0 and not (b & 0x40)) or (n == -1 and (b & 0x40)):
            out.append(b)
            return bytes(out)
        out.append(b | 0x80)


def vec(items):
    return uleb(len(items)) + b"".join(items)


def section(sid, content):
    return bytes([sid]) + uleb(len(content)) + content


I32 = 0x7F


def functype(params, results):
    return b"\x60" + uleb(len(params)) + bytes(params) + uleb(len(results)) + bytes(results)


def imp(mod, name, tidx):
    m, n = mod.encode(), name.encode()
    return uleb(len(m)) + m + uleb(len(n)) + n + b"\x00" + uleb(tidx)


def export(name, idx):
    n = name.encode()
    return uleb(len(n)) + n + b"\x00" + uleb(idx)


def data_seg(offset, blob):
    return b"\x00" + b"\x41" + sleb(offset) + b"\x0B" + uleb(len(blob)) + blob


# instruction emitters
def i32c(n):
    return b"\x41" + sleb(n)


def lget(i):
    return b"\x20" + uleb(i)


def lset(i):
    return b"\x21" + uleb(i)


def call(i):
    return b"\x10" + uleb(i)


LOAD8U = b"\x2D\x00\x00"
ADD = b"\x6A"
GES = b"\x4E"
DROP = b"\x1A"
BLOCK = b"\x02\x40"
LOOP = b"\x03\x40"
END = b"\x0B"


def br(l):
    return b"\x0C" + uleb(l)


def brif(l):
    return b"\x0D" + uleb(l)


def emit(name, module):
    print("#[rustfmt::skip]")
    print("static %s: &[u8] = &[" % name)
    for i in range(0, len(module), 12):
        print("    " + " ".join("0x%02X," % b for b in module[i : i + 12]))
    print("];")


# --- draw.wasm : draws a window via win_create/win_fill/win_present ----------
def build_draw():
    typesec = section(1, vec([functype([I32, I32], [I32]), functype([I32] * 6, []), functype([], [])]))
    importsec = section(2, vec([imp("env", "win_create", 0), imp("env", "win_fill", 1), imp("env", "win_present", 2)]))
    funcsec = section(3, vec([uleb(2)]))
    exportsec = section(7, vec([export("run", 3)]))

    def fill(x, y, w, h, color):
        return lget(0) + i32c(x) + i32c(y) + i32c(w) + i32c(h) + i32c(color) + call(1)

    instr = i32c(200) + i32c(120) + call(0) + lset(0)
    instr += fill(0, 0, 200, 120, 0x102A44)
    instr += fill(0, 0, 200, 22, 0xE0A000)
    instr += fill(16, 36, 72, 60, 0x27C93F)
    instr += fill(104, 36, 80, 60, 0xFF5F56)
    instr += call(2) + END
    body = uleb(1) + uleb(1) + bytes([I32]) + instr
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + exportsec + codesec


# --- file.wasm : write a file, read it back, print it (file_write/read + putc)-
def build_file():
    # t0 (i32,i32,i32,i32)->i32 ; t1 (i32)->() ; t2 ()->()
    typesec = section(1, vec([functype([I32] * 4, [I32]), functype([I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "file_write", 0), imp("env", "file_read", 0), imp("env", "putc", 1)]))
    funcsec = section(3, vec([uleb(2)]))  # run : t2
    exportsec = section(7, vec([export("run", 3)]))  # run = func 3 (3 imports + this one)
    memsec = section(5, vec([b"\x00\x01"]))  # 1 memory, min 1 page
    datasec = section(11, vec([data_seg(0, b"note.txt"), data_seg(16, b"SAVED BY WASM")]))

    # locals: n=0, p=1, end=2
    instr = bytearray()
    instr += i32c(0) + i32c(8) + i32c(16) + i32c(13) + call(0) + DROP        # file_write(name,8,data,13)
    instr += i32c(0) + i32c(8) + i32c(64) + i32c(64) + call(1) + lset(0)     # n = file_read(name,8,buf,64)
    instr += i32c(64) + lset(1)                                              # p = 64
    instr += i32c(64) + lget(0) + ADD + lset(2)                             # end = 64 + n
    instr += BLOCK + LOOP
    instr += lget(1) + lget(2) + GES + brif(1)                              # if p >= end break
    instr += lget(1) + LOAD8U + call(2)                                     # putc(mem[p])
    instr += lget(1) + i32c(1) + ADD + lset(1)                             # p += 1
    instr += br(0) + END + END + END
    body = uleb(1) + uleb(3) + bytes([I32]) + bytes(instr)  # 3 i32 locals
    codesec = section(10, vec([uleb(len(body)) + body]))
    return (
        b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec + datasec
    )


# --- clock.wasm : read the clock and print it (time + print) ----------------
def build_clock():
    typesec = section(1, vec([functype([], [I32]), functype([I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "time", 0), imp("env", "print", 1)]))
    funcsec = section(3, vec([uleb(2)]))
    exportsec = section(7, vec([export("run", 2)]))  # 2 imports + this = func 2
    instr = call(0) + call(1) + END  # print(time())
    body = uleb(0) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + exportsec + codesec


# --- type.wasm : interactive — type into a window until '~' (getkey+win_text) -
def build_type():
    EQ = b"\x46"
    # t0 win_create(i32,i32)->i32 ; t1 win_fill(6) ; t2 win_text(5) ; t3 getkey()->i32 ; t4 ()
    typesec = section(1, vec([
        functype([I32, I32], [I32]), functype([I32] * 6, []), functype([I32] * 5, []),
        functype([], [I32]), functype([], []),
    ]))
    importsec = section(2, vec([
        imp("env", "win_create", 0), imp("env", "win_fill", 1), imp("env", "win_text", 2),
        imp("env", "getkey", 3), imp("env", "win_present", 4),
    ]))
    funcsec = section(3, vec([uleb(4)]))  # run : t4
    exportsec = section(7, vec([export("run", 5)]))  # 5 imports + this = func 5

    def fill(x, y, w, h, color):
        return lget(0) + i32c(x) + i32c(y) + i32c(w) + i32c(h) + i32c(color) + call(1)

    instr = bytearray()
    instr += i32c(210) + i32c(110) + call(0) + lset(0)      # w = win_create
    instr += fill(0, 0, 210, 110, 0x141A28)                 # bg
    instr += fill(0, 0, 210, 18, 0x1B3A6B)                  # header
    instr += call(4)                                        # present
    instr += i32c(8) + lset(1)                              # x = 8
    instr += i32c(28) + lset(2)                             # y = 28
    instr += BLOCK + LOOP
    instr += call(3) + lset(3)                              # k = getkey()
    instr += lget(3) + i32c(0x7E) + EQ + brif(1)            # if k == '~' break
    instr += lget(0) + lget(1) + lget(2) + lget(3) + i32c(0xE0E6F0) + call(2)  # win_text(w,x,y,k,white)
    instr += lget(1) + i32c(8) + ADD + lset(1)             # x += 8
    instr += call(4)                                        # present
    instr += br(0) + END + END + END
    body = uleb(1) + uleb(4) + bytes([I32]) + bytes(instr)  # 4 i32 locals
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + exportsec + codesec


# --- ai.wasm : ask the on-device AI a question and print the answer ----------
def build_ai():
    typesec = section(1, vec([functype([I32] * 4, [I32]), functype([I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "ai", 0), imp("env", "putc", 1)]))
    funcsec = section(3, vec([uleb(2)]))
    exportsec = section(7, vec([export("run", 2)]))  # 2 imports + this = func 2
    memsec = section(5, vec([b"\x00\x01"]))
    datasec = section(11, vec([data_seg(0, b"WHO ARE YOU")]))
    instr = bytearray()
    instr += i32c(0) + i32c(11) + i32c(64) + i32c(192) + call(0) + lset(0)  # n = ai(prompt,11,out=64,cap=192)
    instr += i32c(64) + lset(1)
    instr += i32c(64) + lget(0) + ADD + lset(2)
    instr += BLOCK + LOOP
    instr += lget(1) + lget(2) + GES + brif(1)
    instr += lget(1) + LOAD8U + call(1)
    instr += lget(1) + i32c(1) + ADD + lset(1)
    instr += br(0) + END + END + END
    body = uleb(1) + uleb(3) + bytes([I32]) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec + datasec


# --- chat.wasm : MULTI-TURN chat — type questions, Asili answers; empty line quits
def build_chat():
    EQ = b"\x46"
    EQZ = b"\x45"
    STORE8 = b"\x3A\x00\x00"
    typesec = section(1, vec([
        functype([], [I32]), functype([I32], []), functype([I32] * 4, [I32]), functype([], []),
    ]))
    importsec = section(2, vec([imp("env", "getkey", 0), imp("env", "putc", 1), imp("env", "ai", 2)]))
    funcsec = section(3, vec([uleb(3)]))  # run : t3
    exportsec = section(7, vec([export("run", 3)]))  # 3 imports + this = func 3
    memsec = section(5, vec([b"\x00\x01"]))
    I = bytearray()
    # locals: len=0 k=1 n=2 p=3 end=4
    I += BLOCK                                        # $exit
    I += LOOP                                         #   $chat
    I += i32c(0) + lset(0)                           #     len = 0
    I += BLOCK + LOOP                                #     $lineEnd, $read
    I += call(0) + lset(1)                           #       k = getkey()
    I += lget(1) + i32c(13) + EQ + brif(1)           #       CR  -> $lineEnd
    I += lget(1) + i32c(10) + EQ + brif(1)           #       LF  -> $lineEnd
    I += lget(0) + lget(1) + STORE8                  #       mem[len] = k
    I += lget(1) + call(1)                           #       echo
    I += lget(0) + i32c(1) + ADD + lset(0)           #       len += 1
    I += br(0) + END + END                           #     br $read; end read; end lineEnd
    I += lget(0) + EQZ + brif(1)                     #     if len==0 -> $exit
    I += i32c(10) + call(1)                          #     newline
    I += i32c(0) + lget(0) + i32c(128) + i32c(192) + call(2) + lset(2)  # n = ai(...)
    I += i32c(128) + lset(3)
    I += i32c(128) + lget(2) + ADD + lset(4)
    I += BLOCK + LOOP                                #     $pd, $pr (print reply)
    I += lget(3) + lget(4) + GES + brif(1)
    I += lget(3) + LOAD8U + call(1)
    I += lget(3) + i32c(1) + ADD + lset(3)
    I += br(0) + END + END
    I += i32c(10) + call(1)                          #     newline
    I += br(0)                                       #     br $chat  (loop again)
    I += END + END                                   #   end $chat; end $exit
    I += END                                         # func end
    body = uleb(1) + uleb(5) + bytes([I32]) + bytes(I)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec


# --- el0demo.wasm : for the EL0 interpreter — uses print + putc + time -------
def build_el0demo():
    # t0 (i32)->() [print,putc] ; t1 ()->(i32) [time] ; t2 ()->() [run]
    typesec = section(1, vec([functype([I32], []), functype([], [I32]), functype([], [])]))
    importsec = section(2, vec([imp("env", "print", 0), imp("env", "putc", 0), imp("env", "time", 1)]))
    funcsec = section(3, vec([uleb(2)]))  # run : t2
    exportsec = section(7, vec([export("run", 3)]))  # 3 imports + this = func 3
    instr = call(2) + call(0)              # print(time())
    instr += i32c(ord("H")) + call(1)      # putc('H')
    instr += i32c(ord("I")) + call(1)      # putc('I')
    instr += i32c(10) + call(1)            # putc('\n')
    instr += END
    body = uleb(0) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + exportsec + codesec


# --- el0ai.wasm : for the EL0 interpreter — ai() with pointer args -----------
def build_el0ai():
    # t0 (i32,i32,i32,i32)->(i32) ai ; t1 (i32)->() print ; t2 ()->() run
    typesec = section(1, vec([functype([I32] * 4, [I32]), functype([I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "ai", 0), imp("env", "print", 1)]))
    funcsec = section(3, vec([uleb(2)]))  # run : t2
    exportsec = section(7, vec([export("run", 2)]))  # 2 imports + this = func 2
    memsec = section(5, vec([b"\x00\x01"]))
    datasec = section(11, vec([data_seg(0, b"WHO ARE YOU")]))
    # print(ai(prompt=0, len=11, out=64, cap=192)) -> prints the reply length
    instr = i32c(0) + i32c(11) + i32c(64) + i32c(192) + call(0) + call(1) + END
    body = uleb(0) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec + datasec


# --- el0aispeak.wasm : EL0 app asks the AI and SPEAKS the answer -------------
def build_el0aispeak():
    # t0 ai(4)->i32 ; t1 speak(i32,i32) ; t2 print(i32) ; t3 ()->()
    typesec = section(1, vec([
        functype([I32] * 4, [I32]), functype([I32, I32], []), functype([I32], []), functype([], []),
    ]))
    importsec = section(2, vec([imp("env", "ai", 0), imp("env", "speak", 1), imp("env", "print", 2)]))
    funcsec = section(3, vec([uleb(3)]))  # run : t3
    exportsec = section(7, vec([export("run", 3)]))  # 3 imports + this = func 3
    memsec = section(5, vec([b"\x00\x01"]))
    datasec = section(11, vec([data_seg(0, b"WHO ARE YOU")]))
    # local $n ; n = ai(0,11,64,192) ; speak(64, n) ; print(n)
    instr = i32c(0) + i32c(11) + i32c(64) + i32c(192) + call(0) + lset(0)
    instr += i32c(64) + lget(0) + call(1)
    instr += lget(0) + call(2)
    instr += END
    body = uleb(1) + uleb(1) + bytes([I32]) + bytes(instr)  # 1 i32 local
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec + datasec


# --- notify.wasm : an app speaks a notification via Asili Voice --------------
NOTIFY_MSG = b"YOUR PC IS LOW ON BATTERY, PLEASE CHARGE IT."
# 3.262: the SECOND text for the 3.260 text-independence probe. It is baked as an
# ADDITIVE module (notify2.wasm) rather than substituted into WASM_NOTIFY, so ONE
# kernel build serves BOTH boots and notify.wasm stays byte-identical -- the cheaper
# and more reversible of the two routes 3.261 measured. Deliberately shares NO word
# with NOTIFY_MSG: a formant synthesiser must produce different PCM for it, while
# sound_play_tone() (which takes no argument) cannot.
NOTIFY2_MSG = b"THE KETTLE HAS BOILED, YOUR TEA IS READY."


def build_notify(msg=NOTIFY_MSG):
    # 3.261: `msg` is a PARAMETER so the 3.260 text-independence probe can build two
    # modules differing ONLY in the spoken text (2.765 rule 5 -- call this generator,
    # never hand-roll a second one). The DEFAULT is the original literal byte-for-byte,
    # so `python scripts/gen-wasm.py` still prints exactly the WASM_NOTIFY baked at
    # kernel/src/nia.rs:247 -- verified by COMPARING the two byte arrays, not by eye.
    typesec = section(1, vec([functype([I32, I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "speak", 0)]))
    funcsec = section(3, vec([uleb(1)]))  # run : t1
    exportsec = section(7, vec([export("run", 1)]))  # 1 import + this = func 1
    memsec = section(5, vec([b"\x00\x01"]))
    datasec = section(11, vec([data_seg(0, msg)]))
    instr = i32c(0) + i32c(len(msg)) + call(0) + END  # speak(0, len)
    body = uleb(0) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + memsec + exportsec + codesec + datasec


def build_net():
    # net(op)->i32 (t0), print(i32) (t1), run() (t2). run: print(net(0)); print(net(1)).
    typesec = section(1, vec([functype([I32], [I32]), functype([I32], []), functype([], [])]))
    importsec = section(2, vec([imp("env", "net", 0), imp("env", "print", 1)]))
    funcsec = section(3, vec([uleb(2)]))
    exportsec = section(7, vec([export("run", 2)]))  # 2 imports + this = func 2
    instr = i32c(0) + call(0) + call(1) + i32c(1) + call(0) + call(1) + END
    body = uleb(0) + bytes(instr)
    codesec = section(10, vec([uleb(len(body)) + body]))
    return b"\x00asm" + bytes([1, 0, 0, 0]) + typesec + importsec + funcsec + exportsec + codesec


emit("WASM_NET", build_net())
print()
emit("WASM_DRAW", build_draw())
print()
emit("WASM_FILE", build_file())
print()
emit("WASM_CLOCK", build_clock())
print()
emit("WASM_TYPE", build_type())
print()
emit("WASM_AI", build_ai())
print()
emit("WASM_CHAT", build_chat())
print()
emit("WASM_NOTIFY", build_notify())
print()
emit("WASM_NOTIFY2", build_notify(NOTIFY2_MSG))
print()
emit("WASM_EL0DEMO", build_el0demo())
print()
emit("WASM_EL0AI", build_el0ai())
print()
emit("WASM_EL0AISPEAK", build_el0aispeak())
