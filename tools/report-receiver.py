#!/usr/bin/env python3
"""Receive AfriSovera field reports on the owner's main PC (docs/design/field-report.md).

The spare PC running AfriSovera sends ONE plain-text report when the owner types
`report <this PC's address>` there. This program waits for it on port 7700 and saves each
report, untouched, to build/field-reports/<time>-<sender>.txt.

Receive-only, by design:
  * it never sends anything back (apart from TCP's own acknowledgements);
  * it never runs, opens or interprets what arrives: the bytes are written to a .txt file;
  * a report is capped (1 MiB by default); anything past the cap is dropped and noted;
  * only senders on a private home network (192.168.x.x, 10.x.x.x, 172.16-31.x.x) are
    accepted, unless --any is given.

It can also copy reports that AfriSovera saved on its boot USB stick (folder AFREPORT,
files 1.TXT, 2.TXT and so on) once the owner plugs that stick into this PC:
    python tools/report-receiver.py --from-stick E:   (or double-click tools/report-from-stick.bat)
It only reads the stick; it never deletes, renames or writes anything there.

Reports are never saved inside a git working tree unless git ignores that folder, so a
report can never be committed by accident.

Plain Python 3, standard library only. Start it with:
    python tools/report-receiver.py            (or double-click tools/report-receiver.bat)
Stop it with Ctrl+C. If Windows asks whether Python may use private networks, that is
your choice; the report cannot arrive unless it is allowed.
"""
import argparse
import datetime
import ipaddress
import os
import re
import socket
import stat
import subprocess
import sys

PORT = 7700
CAP = 1024 * 1024
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(HERE), "build", "field-reports")


def local_addresses():
    """This PC's IPv4 addresses on its networks (no packet is sent to find them)."""
    found = []
    try:
        # connect() on UDP only picks the outgoing interface; nothing goes on the wire
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("192.0.2.1", 9))
        found.append(s.getsockname()[0])
        s.close()
    except OSError:
        pass
    try:
        for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET):
            found.append(info[4][0])
    except OSError:
        pass
    out = []
    for a in found:
        if a not in out and not a.startswith("127.") and not a.startswith("169.254."):
            out.append(a)
    return out


def safe_name(peer):
    return "".join(c if c.isalnum() else "-" for c in peer)[:40]


def printable(b, n=100):
    return "".join(chr(c) if 32 <= c < 127 else "." for c in b[:n])


def receive(conn, cap, timeout):
    """Read until the sender closes, the cap is reached or it goes quiet."""
    conn.settimeout(timeout)
    data = bytearray()
    cut = False
    try:
        while True:
            chunk = conn.recv(65536)
            if not chunk:
                break
            room = cap - len(data)
            if len(chunk) > room:
                data += chunk[:room]
                cut = True
                break
            data += chunk
    except socket.timeout:
        return bytes(data), cut, "the sender went quiet for %d s" % timeout
    except OSError as e:
        return bytes(data), cut, "connection error: %s" % e
    return bytes(data), cut, None


def out_refusal(out):
    """None when the report folder is safe, else the line that says why it is refused.

    A folder inside a git working tree is allowed only when git ignores it, so a report
    can never be committed by accident. If git cannot be run, we cannot prove that, so
    the folder is refused too.
    """
    out = os.path.realpath(out)
    near = out
    while not os.path.exists(near):
        up = os.path.dirname(near)
        if up == near:
            break
        near = up
    root = None
    d = near
    while True:
        if os.path.lexists(os.path.join(d, ".git")):
            root = d
            break
        up = os.path.dirname(d)
        if up == d:
            break
        d = up
    if root is None:
        return None
    # the folder itself, or a file inside it (a folder that does not exist yet only
    # matches a rule such as build/ through a path inside it)
    for target in (out, os.path.join(out, "report.txt")):
        try:
            r = subprocess.run(
                ["git", "-C", root, "check-ignore", "-q", target],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60,
            )
        except (OSError, subprocess.SubprocessError):
            return ("%s is inside the git working tree %s and git could not be run to prove that folder is "
                    "ignored; reports must never be committed. Choose a folder outside it." % (out, root))
        if r.returncode == 0:
            return None
    return ("%s is inside the git working tree %s and not ignored; reports must never be committed. "
            "Choose a folder outside it, or one git ignores, such as build/field-reports." % (out, root))


def is_link(path):
    """True for a symbolic link, a Windows junction or any other reparse point."""
    try:
        st = os.lstat(path)
    except OSError:
        return False
    if stat.S_ISLNK(st.st_mode):
        return True
    return bool(getattr(st, "st_file_attributes", 0) & 0x400)  # FILE_ATTRIBUTE_REPARSE_POINT


STICK_NAME = re.compile(r"^([0-9]{1,5})\.(txt|bin)$", re.IGNORECASE | re.ASCII)  # .BIN: acpisave's tables


def stick_root(src):
    """The folder to look in: 'E:', 'E:/' or 'E:\\' mean the root of drive E; anything else is a folder."""
    if re.match(r"^[A-Za-z]:[\\/]?$", src):
        return src[0].upper() + ":\\"
    return src


def source_refusal(root):
    """None when the source may be read, else the line that says why it is refused."""
    real = os.path.realpath(root)
    if os.name == "nt":
        system = os.environ.get("SystemDrive", "C:").rstrip("\\/").lower()
        windows = os.environ.get("WINDIR") or os.environ.get("SystemRoot") or ""
        windows_drive = os.path.splitdrive(windows)[0].lower()
        for p in (os.path.abspath(root), real):
            drive = os.path.splitdrive(p)[0].lower()
            if drive and (drive == system or drive == windows_drive):
                return ("%s is on this PC's system drive %s, not a USB stick; plug in the stick and give its "
                        "drive letter, such as E:" % (root, drive.upper()))
    elif real == "/":
        return "%s is this computer's root folder, not a USB stick; give the stick's folder" % root
    return None


def copy_from_stick(src, out, cap):
    """Copy AFREPORT\\<n>.TXT from the stick to `out`. Reads the stick only; never writes to it."""
    root = stick_root(src)
    why = source_refusal(root)
    if why:
        print("Refused: " + why)
        return 1
    try:
        names = sorted(os.listdir(root))
    except OSError as e:
        print("Could not read %s: %s" % (root, e))
        return 1
    found = [n for n in names if n.upper() == "AFREPORT"]
    if not found:
        print("no AFREPORT folder on %s" % root)
        return 1
    folder = os.path.join(root, found[0])
    if is_link(folder):
        print("Refused: %s is a link, not a real folder on the stick" % folder)
        return 1
    if not os.path.isdir(folder):
        print("no AFREPORT folder on %s (%s is not a folder)" % (root, folder))
        return 1
    try:
        entries = list(os.scandir(folder))
    except OSError as e:
        print("Could not read %s: %s" % (folder, e))
        return 1

    reports = []
    ignored = 0
    for e in entries:
        m = STICK_NAME.match(e.name)
        if not m:
            ignored += 1
            continue
        if is_link(e.path):
            reports.append((int(m.group(1)), e, "it is a link, not a plain file", m.group(2).lower()))
            continue
        try:
            plain = e.is_file(follow_symlinks=False)
        except OSError:
            plain = False
        if not plain:
            ignored += 1
            continue
        reports.append((int(m.group(1)), e, None, m.group(2).lower()))
    reports.sort(key=lambda r: (r[0], r[1].name))

    copied = already = skipped = 0
    os.makedirs(out, exist_ok=True)
    for n, e, problem, ext in reports:
        if problem is None:
            data, stamp, problem = read_stick_file(e.path, cap)
        if problem:
            print("Skipped %s: %s" % (e.name, problem))
            skipped += 1
            continue
        # the extension is one of two fixed words from the pattern, never the stick's own text
        path = os.path.join(out, "%s-stick-%d.%s" % (stamp, n, "bin" if ext == "bin" else "txt"))
        try:
            with open(path, "xb") as f:
                f.write(data)
        except FileExistsError:
            print("%s already copied (%s)" % (e.name, path))
            already += 1
            continue
        except OSError as err:
            print("Skipped %s: could not write %s: %s" % (e.name, path, err))
            skipped += 1
            continue
        copied += 1
        print("Copied %s (%d bytes) to %s" % (e.name, len(data), path))
        if data:
            print("  first line: %s" % printable(data.split(b"\n", 1)[0]))

    if ignored:
        print("Ignored %d other name(s) in %s (only 1.TXT, 2.TXT ... reports and 1.BIN ... saved tables)" % (ignored, folder))
    if not reports:
        print("no reports in %s" % folder)
        return 1
    print("Copied %d report(s) from %s to %s; %d already copied; %d skipped" % (copied, root, out, already, skipped))
    if copied or (already and not skipped):
        return 0
    return 1


def read_stick_file(path, cap):
    """(bytes, time stamp, None) or (None, None, why it was skipped). Opens the file read-only."""
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
    except OSError as e:
        return None, None, "could not open it: %s" % e
    with os.fdopen(fd, "rb") as f:
        st = os.fstat(f.fileno())
        if not stat.S_ISREG(st.st_mode):
            return None, None, "it is not a plain file"
        if st.st_size > cap:
            return None, None, "it is %d bytes, over the %d byte limit (not copied, not cut)" % (st.st_size, cap)
        try:
            data = f.read(cap + 1)
        except OSError as e:
            return None, None, "could not read it: %s" % e
    if len(data) > cap:
        return None, None, "it is over the %d byte limit (not copied, not cut)" % cap
    try:
        stamp = datetime.datetime.fromtimestamp(st.st_mtime).strftime("%Y%m%d-%H%M%S")
    except (OverflowError, OSError, ValueError):
        stamp = "00000000-000000"
    return data, stamp, None


def main():
    ap = argparse.ArgumentParser(description="Receive AfriSovera field reports (receive-only).")
    ap.add_argument("--port", type=int, default=PORT, help="port to listen on (default 7700)")
    ap.add_argument("--bind", default="0.0.0.0", help="address to listen on (default: every network)")
    ap.add_argument("--out", default=OUT, help="folder for the reports (default build/field-reports)")
    ap.add_argument("--max-bytes", type=int, default=CAP, help="largest report kept (default 1 MiB)")
    ap.add_argument("--timeout", type=int, default=60, help="seconds of silence before giving up on a sender")
    ap.add_argument("--once", action="store_true", help="stop after one report (used by the tests)")
    ap.add_argument("--any", action="store_true", help="also accept senders outside a private network")
    ap.add_argument("--from-stick", metavar="DRIVE",
                    help="copy the reports saved on AfriSovera's USB stick (such as E:) instead of listening")
    a = ap.parse_args()

    why = out_refusal(a.out)
    if why:
        print("Refused: " + why)
        return 1
    if a.from_stick is not None:
        return copy_from_stick(a.from_stick, a.out, a.max_bytes)

    os.makedirs(a.out, exist_ok=True)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind((a.bind, a.port))
    except OSError as e:
        print("Could not listen on port %d: %s" % (a.port, e))
        print("(Is the receiver already running in another window?)")
        return 1
    srv.listen(4)
    port = srv.getsockname()[1]

    print("AfriSovera field-report receiver -- receive-only, listening on port %d" % port)
    addrs = local_addresses()
    if addrs:
        print("On the AfriSovera PC, type ONE of these (the first is most likely):")
        for ip in addrs:
            print("    report %s%s" % (ip, "" if port == PORT else ":%d" % port))
    else:
        print("Could not tell this PC's address; run  ipconfig  and use its IPv4 Address.")
    print("Reports are saved in %s" % a.out)
    print("Press Ctrl+C to stop.", flush=True)

    try:
        while True:
            conn, (peer, _) = srv.accept()
            try:
                if not a.any and not ipaddress.ip_address(peer).is_private:
                    print("Ignored a connection from %s (not on a private network)" % peer, flush=True)
                    continue
                print("Receiving from %s ..." % peer, flush=True)
                data, cut, problem = receive(conn, a.max_bytes, a.timeout)
            finally:
                conn.close()
            stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
            path = os.path.join(a.out, "%s-%s.txt" % (stamp, safe_name(peer)))
            n = 1
            while os.path.exists(path):
                n += 1
                path = os.path.join(a.out, "%s-%s-%d.txt" % (stamp, safe_name(peer), n))
            with open(path, "wb") as f:
                f.write(data)
                if cut:
                    f.write(b"\n[receiver: report cut at %d bytes]\n" % a.max_bytes)
                if problem:
                    f.write(b"\n[receiver: incomplete -- " + problem.encode() + b"]\n")
            complete = data.rstrip().endswith(b"before this line) ==") and not cut
            print(
                "Saved %d bytes from %s to %s%s" % (len(data), peer, path, "" if complete else "  (INCOMPLETE)"),
                flush=True,
            )
            if data:
                print("  first line: %s" % printable(data.split(b"\n", 1)[0]), flush=True)
            if a.once:
                return 0 if complete else 2
    except KeyboardInterrupt:
        print("\nStopped.")
    finally:
        srv.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
