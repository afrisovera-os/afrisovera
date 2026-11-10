#!/usr/bin/env python3
"""Receive AfriSovera field reports on the owner's main PC (docs/design/field-report.md).

The spare PC running AfriSovera sends ONE plain-text report when the owner types
`report <this PC's address>` there. This program waits for it on port 7700 and saves each
report, untouched, to build/field-reports/<time>-<sender>.txt.

Receive-only, by design:
  * it never sends anything back (apart from TCP's own acknowledgements);
  * it never runs, opens or interprets what arrives -- the bytes are written to a .txt file;
  * a report is capped (1 MiB by default); anything past the cap is dropped and noted;
  * only senders on a private home network (192.168.x.x, 10.x.x.x, 172.16-31.x.x) are
    accepted, unless --any is given.

Plain Python 3, standard library only. Start it with:
    python tools/report-receiver.py            (or double-click tools/report-receiver.bat)
Stop it with Ctrl+C. If Windows asks whether Python may use private networks, that is
your choice; the report cannot arrive unless it is allowed.
"""
import argparse
import datetime
import ipaddress
import os
import socket
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


def main():
    ap = argparse.ArgumentParser(description="Receive AfriSovera field reports (receive-only).")
    ap.add_argument("--port", type=int, default=PORT, help="port to listen on (default 7700)")
    ap.add_argument("--bind", default="0.0.0.0", help="address to listen on (default: every network)")
    ap.add_argument("--out", default=OUT, help="folder for the reports (default build/field-reports)")
    ap.add_argument("--max-bytes", type=int, default=CAP, help="largest report kept (default 1 MiB)")
    ap.add_argument("--timeout", type=int, default=60, help="seconds of silence before giving up on a sender")
    ap.add_argument("--once", action="store_true", help="stop after one report (used by the tests)")
    ap.add_argument("--any", action="store_true", help="also accept senders outside a private network")
    a = ap.parse_args()

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
