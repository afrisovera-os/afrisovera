#!/usr/bin/env python3
"""AfriSovera performance benchmark (suites `perf` and `pc-perf`, 13 Nov 2026).

Owner's mandate: keep AfriSovera fast on a 4 GB machine and good for servers. This boots ONE kernel in QEMU,
types a fixed list of shell commands, and turns what the kernel prints into numbers: boot time, idle memory,
memory taken by apps, the kernel's own size, CPU speed, drawing speed, sleep accuracy and network intake.

REPEATABLE ON ANY CLOUD MACHINE. QEMU runs with `-icount shift=0`: the guest clock advances one nanosecond per
guest instruction (a 1 GHz in-order core), so a time the kernel measures itself (boot ticks, `bench`, the
drawing timers) counts INSTRUCTIONS, not the speed of whatever the cloud runner or GitHub machine ran it. A slower runner
makes the run longer, not the numbers bigger. Times taken while the guest waits for the outside (the network
peer) still follow the host clock; those rows are marked `host` and graded loosely.

Usage:
  perf-bench.py run   --arch arm|pc --kernel PATH [--ovmf FILE] --out DIR
  perf-bench.py grade --results DIR/perf-<arch>.tsv [--budgets docs/perf/budgets.tsv]
                      [--baseline docs/perf/baseline.tsv]
  perf-bench.py --selftest      the parser and the grader on canned text, both directions
Exit: 0 green, 1 red (a budget or a regression), 2 harness trouble (no boot, no QEMU).
"""
import fnmatch
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
PROMPT = b"afrisovera>"
GPORT = 7811                       # the sink port inside the guest (never bound on the host)
PORTS = {"arm": (5481, 5484), "pc": (5488, 5489), "arm-rel": (5491, 5493), "pc-rel": (5495, 5496)}   # serial, qmp: unused by any other suite (13 Nov 2026)
SINK_SECS, PUSH_SECS = 12, 6
HIGHER_IS_BETTER = ("_mibs", "_kibs_host", "_bytes")   # every other metric: lower is better

# The commands typed after boot, in order. Each is read back by `parse()`.
CMDS = [
    "uptime",            # boot time: ticks since the kernel's clock started
    "echo PERF-MARK idle",
    "mem",               # idle memory: heap and physical frames
    "bench",             # CPU: SHA-256 over 2 MiB, timed by the kernel
    "fbcon time",        # drawing: console text
    "fbcon blit",        # drawing: whole-screen copy
    "fbcon wm",          # drawing: window create / fill / present
    "sleep 50",          # timer accuracy: ask for 50 ms, see what is given
    "net tcp sink %d %d" % (GPORT, SINK_SECS),   # network intake: a peer pushes bytes for PUSH_SECS
    "wm",                # three apps open, draw a window each and exit
    "clear",
    "echo PERF-MARK apps1",
    "mem",               # after the apps: what they left behind
    "wm",                # the same again: memory that grows on every run is a leak
    "clear",
    "echo PERF-MARK apps2",
    "mem",
    "desk",              # the desktop (Tonga/Sovereign UI)
    "desk open files",
    "desk perf",         # its own frame timers
    "echo PERF-MARK desk",
    "mem",               # with the desktop and one app window open
    "clear",
    "desk look probe",   # both looks in both modes, a fixed line each (desk designer, 14 Nov; absent before step 2)
    "clear",
]


# ------------------------------------------------------------------ kernel size --
def static_size(path):
    """Bytes of RAM the kernel image itself takes once loaded: code, read-only data, data, zeroed data.
    ELF (ARM kernel): allocated sections. PE (PC .efi): each section's virtual size."""
    out = {"text": 0, "rodata": 0, "data": 0, "bss": 0}
    with open(path, "rb") as f:
        b = f.read()
    if b[:4] == b"\x7fELF":
        shoff, = struct.unpack_from("<Q", b, 0x28)
        shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
        hdrs = [struct.unpack_from("<IIQQQQIIQQ", b, shoff + i * shentsize) for i in range(shnum)]
        stroff = hdrs[shstrndx][4]
        for (name, typ, flags, _a, _o, size, *_r) in hdrs:
            if not flags & 0x2:            # SHF_ALLOC: not loaded, not counted
                continue
            nm = b[stroff + name:b.index(b"\0", stroff + name)].decode()
            if typ == 8:                   # SHT_NOBITS
                out["bss"] += size
            elif flags & 0x4:              # SHF_EXECINSTR
                out["text"] += size
            elif flags & 0x1:              # SHF_WRITE
                out["data"] += size
            else:
                out["rodata"] += size
            _ = nm
    elif b[:2] == b"MZ":
        pe, = struct.unpack_from("<I", b, 0x3C)
        nsec, = struct.unpack_from("<H", b, pe + 6)
        opt, = struct.unpack_from("<H", b, pe + 20)
        sec = pe + 24 + opt
        for i in range(nsec):
            name = b[sec + i * 40: sec + i * 40 + 8].rstrip(b"\0").decode("latin1")
            vsize, _va, rawsize = struct.unpack_from("<III", b, sec + i * 40 + 8)
            chars, = struct.unpack_from("<I", b, sec + i * 40 + 36)
            if name in (".reloc",) or name.startswith(".debug"):
                continue
            if chars & 0x20000000:         # executable
                out["text"] += vsize
            elif chars & 0x80000000:       # writable: data, and the zeroed tail past the file bytes is bss
                out["data"] += min(vsize, rawsize)
                out["bss"] += max(0, vsize - rawsize)
            else:
                out["rodata"] += vsize
    out["total"] = sum(out.values())
    return {"image." + k: v // 1024 for k, v in out.items()}   # KiB


# ---------------------------------------------------------------------- parse --
def _num(rx, s, cast=int, last=False):
    m = list(re.finditer(rx, s, re.M))
    if not m:
        return None
    return cast((m[-1] if last else m[0]).group(1))


def parse(text):
    """Serial transcript -> {metric: number}. A missing line is a missing metric (graded later), never a guess."""
    r = {}
    t = text.replace("\r", "")
    m = re.search(r"^\[pc\] boot steps \(ms\): total (\d+)(.*)$", t, re.M)
    if m:
        r["boot.steps_total_ms"] = int(m.group(1))
        for name, ms in re.findall(r"; ([a-z0-9-]+) (\d+)", m.group(2)):
            r["boot.step.%s_ms" % name] = int(ms)
    v = _num(r"^(\d+) ticks\s+\(~\d+ ms", t)
    if v is not None:
        r["boot.kernel_ms"] = v * 10           # the shell's clock ticks at 100 Hz
    rx = (r"heap:\s+(\d+) / (\d+) bytes used \((\d+) B high-water.*?\n.*?\n"
          r"frames: (\d+) total .*?, (\d+) used")
    snap = {}
    for part in re.split(r"^PERF-MARK ", t, flags=re.M)[1:]:
        mm = re.search(rx, part, re.S)
        if mm:
            snap[part.split("\n", 1)[0].strip()] = mm
    mems = [snap[k] for k in ("idle", "apps1") if k in snap]
    if "desk" in snap and ("apps2" in snap or "apps1" in snap):
        mems.append(snap.get("apps2", snap.get("apps1")))   # the state just before the desktop opened
        mems.append(snap["desk"])
    if not snap:                                   # transcripts without marks: positional
        mems = list(re.finditer(rx, t, re.S))
    if "apps1" in snap and "apps2" in snap:
        a1, a2 = snap["apps1"], snap["apps2"]
        r["mem.apps_rerun_growth_kib"] = max(0, (int(a2.group(5)) - int(a1.group(5))) * 4
                                             + (int(a2.group(1)) - int(a1.group(1))) // 1024)
        r["mem.apps_rerun_frames"] = int(a2.group(5)) - int(a1.group(5))
        r["mem.apps_rerun_heap_b"] = int(a2.group(1)) - int(a1.group(1))
    if mems:
        m = mems[0]
        r["mem.idle_heap_kib"] = int(m.group(1)) // 1024
        r["mem.heap_arena_kib"] = int(m.group(2)) // 1024
        r["mem.idle_frames_kib"] = int(m.group(5)) * 4
        hw0 = int(m.group(3))
        kib = lambda x: int(x.group(5)) * 4 + int(x.group(1)) // 1024   # frames + heap in use
        if len(mems) > 1:
            m2 = mems[1]
            r["mem.apps3_peak_heap_kib"] = max(0, int(m2.group(3)) - hw0) // 1024
            r["mem.apps3_left_kib"] = max(0, kib(m2) - kib(m))
        if len(mems) > 3:
            r["mem.desk_open_kib"] = max(0, kib(mems[3]) - kib(mems[2]))   # desktop + the files window
        elif len(mems) == 3:                                                 # transcripts without marks
            r["mem.desk_open_kib"] = max(0, kib(mems[2]) - kib(mems[1]))
    m = re.search(r"\[desk\] perf: (\d+)x(\d+) at UI scale \d+x; draw (\d+) us \(worst (\d+) us\).*?"
                  r"copy to the screen (\d+) us \(worst (\d+) us\)", t)
    if m:
        r["draw.desk.frame_us"] = int(m.group(3))
        r["draw.desk.frame_worst_us"] = int(m.group(4))
        r["draw.desk.copy_us"] = int(m.group(5))
        r["draw.desk.copy_worst_us"] = int(m.group(6))
    # `desk look probe` (afriui step 2): one line per mode and look, and the title's worst of 20
    for mode, look, full, part in re.findall(
            r"^\[desk\] probe: mode (touch|pointer) look (flat|premium) \d+x\d+: full (\d+) us, partial (\d+) us", t, re.M):
        r["draw.desk.%s-%s.full_us" % (mode, look)] = int(full)
        r["draw.desk.%s-%s.partial_us" % (mode, look)] = int(part)
    v = _num(r"^\[desk\] probe: title worst of 20: (\d+) us", t)
    if v is not None:
        r["draw.desk.title_worst20_us"] = v
    m = re.search(r"\[desk\] perf: the slowest motion frame (\d+) us over (\d+)", t)
    if m and int(m.group(2)) > 0:
        r["draw.desk.motion_worst_us"] = int(m.group(1))
    m = re.search(r"bench: sha256 .*? in (\d+) us \((\d+) MiB/s\)", t)
    if m:
        r["cpu.sha256_us"] = int(m.group(1))
        r["cpu.sha256_mibs"] = int(m.group(2))
    # drawing timers: every "<word> <N> us" pair on the fbcon lines, e.g. "[fbwm] create 1234 us"
    for line in t.split("\n"):
        tag = re.match(r"^\[(fbcon|fbblit|fbwm|fbcon-time|blit)\]", line)
        if not tag:
            continue
        for w, n in re.findall(r"([A-Za-z][A-Za-z0-9_-]*)[ =:]+(\d+) ?us\b", line):
            r.setdefault("draw.%s.%s_us" % (tag.group(1), w.lower()), int(n))
    m = re.search(r"\[sleep\] slept (\d+) ticks", t)
    if m:
        r["timer.sleep50_over_ms"] = max(0, int(m.group(1)) * 10 - 50)
    m = re.search(r"\[net\] sink: (\d+) byte\(s\) sunk in (\d+) segment\(s\) over (\d+) ms", t)
    if m:
        r["net.sink_bytes"] = int(m.group(1))
        r["net.sink_kibs_host"] = int(m.group(1)) * 1000 // max(1, int(m.group(3))) // 1024
    return r


# ------------------------------------------------------------------------ run --
def json_ret(line):
    """The text of a QMP human-monitor-command reply (or the raw line if it is not one)."""
    import json
    try:
        return json.loads(line).get("return", line)
    except ValueError:
        return line
def run(arch, kernel, out_dir, ovmf=None, label=None):
    label = label or arch
    os.makedirs(out_dir, exist_ok=True)
    sport, qport = PORTS.get(label, PORTS[arch])   # each run its own ports: a dying QEMU cannot block the next
    tmp = tempfile.mkdtemp(prefix="perf-%s-" % label)
    portfile = os.path.join(tmp, "peer.port")
    peerlog = open(os.path.join(out_dir, "perf-%s-peer.log" % label), "w")
    peer = subprocess.Popen([sys.executable, os.path.join(HERE, "sink-peer-3488.py"), portfile, str(GPORT),
                             "240", str(PUSH_SECS)], stdout=peerlog, stderr=subprocess.STDOUT)
    for _ in range(80):
        if os.path.exists(portfile) and open(portfile).read().strip():
            break
        time.sleep(0.25)
    wire = open(portfile).read().strip() if os.path.exists(portfile) else ""
    net = (["-netdev", "socket,id=n0,connect=127.0.0.1:%s" % wire] if wire else [])
    common = ["-icount", "shift=0,align=off,sleep=on", "-display", "none",
              "-serial", "tcp:127.0.0.1:%d,server,nowait" % sport,
              "-qmp", "tcp:127.0.0.1:%d,server,nowait" % qport]
    if arch == "arm":
        cmd = ["qemu-system-aarch64", "-M", "virt", "-cpu", "cortex-a72", "-smp", "1", "-m", "256M",
               "-global", "virtio-mmio.force-legacy=false", "-device", "ramfb"] + common + net
        if wire:
            cmd += ["-device", "virtio-net-device,netdev=n0"]
        cmd += ["-kernel", kernel]
    else:
        esp = os.path.join(tmp, "esp")
        os.makedirs(os.path.join(esp, "EFI", "BOOT"))
        shutil.copy(kernel, os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"))
        cmd = ["qemu-system-x86_64", "-M", "q35", "-smp", "1", "-m", "4G",   # the owner's 4 GB laptop
               "-drive", "if=pflash,format=raw,readonly=on,file=%s" % ovmf,
               "-drive", "format=raw,file=fat:rw:%s" % esp] + common + net
        if wire:
            cmd += ["-device", "virtio-net-pci,netdev=n0"]
    t0 = time.time()
    q = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=open(os.path.join(out_dir, "perf-%s-qemu.err" % label), "w"))
    log = bytearray()
    stamps = []          # (host seconds since QEMU start, log length after this chunk)
    boot_end = 0
    res = {}
    try:
        ser = None
        for _ in range(200):
            try:
                ser = socket.create_connection(("127.0.0.1", sport), timeout=1)
                break
            except OSError:
                if q.poll() is not None:
                    break
                time.sleep(0.1)
        if ser is None:
            print("PERF HARNESS: could not attach to the serial port (QEMU exit %s)" % q.poll())
            return None
        ser.settimeout(0.2)

        def until_prompt(start, ceiling):
            end = time.time() + ceiling
            while time.time() < end:
                try:
                    d = ser.recv(65536)
                    if not d:
                        return False
                    log.extend(d)
                    stamps.append((time.time() - t0, len(log)))
                except socket.timeout:
                    pass
                if PROMPT in log[start:]:
                    return True
            return False

        if not until_prompt(0, 240):
            print("PERF HARNESS: no shell prompt within 240 s (%d serial bytes)" % len(log))
            with open(os.path.join(out_dir, "perf-%s.serial" % label), "wb") as f:
                f.write(bytes(log))
            # where the guest is stuck: QEMU's register dump, three times a second apart (a loop shows as RIP
            # moving inside one function, a halt or a fault loop as one address)
            try:
                with socket.create_connection(("127.0.0.1", qport), timeout=5) as qm, \
                        open(os.path.join(out_dir, "perf-%s-regs.txt" % label), "w") as f:
                    rd = qm.makefile("r")
                    rd.readline()
                    qm.sendall(b'{"execute":"qmp_capabilities"}\n')
                    rd.readline()
                    for _ in range(3):
                        qm.sendall(b'{"execute":"human-monitor-command","arguments":{"command-line":"info registers"}}\n')
                        f.write(json_ret(rd.readline()) + "\n----\n")
                        time.sleep(1)
            except OSError as e:
                print("PERF HARNESS: no register dump (%s)" % e)
            return None
        res["boot.host_s"] = round(time.time() - t0, 1)
        boot_end = len(log)   # informational: the runner's speed, not the kernel's
        time.sleep(0.3)
        for c in CMDS:
            start = len(log)
            ser.sendall((c + "\r").encode())
            ceiling = SINK_SECS + 60 if c.startswith("net tcp sink") else 120
            if not until_prompt(start + len(c), ceiling):
                print("PERF HARNESS: no prompt back after %r" % c)
        ser.close()
    finally:
        q.kill()
        q.wait()
        peer.kill()
        peer.wait()
        peerlog.close()
        shutil.rmtree(tmp, ignore_errors=True)
    text = log.decode("utf-8", "replace")
    # The boot trail: each line with the host time it arrived; the biggest gaps are the slow boot steps (under
    # -icount the host time of a busy guest is proportional to its instructions; a guest WAITING is not).
    trail, prev_t, prev_line, pos, part = [], 0.0, "(QEMU start: firmware)", 0, ""
    for st, end in stamps:
        if pos >= boot_end:
            break
        part += bytes(log[pos:min(end, boot_end)]).decode("utf-8", "replace").replace("\r", "")
        *done, part = part.split("\n")
        for ln in done:
            if ln.strip():
                trail.append((st - prev_t, prev_line))
                prev_t, prev_line = st, ln.strip()[:110]
        pos = end
    with open(os.path.join(out_dir, "perf-%s-boot.txt" % label), "w", encoding="utf-8") as f:
        for gap, ln in sorted(trail, reverse=True)[:12]:
            f.write("%7.2f s after: %s\n" % (gap, ln))
    with open(os.path.join(out_dir, "perf-%s.serial" % label), "w", encoding="utf-8") as f:
        f.write(text)
    res.update(parse(text))
    res.update(static_size(kernel))
    if "mem.idle_frames_kib" in res:
        # idle RAM in use = the loaded image (its zeroed data holds the heap's reserved arena) + frames taken
        res["mem.idle_total_kib"] = res["mem.idle_frames_kib"] + res["image.total"]
    path = os.path.join(out_dir, "perf-%s.tsv" % label)
    with open(path, "w") as f:
        for k in sorted(res):
            f.write("%s\t%s\t%s\n" % (label, k, res[k]))
    return path


# ---------------------------------------------------------------------- grade --
def read_tsv(path):
    """arch<TAB>metric<TAB>value[<TAB>more]; '#' lines and blanks skipped."""
    rows = {}
    if path and os.path.exists(path):
        for line in open(path):
            if line.strip() and not line.startswith("#"):
                p = line.rstrip("\n").split("\t")
                rows[(p[0], p[1])] = p[2:]
    return rows


def grade(results, budgets, baseline, quiet=False):
    """Red when a metric is over its budget, or worse than the kept baseline by more than its tolerance.
    Budgets: arch metric max tolerance_pct required(yes|no) note. A '*' arch applies to both kernels."""
    res = read_tsv(results)
    bud = read_tsv(budgets)
    base = read_tsv(baseline)
    arch = next(iter(res))[0] if res else "?"
    fails, lines = [], []
    def budget_for(a, m):
        for (ba, bm), b in bud.items():
            if (ba == a and bm == m):
                return b
        for (ba, bm), b in bud.items():
            if ba in (a, "*") and fnmatch.fnmatch(m, bm):
                return b
        return None
    for (a, m), (v, *_r) in sorted(res.items()):
        b = budget_for(a, m)
        v = float(v)
        up = m.endswith(HIGHER_IS_BETTER)
        note = ""
        if b:
            mx, tol, *_ = b + ["", ""]
            if mx not in ("", "-") and (v < float(mx) if up else v > float(mx)):
                fails.append("%s %s = %g is %s its budget %s" % (a, m, v, "under" if up else "over", mx))
                note = "OUTSIDE BUDGET %s" % mx
            bv = base.get((a, m))
            if bv and tol not in ("", "-"):
                old = float(bv[0])
                worse = (v < old * (1 - float(tol) / 100) - 1) if up else (v > old * (1 + float(tol) / 100) + 1)
                if worse:
                    fails.append("%s %s = %g, was %g (more than %s%% worse)" % (a, m, v, old, tol))
                    note = (note + " " if note else "") + "REGRESSION from %g" % old
                elif not note:
                    note = "baseline %g" % old
        lines.append("  %-4s %-34s %12g  %s" % (a, m, v, note))
    for (a, m), b in sorted(bud.items()):
        if (a in (arch, "*")) and len(b) > 2 and b[2] == "yes" and (arch, m) not in res:
            fails.append("%s %s is missing (the kernel never printed it)" % (arch, m))
    if not quiet:
        print("\n".join(lines))
        for f in fails:
            print("  FAIL: " + f)
    return 1 if fails else 0


# ------------------------------------------------------------------- selftest --
CANNED = """[pc] boot steps (ms): total 5590; self-tests 3000; ahci 2100; xhci 400
afrisovera> uptime
150 ticks  (~1500 ms, ~1 s)
afrisovera> echo PERF-MARK idle
PERF-MARK idle
afrisovera> mem
heap:   40960 / 1048576 bytes used (51200 B high-water, arena 1024 KiB @ 0x40100000 = 1024 KiB reserved + 0 KiB grown)
        0 B returned in 0 block(s), 0 B of it free again (0 reuse(s), bump at 40960 B)
frames: 60000 total (234 MiB of 4KiB physical frames), 1000 used \xe2\x80\x94 0 of them the heap's arena
afrisovera> bench
bench: sha256 512 iters x 4096 bytes = 2048 KiB in 80000 us (25 MiB/s)
afrisovera> fbcon wm
[fbwm] create 120 us, present 3400 us
afrisovera> sleep 50
[sleep] slept 6 ticks (~60 ms)
afrisovera> net tcp sink 7811 12
[net] sink: 1048576 byte(s) sunk in 2048 segment(s) over 6000 ms (window 12000 ms; 0 dup, 0 gap) \xe2\x80\x94 nothing echoed
afrisovera> echo PERF-MARK apps1
PERF-MARK apps1
afrisovera> mem
heap:   45056 / 1048576 bytes used (81920 B high-water, arena 1024 KiB @ 0x40100000 = 1024 KiB reserved + 0 KiB grown)
        0 B returned in 0 block(s), 0 B of it free again (0 reuse(s), bump at 45056 B)
frames: 60000 total (234 MiB of 4KiB physical frames), 1002 used \xe2\x80\x94 0 of them the heap's arena
afrisovera> desk perf
[desk] perf: the slowest motion frame 0 us over 0 motion frame(s)
[desk] perf: 640x480 at UI scale 1x; draw 9000 us (worst 12000 us) over 3 frame(s); partial 0 us (worst 0 us) over 0 frame(s); copy to the screen 2500 us (worst 4000 us) over 3 copies, the last 480 row(s)
afrisovera> echo PERF-MARK desk
PERF-MARK desk
afrisovera> mem
heap:   61440 / 1048576 bytes used (81920 B high-water, arena 1024 KiB @ 0x40100000 = 1024 KiB reserved + 0 KiB grown)
        0 B returned in 0 block(s), 0 B of it free again (0 reuse(s), bump at 61440 B)
frames: 60000 total (234 MiB of 4KiB physical frames), 1100 used \xe2\x80\x94 0 of them the heap's arena
afrisovera> """


def selftest():
    ok = True

    def check(cond, what):
        nonlocal ok
        print("  %s %s" % ("ok  " if cond else "FAIL", what))
        ok = ok and cond
    r = parse(CANNED)
    check(r.get("boot.kernel_ms") == 1500, "boot.kernel_ms 1500 from 150 ticks")
    check(r.get("mem.idle_heap_kib") == 40, "idle heap 40 KiB")
    check(r.get("mem.idle_frames_kib") == 4000, "idle frames 1000 x 4 KiB")
    check(r.get("mem.apps3_peak_heap_kib") == 30, "apps peak heap = high-water rise 30 KiB")
    check(r.get("mem.apps3_left_kib") == 12, "apps left 2 frames + 4 KiB heap = 12 KiB")
    check(r.get("cpu.sha256_us") == 80000, "bench us")
    check(r.get("boot.steps_total_ms") == 5590 and r.get("boot.step.ahci_ms") == 2100, "PC boot steps")
    check(r.get("mem.desk_open_kib") == 408, "desk open: 98 frames + 16 KiB heap over the after-apps state")
    r2 = parse(CANNED.replace("PERF-MARK apps1", "PERF-MARK apps1").replace(
        "afrisovera> echo PERF-MARK desk", "afrisovera> echo PERF-MARK apps2\nPERF-MARK apps2\nafrisovera> mem\n"
        "heap:   49152 / 1048576 bytes used (81920 B high-water, arena)\n x\n"
        "frames: 60000 total (234 MiB), 1005 used\nafrisovera> echo PERF-MARK desk"))
    check(r2.get("mem.apps_rerun_frames") == 3 and r2.get("mem.apps_rerun_heap_b") == 4096
          and r2.get("mem.apps_rerun_growth_kib") == 16, "rerun leak split: 3 frames + 4 KiB heap")
    check(r2.get("mem.desk_open_kib") == 392, "desk measured against the second apps run")
    check(r.get("draw.desk.frame_us") == 9000 and r.get("draw.desk.copy_worst_us") == 4000, "desk perf")
    check("draw.desk.motion_worst_us" not in r, "no motion frames, no motion metric")
    rp = parse("[desk] probe: mode touch look premium 640x480: full 12345 us, partial 2345 us\n"
               "[desk] probe: mode pointer look flat 1280x800: full 30000 us, partial 900 us (more text)\n"
               "[desk] probe: title worst of 20: 549 us\n")
    check(rp.get("draw.desk.touch-premium.full_us") == 12345 and rp.get("draw.desk.pointer-flat.partial_us") == 900
          and rp.get("draw.desk.title_worst20_us") == 549, "desk look probe lines")
    check(r.get("draw.fbwm.create_us") == 120 and r.get("draw.fbwm.present_us") == 3400, "fbwm pairs")
    check(r.get("timer.sleep50_over_ms") == 10, "sleep overshoot 10 ms")
    check(r.get("net.sink_kibs_host") == 170, "sink 1 MiB in 6 s = 170 KiB/s")
    check(parse("no prompt at all") == {}, "nothing printed, nothing invented")
    d = tempfile.mkdtemp()
    w = lambda n, s: open(os.path.join(d, n), "w").write(s) and os.path.join(d, n)
    res = w("r.tsv", "arm\tboot.kernel_ms\t1500\narm\tmem.idle_total_kib\t4000\n")
    bud = w("b.tsv", "*\tboot.kernel_ms\t2000\t20\tyes\n*\tmem.idle_total_kib\t65536\t10\tyes\n")
    good = w("g.tsv", "arm\tboot.kernel_ms\t1400\narm\tmem.idle_total_kib\t3900\n")
    slow = w("s.tsv", "arm\tboot.kernel_ms\t1000\narm\tmem.idle_total_kib\t4000\n")
    check(grade(res, bud, good, quiet=True) == 0, "within budget and tolerance: green")
    check(grade(res, bud, slow, quiet=True) == 1, "50% slower boot than baseline: red")
    tight = w("t.tsv", "*\tboot.kernel_ms\t1000\t-\tyes\n")
    check(grade(res, tight, None, quiet=True) == 1, "over budget: red")
    miss = w("m.tsv", "*\tnet.sink_bytes\t-\t-\tyes\n")
    check(grade(res, miss, None, quiet=True) == 1, "required metric missing: red")
    fast = w("f.tsv", "*\tnet.*\t100\t30\tno\n")
    netr = w("n.tsv", "arm\tnet.sink_kibs_host\t50\n")
    check(grade(netr, fast, None, quiet=True) == 1, "higher-is-better metric under its glob floor: red")
    wide = w("wd.tsv", "*\tnet.*\t10\t30\tno\n")
    check(grade(netr, wide, w("nb.tsv", "arm\tnet.sink_kibs_host\t100\n"), quiet=True) == 1,
          "intake halved against the baseline: red")
    check(grade(netr, wide, w("nc.tsv", "arm\tnet.sink_kibs_host\t55\n"), quiet=True) == 0,
          "intake within 30% of the baseline: green")
    shutil.rmtree(d, ignore_errors=True)
    print("PERF SELFTEST %s" % ("PASSED" if ok else "FAILED"))
    return 0 if ok else 1


def owner(name):
    """The kernel module or the crate a mangled (v0) symbol belongs to: `mansa_pc::wifi`, `aes`, `core`."""
    m = re.search(r"C[sR]?[0-9a-zA-Z]*_(\d+)([A-Za-z_][A-Za-z0-9_]*)", name)
    if not m:
        return name.split("@")[0][:24] or "?"
    n = int(m.group(1))
    crate = (m.group(2) + name[m.end():])[:n]
    rest = name[m.start(2) + n:]
    if crate in ("mansa_pc", "mansa_kernel"):
        mm = re.match(r"(\d+)([A-Za-z_][A-Za-z0-9_]*)", rest)
        if mm:
            k = int(mm.group(1))
            return crate + "::" + (mm.group(2) + rest[mm.end():])[:k]
    return crate


def mapmods(path, top=60):
    """Code and read-only data per kernel module and per crate, from an MSVC-style link map, each symbol sized by
    the gap to the next one in its section (KiB). Says which part of the image grew."""
    rows, names = [], {}
    for line in open(path, errors="replace"):
        h = re.match(r"^\s*([0-9a-fA-F]{4}):[0-9a-fA-F]{8}\s+[0-9a-fA-F]{8}H\s+(\S+)", line)
        if h:
            names.setdefault(int(h.group(1), 16), h.group(2))
            continue
        m = re.match(r"^\s*([0-9a-fA-F]{4}):([0-9a-fA-F]{8})\s+(\S+)\s+[0-9a-fA-F]{8,16}", line)
        if m:
            rows.append((int(m.group(1), 16), int(m.group(2), 16), m.group(3)))
    rows.sort()
    tally = {}
    for (s, o, n), nxt in zip(rows, rows[1:] + [None]):
        if not nxt or nxt[0] != s:
            continue
        sec = names.get(s, str(s))
        if not sec.startswith((".text", ".rdata")):
            continue
        key = (owner(n), "text" if sec.startswith(".text") else "rodata")
        tally[key] = tally.get(key, 0) + (nxt[1] - o)
    for kind in ("text", "rodata"):
        items = sorted(((v, k[0]) for k, v in tally.items() if k[1] == kind), reverse=True)[:top]
        print("  %s by module or crate (KiB): %s" % (kind, ", ".join("%s %d" % (o, v // 1024) for v, o in items)))
    return 0


def mapsyms(path, top=20):
    """The biggest symbols of an MSVC-style link map (lld-link /MAP), sized by the gap to the next symbol in the
    same section: the PC image has no symbol table of its own. Prints KiB, section, name."""
    rows, secend = [], {}
    for line in open(path, errors="replace"):
        h = re.match(r"^\s*([0-9a-fA-F]{4}):([0-9a-fA-F]{8})\s+([0-9a-fA-F]{8})H\s+(\S+)", line)
        if h:
            sec, st = int(h.group(1), 16), int(h.group(2), 16)
            secend[sec] = max(secend.get(sec, 0), st + int(h.group(3), 16))
            continue
        m = re.match(r"^\s*([0-9a-fA-F]{4}):([0-9a-fA-F]{8})\s+(\S+)\s+([0-9a-fA-F]{8,16})", line)
        if m:
            rows.append((int(m.group(1), 16), int(m.group(2), 16), m.group(3)))
    rows.sort()
    sized = []
    for (s, o, n), nxt in zip(rows, rows[1:] + [None]):
        if nxt and nxt[0] == s:
            sized.append((nxt[1] - o, s, n))
        elif s in secend:
            sized.append((secend[s] - o, s, n + " (last in its section)"))
    for size, s, n in sorted(sized, reverse=True)[:top]:
        print("  %8d KiB  sec %d  %s" % (size // 1024, s, n))
    return 0


def riplookup(mappath, regspath, serialpath):
    """Name the functions the stuck PC kernel was in: the map's addresses are moved by the difference between
    where the frame pool really was (its boot line) and where the map put it."""
    syms = []
    for line in open(mappath, errors="replace"):
        m = re.match(r"^\s*[0-9a-fA-F]{4}:[0-9a-fA-F]{8}\s+(\S+)\s+([0-9a-fA-F]{8,16})", line)
        if m:
            syms.append((int(m.group(2), 16), m.group(1)))
    syms.sort()
    pool = dict((n, v) for v, n in syms).get("mansa_pc_frame_pool")
    m = re.search(r"frame pool: \d+ frames .*? at (0x[0-9a-f]+)", open(serialpath, errors="replace").read())
    if pool is None or not m:
        print("  (no frame pool line or symbol: cannot place the image)")
        return 0
    delta = int(m.group(1), 16) - pool
    for rip in re.findall(r"RIP=([0-9a-fA-F]+)", open(regspath, errors="replace").read()):
        va = int(rip, 16) - delta
        best = None
        for v, n in syms:
            if v > va:
                break
            best = (v, n)
        print("  RIP %s = %s + %#x" % (rip, best[1] if best else "?", va - best[0] if best else 0))
    return 0


def main():
    a = sys.argv[1:]
    if not a or a[0] == "--selftest":
        return selftest()
    if a[0] == "mapsyms":
        return mapsyms(a[1])
    if a[0] == "mapmods":
        return mapmods(a[1])
    if a[0] == "riplookup":
        return riplookup(a[1], a[2], a[3])
    opt = {a[i]: a[i + 1] for i in range(1, len(a) - 1, 2)}
    if a[0] == "run":
        p = run(opt["--arch"], opt["--kernel"], opt["--out"], opt.get("--ovmf"), opt.get("--label"))
        print(p or "")
        return 0 if p else 2
    if a[0] == "grade":
        return grade(opt["--results"], opt.get("--budgets"), opt.get("--baseline"))
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
