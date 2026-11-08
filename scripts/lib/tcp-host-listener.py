#!/usr/bin/env python3
"""Host-side TCP listener the tcp suite owns, so its grade measures the GUEST.

Phase 2.834 (a4). 10.0.2.2:<port> inside the guest is slirp's alias for the
host's 127.0.0.1:<port>. Before this, `tcp-test.sh` silently depended on some
OTHER process happening to listen there: with nothing bound, slirp on this
Windows host does not RST, it stays SILENT, so the suite FAILed for a reason
that lives entirely outside the kernel under test (measured in 2.834 arm 3).

Contract, first line of the log file:
    READY     - we bound the port; we are the listener for this run
    INUSE     - somebody else already listens there; precondition already met
    BIND-FAIL - could not bind and nobody else has it; precondition NOT met
Every accepted connection appends `ACCEPTED <peer>` - independent corroboration
of the guest serial's own handshake claim.

Usage: tcp-host-listener.py <host> <port> <logfile>
"""
import errno
import socket
import sys


def main() -> int:
    host, port, logpath = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    log = open(logpath, "w", buffering=1)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    # deliberately NOT SO_REUSEADDR: we want a genuine INUSE when another
    # process holds the port, not a second silent bind beside it.
    try:
        srv.bind((host, port))
    except OSError as exc:
        if exc.errno in (errno.EADDRINUSE, getattr(errno, "WSAEADDRINUSE", 10048)):
            log.write("INUSE %s:%d already has a listener\n" % (host, port))
            return 0
        log.write("BIND-FAIL %s:%d %s\n" % (host, port, exc))
        return 1
    srv.listen(8)
    log.write("READY %s:%d\n" % (host, port))
    srv.settimeout(1.0)
    while True:
        try:
            conn, peer = srv.accept()
        except socket.timeout:
            continue
        except OSError:
            return 0
        log.write("ACCEPTED %s\n" % (peer,))
        # the suite only needs the handshake to complete; close politely.
        try:
            conn.close()
        except OSError:
            pass


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(0)
