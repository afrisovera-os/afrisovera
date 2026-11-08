#!/usr/bin/env python3
"""FALSIFY-FIRST PEER 3.488 -- can the guest INGEST bulk bytes at all?

3.486 pre-registered the measurement (B) rests on: >= 1x real time inbound
(>= 44 kB/s at 22050 Hz mono, held >= 10 s). 3.487 measured the ROUTE before the
RATE (the 3.261 rule) and found there is none: `sound_stream_write` has exactly
two live callers and neither is a network path, and the ONLY inbound verb
ECHOES. So the verb `net tcp sink <port> <secs>` is owed -- accept one
connection, ACK inbound payload, echo NOTHING, report bytes + elapsed at close.

This peer is the falsifier for that verb, and it is written BEFORE the verb.
On the UNPATCHED tree it is EXPECTED to exit 10: nothing is listening on the
sink port because `sink` parses as nothing at all.

    10  SINK-ABSENT    no SYN/ACK -- the verb does not exist (today)
    20  SINK-ECHOES    a connection, but the guest sent our bytes BACK. That is
                       `serve`, not `sink`; echo throughput is NOT ingest
                       throughput (3.487's pre-committed non-evidence), so this
                       arm exists to stop the rig measuring the wrong verb.
     0  SINK-MEASURED  established, ACKs advanced, nothing echoed -> a rate
     1  harness / inconclusive

The rate printed here is the PEER's reading (bytes the guest ACKed / seconds
since ESTABLISHED). The guest's own `bytes + elapsed` line is a SECOND, separate
derivation, graded by the shell probe. Neither is asserted from the other.

The wire helpers are IMPORTED from scripts/tcp-serve2-peer.py, unedited -- a
second hand-rolled checksum is a second place to be wrong.
"""

import importlib.util
import os
import socket
import struct
import sys
import time

_HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "srv2peer", os.path.join(_HERE, "tcp-serve2-peer.py"))
srv2 = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(srv2)          # module level is constants + defs only

PEER_IP, GUEST_IP, PEER_MAC = srv2.PEER_IP, srv2.GUEST_IP, srv2.PEER_MAC
Wire, ip_tcp, parse_tcp, dhcp_reply = srv2.Wire, srv2.ip_tcp, srv2.parse_tcp, srv2.dhcp_reply

OUR_PORT = 50011                  # 50001 dial, 50002 chat, 50003+ serve2
OUR_ISN = 0x0C4C_0000
SEG = 512                         # kernel/src/virtio.rs TCP_MAX_PAYLOAD


def _knob(name, default, cast):
    """ONE additive env knob beside a default (the 2.762 rule: a typed opt-in,
    never a changed default). Unset -> the default, so every existing caller
    keeps the exact old behaviour. A value that will not parse, or is <= 0, is
    REFUSED LOUDLY and never silently defaulted: a mistyped SINK_RTO that quietly
    ran at 0.5 would be read as MODEL S by 3.500's pre-committed reading -- the
    FLATTERING failure, so it must not be reachable."""
    v = os.environ.get(name)
    if v is None:
        return default
    try:
        out = cast(v)
    except ValueError:
        sys.stderr.write("sink-peer: %s=%r does not parse as %s\n"
                         % (name, v, cast.__name__))
        sys.exit(2)
    if out <= 0:
        sys.stderr.write("sink-peer: %s=%r must be > 0\n" % (name, v))
        sys.exit(2)
    return out


INFLIGHT = _knob("SINK_INFLIGHT", 4096, int)   # 8 segments -- the pool's depth
RTO = _knob("SINK_RTO", 0.5, float)
                                  # seconds with no ACK ADVANCE before the oldest
                                  # unacked segment is re-sent (3.493: without this
                                  # the forced-gap arm deadlocks by construction).
                                  # 3.500's discriminator drives THIS knob: if the
                                  # advance cadence tracks RTO the PEER owns the
                                  # shortfall (MODEL R); if it does not, the GUEST
                                  # does (MODEL S). Assigned EXACTLY ONCE -- 3.500
                                  # recorded a duplicate bare `RTO = 0.5` below,
                                  # which would now silently overwrite the knob.
GIVEUP = _knob("SINK_GIVEUP", 5.0, float)
                                  # seconds with no ACK ADVANCE before the peer
                                  # STOPS and closes the run. 3.502/3.503: this
                                  # was a hard-coded 5.0 racing SINK_RTO=5.0
                                  # ~0.1 s apart, so the giveup won and the
                                  # post-RTO advance was NEVER observed -- the
                                  # blocker MODEL R's mechanism has to be read
                                  # through. Assigned EXACTLY ONCE, same as RTO.
FASTRETX = _knob("SINK_FASTRETX", 0, int)
                                  # design-3.506 (B), landed as a TYPED OPT-IN:
                                  # after N DUPLICATE ACKs the oldest unacked
                                  # segment is re-sent WITHOUT waiting out the
                                  # RTO. UNSET (0) == the exact pre-3.507 peer,
                                  # byte for byte -- disabling is UNSETTING it,
                                  # never `=0` (a typed 0 is refused loudly like
                                  # any other bad value). 3.506 READ off the
                                  # 3.505 pcap that nine pure-ACK frames carried
                                  # the SAME ack point inside 1.345 s while
                                  # RETRANSMIT #1 waited the full 5.0 s RTO, so
                                  # any N <= 9 fires on that trace. Assigned
                                  # EXACTLY ONCE, same as RTO/GIVEUP.
MAX_RETX = 40                     # backstop: a wedged guest must not spin forever

T0 = time.time()


def say(msg):
    print("[%7.3fs] %s" % (time.time() - T0, msg), flush=True)


def grade(syn_ack, echoed, acked, elapsed):
    """The whole verdict, as pure arithmetic -- so every arm is reachable with
    no socket, no QEMU and no kernel (Law 9: reach both arms offline first)."""
    if not syn_ack:
        return 10, "SINK-ABSENT", 0.0
    if echoed > 0:
        return 20, "SINK-ECHOES", 0.0
    if acked <= 0 or elapsed <= 0:
        return 1, "SINK-SILENT", 0.0
    return 0, "SINK-MEASURED", acked / elapsed


RATE_MIN = 44100.0        # B/s -- 3.486's claim: 22050 Hz mono i16 = 1x real time
RATE_HOLD = 10.0          # s   -- and it must be SUSTAINED, not a burst


def grade_rate(code, acked, elapsed):
    """3.486's rate claim as MACHINE arithmetic, BESIDE grade(), never inside it
    (the 2.762 rule: a new verb beside the old one, not a change to it).

    grade() answers ROUTE and keeps exit 0 == SINK-MEASURED for every existing
    caller. This answers RATE and nothing else, so exit 0 stops meaning two
    different things the moment a caller opts in (SINK_RATE_STRICT=1). A route
    red DOMINATES: a rate computed off a connection that never happened is not
    a number, so it is passed straight through rather than re-graded."""
    if code != 0:
        return code, "RATE-NOT-APPLICABLE", 0.0
    rate = (acked / elapsed) if elapsed > 0 else 0.0
    if elapsed < RATE_HOLD:
        return 31, "SINK-RATE-UNHELD", rate
    if rate < RATE_MIN:
        return 30, "SINK-RATE-SHORT", rate
    return 0, "SINK-RATE-MET", rate


def selftest():
    cases = [
        ((False, 0, 0, 0.0), 10),
        ((False, 999, 999, 9.0), 10),      # no handshake dominates
        ((True, 13, 13, 1.0), 20),
        ((True, 1, 200000, 10.0), 20),     # one echoed byte is still an echo
        ((True, 0, 0, 5.0), 1),
        ((True, 0, 500000, 0.0), 1),
        ((True, 0, 441000, 10.0), 0),
        ((True, 0, 1000, 10.0), 0),        # a SLOW sink is still a sink: the
    ]                                      # threshold is 3.486's, not the rig's
    bad = 0
    seen = set()
    for args, want in cases:
        code, label, rate = grade(*args)
        seen.add(code)
        ok = code == want
        bad += 0 if ok else 1
        print("  %s grade%r -> %d %s (%.0f B/s), want %d"
              % ("ok  " if ok else "FAIL", args, code, label, rate, want))
    payload = bulk(0, SEG)
    if len(payload) != SEG:
        print("  FAIL bulk() returned %d bytes, want %d" % (len(payload), SEG))
        bad += 1
    else:
        print("  ok   bulk() segment is %d bytes and self-identifying: %r..."
              % (SEG, payload[:16]))
    # forced-gap arms: BOTH directions offline, before any boot
    plan = []
    ds = 2
    seqs = []
    seq = 0
    for kk in range(8):
        if drop_now(kk, ds):
            ds = -1                     # one-shot, exactly as the push loop does
        else:
            plan.append(kk)
            seqs.append(seq)
        seq += SEG                      # the seq space advances over the hole
    if plan == [0, 1, 3, 4, 5, 6, 7] and seqs[2] - seqs[1] == 2 * SEG:
        print("  ok   forced gap drops EXACTLY one segment (k=2) and the seq space "
              "skips %d B across the hole" % (2 * SEG))
    else:
        print("  FAIL forced gap plan %r seqs %r" % (plan, seqs))
        bad += 1
    if [kk for kk in range(8) if drop_now(kk, -1)] == []:
        print("  ok   unarmed (SINK_DROP_SEG unset / -1) drops nothing")
    else:
        print("  FAIL unarmed forced gap dropped a segment")
        bad += 1
    if seen != {0, 1, 10, 20}:
        print("  FAIL not every arm was reached: %s" % sorted(seen))
        bad += 1
    else:
        print("  ok   all four arms reached offline: 0/1/10/20")
    # --- retransmission picker: both arms offline, no wire ------------------
    ua = [(0, b"a" * 512), (512, b"b" * 512), (1024, b"c" * 512)]
    if retx_pick(ua, 0) != (0, b"a" * 512):
        print("  FAIL retx_pick did not choose the oldest unacked segment")
        bad += 1
    else:
        print("  ok   retx_pick chooses the OLDEST unacked segment")
    if retx_pick(ua, 1024) != (1024, b"c" * 512):
        print("  FAIL retx_pick did not skip fully-acked segments")
        bad += 1
    else:
        print("  ok   retx_pick skips segments the guest already acked")
    if retx_pick(ua, 1536) is not None:
        print("  FAIL retx_pick re-sent a fully-acked stream")
        bad += 1
    else:
        print("  ok   retx_pick returns None when nothing is outstanding")
    if retx_pick([], 0) is not None:
        print("  FAIL retx_pick on an empty window")
        bad += 1
    # --- rate grader: EVERY arm offline, before any boot (3.493's grade() gap)
    rcases = [
        # (route_code, acked, elapsed) -> want code
        ((10, 0, 0.0), 10),               # route red passes THROUGH, ungraded
        ((20, 999999, 30.0), 20),         # an echo is not a sink at any rate
        ((1, 0, 5.0), 1),
        ((0, 7169, 12.195), 30),          # the REAL 3.496 run: 588 B/s -> SHORT
        ((0, 441000, 10.0), 0),           # exactly 44100 B/s held exactly 10 s
        ((0, 440999, 10.0), 30),          # one byte under the threshold is SHORT
        ((0, 441000, 9.999), 31),         # fast enough, NOT held -> UNHELD
        ((0, 100, 9.0), 31),              # hold is checked BEFORE rate
    ]
    rseen = set()
    for args, want in rcases:
        rcode, rlabel, rrate = grade_rate(*args)
        rseen.add(rcode)
        ok = rcode == want
        bad += 0 if ok else 1
        print("  %s grade_rate%r -> %d %s (%.0f B/s), want %d"
              % ("ok  " if ok else "FAIL", args, rcode, rlabel, rrate, want))
    if rseen != {0, 1, 10, 20, 30, 31}:
        print("  FAIL rate grader arms reached: %s" % sorted(rseen))
        bad += 1
    else:
        print("  ok   rate grader: both new arms (30 SHORT / 31 UNHELD), the MET "
              "arm and all three pass-throughs reached offline")
    if grade(True, 0, 1000, 10.0)[0] == 0:
        print("  ok   grade() is UNCHANGED by the rate grader: a slow sink is "
              "still exit 0 SINK-MEASURED (route, never rate)")
    else:
        print("  FAIL grade() changed meaning")
        bad += 1
    # env-knob arms (3.500's pre-registered discriminator): the DEFAULT is
    # unchanged, an override IS taken, and a bad value is refused -- all three
    # reached offline before any boot.
    kcases = [
        ("SINK_RTO", None, 0.5, float, 0.5, "unset  "),
        ("SINK_RTO", "5.0", 0.5, float, 5.0, "override"),
        ("SINK_INFLIGHT", None, 4096, int, 4096, "unset  "),
        ("SINK_INFLIGHT", "512", 4096, int, 512, "override"),
        ("SINK_GIVEUP", None, 5.0, float, 5.0, "unset  "),
        ("SINK_GIVEUP", "20", 5.0, float, 20.0, "override"),
        ("SINK_FASTRETX", None, 0, int, 0, "unset  "),
        ("SINK_FASTRETX", "3", 0, int, 3, "override"),
    ]
    for name, val, dflt, cast, want, what in kcases:
        old = os.environ.pop(name, None)
        if val is not None:
            os.environ[name] = val
        try:
            got = _knob(name, dflt, cast)
        finally:
            os.environ.pop(name, None)
            if old is not None:
                os.environ[name] = old
        ok = got == want
        bad += 0 if ok else 1
        print("  %s knob %s %s -> %r, want %r"
              % ("ok  " if ok else "FAIL", name, what, got, want))
    for name, val in [("SINK_RTO", v) for v in ("five", "0", "-1", "5,0")] + \
                     [("SINK_GIVEUP", v) for v in ("twenty", "0", "-1", "2,0")]:
        old = os.environ.pop(name, None)
        os.environ[name] = val
        try:
            _knob(name, 0.5, float)
            print("  FAIL knob %s=%r ACCEPTED; a silent default here would "
                  "be read as MODEL S" % (name, val))
            bad += 1
        except SystemExit as e:
            print("  ok   knob %s=%r refused loudly (exit %s)" % (name, val, e.code))
        finally:
            os.environ.pop(name, None)
            if old is not None:
                os.environ[name] = old
    for val in ("three", "0", "-1", "3.5"):
        name = "SINK_FASTRETX"
        old = os.environ.pop(name, None)
        os.environ[name] = val
        try:
            _knob(name, 0, int)
            print("  FAIL knob %s=%r ACCEPTED; disabling is UNSETTING it, and a "
                  "silently-defaulted N would read as 'B does not work'"
                  % (name, val))
            bad += 1
        except SystemExit as e:
            print("  ok   knob %s=%r refused loudly (exit %s)" % (name, val, e.code))
        finally:
            os.environ.pop(name, None)
            if old is not None:
                os.environ[name] = old
    # The race this knob exists to break (3.502/3.503): a giveup EQUAL to the RTO
    # ends the run before the retransmission it is meant to expose can land.
    # Checked as arithmetic on the two effective constants, not on a boot.
    for rto, give, want_readable in ((5.0, 5.0, False), (5.0, 20.0, True)):
        readable = give > rto + 1.0
        ok = readable == want_readable
        bad += 0 if ok else 1
        print("  %s race arm rto=%s giveup=%s -> post-RTO window %s"
              % ("ok  " if ok else "FAIL", rto, give,
                 "READABLE" if readable else "RACED SHUT"))
    # ---- design-3.506 (B) FAST RETRANSMIT, falsified offline before any boot --
    # (i) the counter itself: every arm reached, including the two that must NOT
    #     count (an advance, and a stale ack below the credited point).
    for dups, adv, acked_, out, want, why in (
            (2, 1024, 1024, True,  3, "a repeat at the same point is a duplicate"),
            (2, 1536, 1024, True,  0, "an ADVANCE ends the run"),
            (2,  512, 1024, True,  2, "a STALE ack below the point is ignored"),
            (2, 1024, 1024, False, 0, "nothing outstanding -> nothing to count")):
        got = dupack_bump(dups, adv, acked_, out)
        ok = got == want
        bad += 0 if ok else 1
        print("  %s dupack_bump(%d,%d,%d,%s) -> %d  (%s)"
              % ("ok  " if ok else "FAIL", dups, adv, acked_, out, got, why))
    # (ii) the DEFAULT cannot fire, by construction and not by inspection.
    for dups, n, want in ((99, 0, False), (2, 3, False), (3, 3, True), (9, 3, True)):
        got = fast_retx_due(dups, n)
        ok = got == want
        bad += 0 if ok else 1
        print("  %s fast_retx_due(dups=%d, n=%d) -> %s" % ("ok  " if ok else "FAIL",
                                                           dups, n, got))
    # (iii) THE FALSIFYING ARM, on the ARCHIVED trace's own shape: 3.506 decoded
    #       NINE pure-ACK frames at ONE ack point inside 1.345 s while RETRANSMIT
    #       #1 waited the full 5.0 s RTO. Replaying that many duplicates, the
    #       default NEVER fires (which is the 3.505 behaviour, i.e. the failing
    #       arm) and N=3 fires with six duplicates to spare -- inside 1.345 s,
    #       i.e. BEFORE the RTO. The 9 is READ off that decode, not assumed.
    for n, want_fire, want_at in ((0, False, None), (3, True, 3)):
        d, fired_at = 0, None
        for i in range(1, 9):                # 9 acks == 8 duplicates
            d = dupack_bump(d, 1024, 1024, True)
            if fired_at is None and fast_retx_due(d, n):
                fired_at, d = i, 0
        ok = (fired_at is not None) == want_fire and fired_at == want_at
        bad += 0 if ok else 1
        print("  %s archived-trace arm N=%d -> %s (RTO 5.0s vs duplicates spanning "
              "1.345s)" % ("ok  " if ok else "FAIL", n,
                           "fired at duplicate %s" % fired_at if fired_at
                           else "never fired -- the 3.505 behaviour, unchanged"))
    # (iv) THE 3.509 HAZARD, reproduced OFFLINE on the archived shape before the
    #      guard is trusted. 3.508 saw a SECOND fast retransmit of the SAME
    #      segment 197 ms after the first; 3.509 REPRODUCED it at 661 ms, so it
    #      is the behaviour and not one boot. Replaying 3.506's own decoded run
    #      (nine acks at ONE point, i.e. eight duplicates, N=3): the UNGUARDED
    #      rule fires TWICE at the SAME offset -- that is the failing arm, and it
    #      must be REACHED here, not argued -- while the guard fires ONCE.
    for guard, want_fires in ((False, [2560, 2560]), (True, [2560])):
        fires, d, last = [], 0, None
        for _ in range(8):                   # 9 acks == 8 duplicates, one point
            d = dupack_bump(d, 1024, 1024, True)
            if fast_retx_due(d, 3):
                if guard and not fast_retx_fresh(2560, last):
                    d = 0                    # HELD: run restarts, nothing sent
                    continue
                fires.append(2560)
                last, d = 2560, 0
        ok = fires == want_fires
        bad += 0 if ok else 1
        print("  %s 3.509 hazard arm guard=%-5s -> %d fast resend(s) of offset "
              "2560 %s" % ("ok  " if ok else "FAIL", guard, len(fires),
                           "(the duplicate resend, reproduced)" if not guard
                           else "(the duplicate resend is GONE)"))
    # (iv-b) the guard must not become a permanent gag: once the ack ADVANCES the
    #        episode is over (the caller clears it), and the next hole is eligible.
    for off, last, want, why in (
            (2560, 2560, False, "same offset, same episode -> HELD"),
            (3072, 2560, True,  "a DIFFERENT hole is still eligible"),
            (2560, None, True,  "episode cleared by an ACK ADVANCE -> eligible"),
            (2560, 3072, True,  "last fast resend was elsewhere -> eligible")):
        got = fast_retx_fresh(off, last)
        ok = got == want
        bad += 0 if ok else 1
        print("  %s fast_retx_fresh(off=%d, last=%s) -> %s  (%s)"
              % ("ok  " if ok else "FAIL", off, last, got, why))
    src = open(os.path.abspath(__file__), "r", encoding="utf-8").read()
    dupes = [l for l in src.splitlines()
             if l.startswith("RTO =") or l.startswith("INFLIGHT =")
             or l.startswith("GIVEUP =") or l.startswith("FASTRETX =")]
    if len(dupes) == 4:
        print("  ok   each knob assigned EXACTLY once at module level (3.500's "
              "duplicate bare `RTO = 0.5` would silently overwrite the knob)")
    else:
        print("  FAIL module-level knob assignments: %r" % dupes)
        bad += 1
    print("  selftest: %s" % ("PASS" if bad == 0 else "FAIL (%d)" % bad))
    return 1 if bad else 0


def bulk(k, n):
    """Segment k of the bulk stream. Self-identifying so a capture (and the
    guest's own byte count) can be read at a glance; PCM-shaped only in size."""
    head = ("sink-%06d:" % k).encode("ascii")
    return (head + bytes((i * 7 + k) & 0xFF for i in range(n - len(head))))[:n]


def retx_pick(unacked, acked):
    """The OLDEST segment the guest has not fully acknowledged, or None.
    `unacked` is [(offset, body)] in send order, `acked` the contiguous byte
    count the guest has ACKed. Pure, so the selftest reaches both arms with no
    wire (the 2.994 lesson: grade the function the run actually calls)."""
    for off, body in unacked:
        if off + len(body) > acked:
            return (off, body)
    return None


def dupack_bump(dups, adv, acked, outstanding):
    """Pure duplicate-ACK counter (3.506 (B)). `adv` is the guest's ack point in
    OUR byte space, `acked` the highest point already credited. An ADVANCE ends
    the run; a REPEAT of the same point with data still outstanding is one
    duplicate; anything BELOW `acked` (stale/reordered) is ignored rather than
    counted, so a late frame cannot manufacture a retransmission. Pure, so the
    selftest reaches every arm with no wire (the 2.994 lesson)."""
    if not outstanding:
        return 0
    if adv > acked:
        return 0
    if adv == acked:
        return dups + 1
    return dups


def fast_retx_due(dups, n):
    """True when N duplicate ACKs have accumulated and the knob is ARMED.
    n <= 0 (the default, i.e. SINK_FASTRETX unset) can never fire -- the
    defaults are unchanged BY CONSTRUCTION, not by inspection."""
    return n > 0 and dups >= n


def fast_retx_fresh(off, last_off):
    """True when this offset has NOT already been fast-retransmitted in the
    current recovery episode. The hazard 3.508 recorded and 3.509 REPRODUCED:
    after a fast retransmit the counter restarts, the guest keeps emitting gap
    ACKs at the SAME point (it has not received the resend yet), the duplicate
    run re-reaches N, and the peer resends a segment ALREADY IN FLIGHT -- 197 ms
    later at 3.508, 661 ms later at 3.509. One hole, two identical frames.

    The episode ends when the ack point ADVANCES (the caller clears `last_off`),
    so a NEW hole is always eligible. STATED NOT GLOSSED: if the fast retransmit
    is itself LOST, the fast path will no longer cover it -- the plain RTO timer
    (`retx_pick` on `now - last_progress > RTO`) is the backstop, unchanged and
    untouched by this guard. That is deliberate: real stacks do not fast-
    retransmit the same segment twice inside one recovery episode either.

    Pure, so the selftest reaches both arms with no wire (the 2.994 lesson)."""
    return last_off is None or off != last_off


def drop_now(k, drop_seg):
    """One-shot FORCED GAP (3.490(2)/3.492 pre-registration). True for exactly
    the configured segment index; the caller clears drop_seg after it fires, and
    the sequence space still advances over the hole -- nothing re-sends it.
    This is the FAILING arm: it must be shown to deadlock BEFORE any peer-side
    retransmission timer exists (the 3.345/3.347 rule)."""
    return drop_seg >= 0 and k == drop_seg


def main():
    if "--selftest" in sys.argv[1:]:
        return selftest()
    argv = sys.argv[1:]
    if len(argv) < 3:
        print("usage: sink-peer-3488.py <portfile> <guest_port> <secs> [push_secs]")
        return 2
    portfile, guest_port, limit = argv[0], int(argv[1]), float(argv[2])
    push_secs = float(argv[3]) if len(argv) > 3 else 12.0
    # Anchored at column 0 so a later grader reads the EFFECTIVE knobs off the
    # artefact instead of assuming the defaults (the 2.994 lesson).
    print("SINK-KNOBS: rto=%s inflight=%s giveup=%s fastretx=%s max_retx=%s seg=%s"
          % (RTO, INFLIGHT, GIVEUP, FASTRETX, MAX_RETX, SEG), flush=True)

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))              # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d; will dial 10.0.2.15:%d from port %d "
        "and push bulk bytes for up to %.0fs"
        % (srv.getsockname()[1], guest_port, OUR_PORT, push_secs))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    say("peer: QEMU attached to the wire")
    conn.settimeout(0.2)
    w = Wire(conn)

    guest_mac = None
    syn_ack = False
    echoed = 0
    acked = 0                  # bytes the guest has ACKed
    our_seq = OUR_ISN
    our_ack = 0
    sent_hi = 0                # bytes handed to the wire
    unacked = []               # [(offset, body)] -- EVERY segment the sequence
                               # space advanced over, INCLUDING the forced-gap
                               # one, which is exactly what a real sender keeps
    retx = 0
    fastretx = 0               # of `retx`, how many were 3.506 (B) fast ones
    dups = 0                   # consecutive duplicate ACKs at the same point
    fast_off = None            # offset already fast-retransmitted THIS episode
    fastheld = 0               # duplicate fast resends SUPPRESSED (3.509 hazard)
    last_progress = None       # last ACK ADVANCE (the retransmission timer's base)
    k = 0
    drop_seg = int(os.environ.get("SINK_DROP_SEG", "-1"))
    if drop_seg >= 0:
        say("peer: FORCED-GAP ARM ARMED -- segment %d will be built and NOT sent" % drop_seg)
    est_at = None
    last_ack_at = None
    dialed = None
    dials = 0
    guest_fin = False
    deadline = time.time() + limit

    def dial():
        w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, OUR_ISN, 0, 0x02))

    while time.time() < deadline:
        now = time.time()
        if dialed is not None and not syn_ack and now - dialed > 1.5 and dials < 4:
            dials += 1
            dialed = now
            dial()
            say("peer -> SYN retransmit %d (no SYN/ACK yet)" % dials)

        # ---- push: keep INFLIGHT bytes outstanding, never more --------------
        if syn_ack and not guest_fin and est_at is not None \
                and now - est_at < push_secs:
            while sent_hi - acked < INFLIGHT:
                body = bulk(k, SEG)
                if drop_now(k, drop_seg):
                    say("peer: FORCED GAP -- segment %d (%d B, seq %#010x) built and "
                        "NOT sent; seq advances over it and nothing re-sends it"
                        % (k, len(body), our_seq))
                    drop_seg = -1
                else:
                    w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port,
                                        our_seq, our_ack, 0x18, body))
                unacked.append((sent_hi, body))
                our_seq += len(body)
                sent_hi += len(body)
                k += 1
                if k % 200 == 0:
                    say("peer -> %d segments handed to the wire (%d bytes), acked %d"
                        % (k, sent_hi, acked))
        elif syn_ack and est_at is not None and now - est_at >= push_secs and not guest_fin:
            say("peer: push window over (%.1fs) -- closing" % push_secs)
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
            guest_fin = True     # stop pushing; the guest's own FIN ends the run

        # ---- retransmission: the OLDEST unacked segment, on a plain RTO -----
        if syn_ack and not guest_fin and last_progress is not None                 and now - last_progress > RTO and retx < MAX_RETX:
            pick = retx_pick(unacked, acked)
            if pick is not None:
                off, body = pick
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port,
                                    (OUR_ISN + 1 + off) & 0xFFFFFFFF, our_ack,
                                    0x18, body))
                retx += 1
                last_progress = now
                say("peer -> RETRANSMIT #%d: %d B at offset %d (seq %#010x); "
                    "acked=%d handed=%d" % (retx, len(body), off,
                                            (OUR_ISN + 1 + off) & 0xFFFFFFFF,
                                            acked, sent_hi))

        try:
            fr = w.recv_frame()
        except socket.timeout:
            if syn_ack and last_ack_at and time.time() - last_ack_at > GIVEUP:
                say("peer: no ACK for %gs -- the sink stopped consuming" % GIVEUP)
                break
            continue
        except OSError as e:
            say("peer: the wire was reset by QEMU (%s)" % e)
            break
        if fr is None:
            say("peer: QEMU closed the wire")
            break
        if len(fr) < 14:
            continue
        if guest_mac is None:
            guest_mac = fr[6:12]
            say("peer: first frame from the guest, MAC %s" % guest_mac.hex(":"))
        et = (fr[12] << 8) | fr[13]

        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            if op == 1 and fr[38:42] == PEER_IP:
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                w.send_frame(sender_mac + PEER_MAC + b"\x08\x06" +
                             bytes([0, 1, 8, 0, 6, 4, 0, 2]) + PEER_MAC + PEER_IP +
                             sender_mac + sender_ip)
                if dialed is None:
                    # the ARP that resolves the gateway is the readiness signal
                    # (2.759): the guest is up and the sink window is open.
                    dials = 1
                    dialed = time.time()
                    dial()
                    say("peer -> SYN to 10.0.2.15:%d (the guest just ARPed)" % guest_port)
            continue

        if et == 0x0800 and len(fr) >= 42 and fr[23] == 17:
            # DHCP: without a lease the boot costs half a minute (see the import)
            ihl = (fr[14] & 0x0F) * 4
            u = 14 + ihl
            if (fr[u + 2] << 8) | fr[u + 3] == 67:
                b = fr[u + 8:]
                xid = struct.unpack(">I", b[4:8])[0]
                chaddr = b[28:34]
                mt = 1
                i = 240
                while i + 1 < len(b):
                    if b[i] == 255:
                        break
                    if b[i] == 53:
                        mt = b[i + 2]
                    i += 2 + b[i + 1]
                w.send_frame(dhcp_reply(guest_mac, xid, chaddr, 2 if mt == 1 else 5))
            continue

        seg = parse_tcp(fr)
        if seg is None:
            continue
        sport, dport, seq, ack, flags, payload = seg
        if dport != OUR_PORT or sport != guest_port:
            continue
        if flags & 0x02 and flags & 0x10:          # SYN/ACK
            if not syn_ack:
                syn_ack = True
                est_at = time.time()
                last_ack_at = est_at
                last_progress = est_at
                our_seq = OUR_ISN + 1
                our_ack = (seq + 1) & 0xFFFFFFFF
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port,
                                    our_seq, our_ack, 0x10))
                say("peer: ESTABLISHED (guest ISS %#010x) -- the sink port answered" % seq)
            continue
        if payload:
            echoed += len(payload)
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))
            say("peer: the guest sent %d bytes BACK (%d total) -- that is an ECHO, not a sink"
                % (len(payload), echoed))
        if flags & 0x10:
            adv = (ack - (OUR_ISN + 1)) & 0xFFFFFFFF
            if 0 <= adv <= sent_hi and adv > acked:
                acked = adv
                last_ack_at = time.time()
                last_progress = last_ack_at
                unacked = [(o, b) for (o, b) in unacked if o + len(b) > acked]
                dups = 0
                fast_off = None       # the ack ADVANCED: this recovery episode
                                      # is over, a new hole is eligible again
            elif 0 <= adv <= sent_hi and syn_ack and not guest_fin:
                dups = dupack_bump(dups, adv, acked, bool(unacked))
                if fast_retx_due(dups, FASTRETX) and retx < MAX_RETX:
                    pick = retx_pick(unacked, acked)
                    if pick is not None and not fast_retx_fresh(pick[0], fast_off):
                        dups = 0                  # start the run over; the RTO
                        fastheld += 1             # timer remains the backstop
                        say("peer: fast retransmit HELD (#%d) -- offset %d was "
                            "already fast-retransmitted this episode and the "
                            "guest has not acked past it; not resending a "
                            "segment already in flight (3.509 hazard)"
                            % (fastheld, pick[0]))
                        pick = None
                    if pick is not None:
                        off, body = pick
                        w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port,
                                            (OUR_ISN + 1 + off) & 0xFFFFFFFF,
                                            our_ack, 0x18, body))
                        retx += 1
                        fastretx += 1
                        fast_off = off
                        last_progress = time.time()   # we just re-sent: the RTO
                                                      # timer restarts, exactly
                                                      # as after an RTO retx
                        dups = 0
                        say("peer -> FAST RETRANSMIT #%d (retx #%d) after %d "
                            "duplicate ACKs: %d B at offset %d (seq %#010x); "
                            "acked=%d handed=%d"
                            % (fastretx, retx, FASTRETX, len(body), off,
                               (OUR_ISN + 1 + off) & 0xFFFFFFFF, acked, sent_hi))
        if flags & 0x01:                            # guest FIN
            guest_fin = True
            our_ack = (seq + len(payload) + 1) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
            say("peer: the guest FINed -- the sink window closed")
            break

    elapsed = (last_ack_at - est_at) if (est_at and last_ack_at) else 0.0
    code, label, rate = grade(syn_ack, echoed, acked, elapsed)
    say("peer: syn_ack=%s echoed=%d handed=%d acked=%d retx=%d elapsed=%.3fs"
        % (syn_ack, echoed, sent_hi, acked, retx, elapsed))
    if code == 0:
        say("peer: PEER READING %d bytes in %.3f s = %.0f B/s (%.2fx of 44100 B/s)"
            % (acked, elapsed, rate, rate / 44100.0))
    rcode, rlabel, rrate = grade_rate(code, acked, elapsed)
    # ANCHORED at column 0 so a later suite grades the RATE by machine instead of
    # leaving a human to read it off a banner (3.493's gap, re-costed at 3.496).
    print("SINK-RATE: %s code=%d rate=%.0f elapsed=%.3f need_rate=%.0f need_hold=%.1f"
          % (rlabel, rcode, rrate, elapsed, RATE_MIN, RATE_HOLD), flush=True)
    say("peer verdict: %s (exit %d)" % (label, code))
    if os.environ.get("SINK_RATE_STRICT") == "1" and rcode != code:
        say("peer: SINK_RATE_STRICT=1 -- exiting on the RATE verdict %s (%d), "
            "not the route verdict %s (%d)" % (rlabel, rcode, label, code))
        return rcode
    return code


if __name__ == "__main__":
    sys.exit(main())
