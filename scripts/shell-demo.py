#!/usr/bin/env python3
"""Drive the AfriSh shell over a TCP serial port and screenshot the result.

Usage: shell-demo.py <serial_port> <qmp_port> <out.png> [cmd1] [cmd2] ...
Sends each command over the serial socket (reliable native TCP — avoids the
MSYS->Windows stdin-pipe problem), then captures the framebuffer via QMP.
"""
import json
import os
import socket
import sys
import time


def stdout_utf8():
    """Write the capture as UTF-8, whatever the host's locale says.

    The console speaks UTF-8 and this script decodes it as UTF-8 (below), but
    every suite reads the capture from THIS script's stdout — and stdout is
    re-encoded on the way out using the host locale, which on the Windows dev
    box is cp1252. Measured: the kernel prints `(1 x 1GiB) — clamp slack` as
    the three UTF-8 bytes e2 80 94, and the capture on disk held the single
    byte 0x97, cp1252's em dash. So the capture was neither what the kernel
    sent nor the same file CI produces from the identical boot: a suite whose
    pattern includes any non-ASCII character asserts against different bytes
    on different machines, which is why the one board-window check that wanted
    such a line had to stop its regex short of the em dash instead.

    Worse than mojibake, cp1252 cannot represent most of what UTF-8 can, and
    stdout re-encodes with errors='strict': a single character outside the
    codepage raises UnicodeEncodeError HERE, killing the driver mid-capture
    and leaving the suite a truncated file to fail on — a harness fault
    wearing the costume of a kernel one.
    """
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):          # 3.7+; older hosts keep the locale
            stream.reconfigure(encoding="utf-8", errors="replace")


PROMPT = b"afrisovera>"

# Every one of these was a hard-coded constant. They are named, and overridable
# from the environment, so a slow CI box can be told it is slow instead of being
# recorded as broken. The defaults are generous on purpose: they are ceilings on
# a pathological guest, not the time a healthy run actually spends — a healthy
# suite touches none of them and finishes in the same ~14s it always did.
#
# They are also SIZED TO FIT run-all-tests.sh's per-suite timeout (RAT_TIMEOUT,
# default 300s): a guest that is genuinely dead must still be recorded as that
# suite's own FAIL rather than run past the runner's clock and land as TIMEOUT.
# Worst case here is BOOT_CEILING + n_cmds * CMD_QUIET, so keep the sum under
# 300s if these are raised (90 + 6*30 = 270 for a typical six-command suite).
# BOOT_GRACE does NOT widen that worst case: it is only ever spent below the
# ceiling (BOOT_QUIET + BOOT_GRACE = 9s at the shipped defaults; the two persist
# suites pass AFRISH_BOOT_GRACE=10, so 13s there - both against a 90s
# BOOT_CEILING), and if it is ever raised past the ceiling the loop simply
# exits by the ceiling instead.
BOOT_CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "90"))    # boot -> prompt
BOOT_QUIET = float(os.environ.get("AFRISH_BOOT_QUIET", "3"))         # total silence -> suspect already booted
BOOT_GRACE = float(os.environ.get("AFRISH_BOOT_GRACE", "6"))         # ...then this much more before typing
CMD_QUIET = float(os.environ.get("AFRISH_CMD_QUIET", "30"))          # silence that means "stuck"
CMD_CEILING = float(os.environ.get("AFRISH_CMD_CEILING", "90"))      # backstop on one command


def connect_serial(port, ceiling=BOOT_CEILING):
    """Connect to QEMU's serial socket, retrying while it is not yet listening.

    QEMU binds this socket early but not instantly — measured 0.53s, 0.56s and
    1.14s on three consecutive starts of the same idle box. The suites hid that
    behind their `sleep`; without the retry, removing the sleep would trade a
    late-boot failure for a connection-refused one.
    """
    deadline = time.time() + ceiling
    last = None
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=10)
        except OSError as e:                     # not listening yet
            last = e
            time.sleep(0.05)
    raise SystemExit(f"shell-demo: serial port {port} never accepted in {ceiling}s ({last})")


def main():
    stdout_utf8()
    serial_port = int(sys.argv[1])
    qmp_port = int(sys.argv[2])
    out = sys.argv[3]
    cmds = sys.argv[4:] or ["about", "mem", "uptime", "echo AFRISOVERA IS LIVE"]

    log = bytearray()
    ser = connect_serial(serial_port)
    ser.settimeout(0.4)

    def drain():
        try:
            while True:
                b = ser.recv(4096)
                if not b:
                    break
                log.extend(b)
        except socket.timeout:
            pass

    def drain_until_prompt(start, echo=b"", quiet=CMD_QUIET, ceiling=CMD_CEILING):
        # Read until THIS command's output ends with the shell prompt. Only the
        # bytes produced after `start` (the moment we sent the command) are
        # examined, so the trailing space left over from the previous prompt
        # cannot trigger an immediate (empty) return.
        #
        # The giving-up rule is IDLE-BASED, not a stopwatch from the send. The
        # old absolute `cap=10.0` asked "has the answer arrived within 10s?",
        # which is a question about the HOST as much as the guest: a starved
        # guest that answers correctly at t=12s was recorded as no answer at
        # all, and the suite blamed whichever subsystem it happened to name.
        # The rule here is "has the guest gone SILENT for `quiet` seconds?" — a
        # guest still emitting bytes is still working and is given more time,
        # so contention costs wall clock instead of manufacturing a red. A real
        # regression is unaffected: a wrong answer, a usage error, even an
        # unknown command all end in a prompt, which returns immediately; only
        # a guest producing NOTHING waits, and that is a wedge worth waiting on.
        # `ceiling` is the backstop so a truly dead guest cannot hang a run.
        # 3.351 OBSERVABILITY ONLY -- no window is widened here (the 3.345/3.347
        # rule: do not change the constant in the same tick as the measurement
        # meant to justify it). This loop has THREE exits and, until now, two of
        # them were indistinguishable from the third: only the prompt match
        # `return`s from inside, while an idle expiry and a ceiling expiry both
        # fall out of the `while` silently, so the caller (and audio-shot.sh's
        # rc=0) could not tell "the guest answered" from "the guest went quiet
        # and I gave up". 3.350's truncated boot ended its serial at the module
        # load with NO prompt and no [asili voice] line, so which exit fired is
        # exactly the unmeasured quantity. The reason is now named, with the
        # elapsed time and the tail actually seen.
        hard_stop = time.time() + ceiling
        idle_stop = time.time() + quiet
        t_send = time.time()
        while time.time() < min(idle_stop, hard_stop):
            try:
                b = ser.recv(4096)
                if b:
                    log.extend(b)
                    idle_stop = time.time() + quiet     # progress: extend the lease
                    # 3.354 REPAIR -- ONE added conjunct, nothing else: a prompt
                    # only counts once THIS command's own echo has appeared in
                    # the post-send bytes. 3.353 measured the defect: when the
                    # guest types BLIND (wait_ready gave up on an empty log, so
                    # start=0), the boot BANNER arrives after the send and its
                    # own trailing prompt satisfied the old predicate 16 bytes
                    # BEFORE the command echo had even finished -- the driver
                    # declared `nia notify.wasm` done before it had printed a
                    # line, losing the whole serial tail. Falsified OFFLINE
                    # first (scripts/drainmatch-falsify-3354.py, 4/4): current
                    # rule PREMATURE at 104 on the bad boot, CORRECT at 543 on
                    # the good one; proposed rule NO-MATCH on the bad boot and
                    # CORRECT at the SAME 543 on the good one -- no regression.
                    # No window, no timeout and no return value is changed here
                    # (the 3.345/3.347 rule). Because the rule can now DECLINE
                    # to match, the arm to watch is the IDLE/CEILING exit below,
                    # which 3.351 made observable.
                    if echo in log[start:] and \
                            log[start:].rstrip().endswith(b"afrisovera>"):
                        if os.environ.get("AFRISH_DRAIN_TRACE"):
                            print("shell-demo: drain exit PROMPT after %.2fs (%d bytes)"
                                  % (time.time() - t_send, len(log) - start),
                                  file=sys.stderr)
                        return "prompt"
            except socket.timeout:
                pass
        # Abnormal exits are reported ALWAYS (they were silent before, which is
        # the defect); the passing PROMPT path stays quiet unless traced, so the
        # 686 green suites' logs are unchanged on their normal path.
        why = "CEILING" if time.time() >= hard_stop else "IDLE"
        print("shell-demo: drain exit %s after %.2fs (quiet=%.1f ceiling=%.1f, "
              "%d bytes since send, tail %r)"
              % (why, time.time() - t_send, quiet, ceiling, len(log) - start,
                 bytes(log[max(start, len(log) - 60):])),
              file=sys.stderr)
        return why.lower()

    def wait_ready(ceiling=BOOT_CEILING, quiet=BOOT_QUIET, grace=BOOT_GRACE):
        """Block until the guest is actually listening, instead of assuming it.

        Every suite that drives this script gates on a fixed `sleep` before
        invoking it — 464 of them on `sleep 8` — which is a guess about how long
        this box takes to boot. Measured on an idle host, three consecutive
        boots reached the prompt at 20.25s, 2.96s and 8.82s: the guess is not
        merely wasteful, it is wrong roughly a third of the time, and when it is
        wrong the first commands are typed at a guest that is not yet reading
        them. The suite then reports a failure OF THE SUBSYSTEM IT NAMES.

        So: read the boot output and wait for the shell's prompt. Bytes read
        here are kept in `log`, so the banner still reaches the capture. Returns
        early on the prompt, and falls through on `ceiling` rather than dying —
        a guest that never prints a prompt is left to the assertions, exactly as
        before, so this can make a suite slower but never newly red.

        The case this actually buys is the realistic one: a starved guest is
        SLOW, not stopped, so its boot text keeps arriving and we simply wait for
        the prompt however long it takes. What it cannot rescue is a guest given
        no CPU at all, because the harness has already typed into it by then —
        see the `quiet` fall-through below.
        """
        t_attach = time.time()
        hard_stop = t_attach + ceiling
        t_suspect = None            # when the quiet window first ran out, if it did
        while time.time() < hard_stop:
            try:
                b = ser.recv(4096)
                if b:
                    log.extend(b)
                    if t_suspect is not None:
                        # RETRACTION. The guest we had written off as
                        # already-booted is in fact still booting, and it said so
                        # before we typed at it. This is the whole point of the
                        # grace window below: the decision is provisional until
                        # the first keystroke, and this is where it is undone.
                        print("shell-demo: boot output arrived %.1fs after attach — retracting "
                              "the quiet-window guess (guest was booting, not booted), "
                              "waiting for the prompt"
                              % (time.time() - t_attach), file=sys.stderr)
                        t_suspect = None
                    if PROMPT in log:
                        return True
                    continue
            except socket.timeout:
                pass
            # Total silence since we attached is AMBIGUOUS: either the guest has
            # not booted, or it booted before we connected and QEMU's serial
            # socket does not replay what it emitted with no client attached.
            # Resolve it by falling through, NOT by sending a probe byte. A
            # version of this poll that nudged with a bare CR was measured to
            # WEDGE the guest: typing one byte into a kernel that is still
            # booting stops it ever reaching the shell (0 bytes -> prompt at
            # 14.4s; 1 byte -> never, boot dies at the preemption demo). That is
            # a kernel defect, filed as its own milestone; until it is fixed the
            # rule here is that this script does not speak before it is spoken
            # to. Falling through costs `quiet` on an already-booted guest and
            # leaves that case behaving exactly as it did before this change.
            #
            # 2.781 MEASURED WHAT THAT COSTS THE OTHER CASE, and 2.782 pays for
            # it here. A guest merely SLOW to its first byte is indistinguishable
            # on this socket from one that booted before we attached, and the
            # fall-through resolved the ambiguity IMMEDIATELY and IN SILENCE:
            # `bootquiet-falsify-2781.sh` arm A (6s of silence, then a real boot,
            # wedge on) reproduced `.rat-logs-2770/rjust.out` exactly — no
            # banner, no prompt, capture ending on the preemption demo, 185s
            # spent — while arm D (same silence, wedge off) was green. So the
            # branch is right about the case it was written for and wrong, at
            # full price, about the neighbouring one.
            #
            # The repair is not a longer `quiet` and not a deleted branch (arm C2
            # measured deletion at the whole BOOT_CEILING). It is that the
            # decision becomes PROVISIONAL: `quiet` of silence only makes the
            # guest a SUSPECT, `grace` more must pass before we type, and any
            # byte in between retracts it (above) and costs nothing. Typing early
            # loses the whole suite and misattributes it; waiting loses seconds,
            # and only on the suites that reach here at all — measured across the
            # 2.770 walk, 657 of 665 captures hold `AfriSh ready`, so 98.8% of
            # them saw boot output and cannot reach this branch.
            #
            # Both exits now NAME THEMSELVES on stderr (2.781 finding 3: the
            # ceiling exit printed `no prompt within Ns`, this one printed
            # nothing, which is why `rjust` had to be ruled by elimination across
            # a whole walk directory instead of read off its own capture).
            #
            # The condition is spelled with `silent_for` rather than inline, and
            # that is LOAD-BEARING, not style: `bootquiet-falsify-2781.sh:76`
            # identifies the pre-fix driver by the SUBSTRING of the old inline
            # test, so that its arm A0 can keep grading a revision that still has
            # the defect. Leave the old spelling here and A0 selects THIS file the
            # moment it is committed — the falsifier would grade the fix against
            # itself and go green where it must stay red forever.
            silent_for = time.time() - t_attach
            if not log and silent_for >= quiet:
                if t_suspect is None:
                    t_suspect = time.time()
                    print("shell-demo: no boot output in the %gs quiet window after attach — "
                          "the guest may have booted before we attached; holding %gs of grace "
                          "before typing (a keystroke mid-boot wedges a booting guest)"
                          % (quiet, grace), file=sys.stderr)
                elif time.time() - t_suspect >= grace:
                    print("shell-demo: assuming already booted after %.1fs of total silence "
                          "(quiet %gs + grace %gs) — typing blind"
                          % (time.time() - t_attach, quiet, grace), file=sys.stderr)
                    return False
        print(f"shell-demo: no prompt within {ceiling}s of boot", file=sys.stderr)
        return False

    wait_ready()
    for c in cmds:
        # A ":" prefix sends the payload raw (no CR), for feeding keys to an
        # interactive app; \xNN escapes are honored. Otherwise send cmd + CR.
        if c.startswith(":"):
            payload = c[1:].encode().decode("unicode_escape").encode("latin1")
            ser.sendall(payload)
            time.sleep(0.9)
            drain()
        else:
            start = len(log)
            ser.sendall((c + "\r").encode())
            drain_until_prompt(start, c.encode())
    time.sleep(1.0)
    drain()
    drain()
    mouse_rel = os.environ.get("SHOT_MOUSE_REL", "")  # "dx,dy": move the mouse before the shot (afriui.md section 22)

    # Screenshot via QMP -- a DECORATION, and it must not be able to destroy
    # the evidence. 2.772: this reply arrives on a 10s socket timeout, and
    # when it did not (once in 236 captures of the 2.770 walk -- see
    # .rat-logs-2770/firstword.out) the exception killed the driver BEFORE
    # the line below that emits the serial log. The suite then graded an
    # EMPTY capture and reported the failure under the kernel's name:
    # "FIRSTWORD TEST FAILED (missing 'firstword: line 1: Welcome')", for a
    # reason living entirely in a screenshot. Same species as the
    # UnicodeEncodeError described in stdout_utf8() above, which fixed the
    # cause and left the shape: the capture is still written LAST, so any
    # exception after the drain loses all of it. The picture may now fail;
    # the capture may not.
    # 3.655: the `timeout=10` below is a PER-RECV timeout, not a bound on this
    # block. scripts/qmpwin-falsify-3654.py arm D measured that: a peer that
    # dribbles one non-newline byte every 2.5 s never lets the timeout expire,
    # so the block ran 15.10 s against the blackhole's 10.01 s and would run
    # arbitrarily longer -- the dribble interval is the PEER's choice, not
    # ours, while the census charges this line 10 s.
    #
    # Re-arming the socket timeout once before each readline() does NOT fix it
    # (the shape 3.654's own => NEXT clause proposed): readline() loops on
    # recv, and every recv inside it gets the whole timeout again, so the cost
    # is N recvs x 10 s and the dribbler still wins. The bound has to be
    # re-armed INSIDE the read loop, which is why the buffered makefile() is
    # gone and the line assembly is explicit below.
    #
    # The budget is READ BACK off the socket (q.gettimeout()) instead of being
    # re-typed, so there is exactly ONE 10 in this block -- the 3.501
    # duplicate-constant hazard, where an edit to the documented copy is
    # silently overwritten by a bare second one.
    #
    # 3.657: the deadline used to be armed AFTER create_connection returned, so
    # the connect's own 10 s sat OUTSIDE it and the worst case was 10 + 10 = 20 s
    # -- 3.656's own honest gap (a). scripts/qmpconnect-falsify-3657.py measured
    # it on the REAL driver with no QEMU: a connect made to cost 1.2 s (a
    # listener armed inside Windows' ~2.05 s SYN-retransmit window, so the connect
    # is SLOW and then SUCCEEDS) put the block at 11.56 s against the blackhole's
    # 10.11 s -- the connect ADDED. Anchoring `t_open` BEFORE the connect and
    # deriving the deadline from it makes the budget cover both phases, so the
    # 10 s the census charges this line is the bound on the WHOLE block.
    #
    # Note what does NOT work here: a filtered/unroutable peer, which 3.656's
    # `=> NEXT:` clause proposed as the test. A connect that burns its whole
    # window ENDS IN AN EXCEPTION, so the read phase is never entered and the two
    # windows are never spent in the same run -- arm 7 of the falsifier re-derives
    # that every run. Only a slow-and-then-successful connect can show 10 + 10.
    #
    # 3.667: 3.659 left the OTHER half of the same gap open -- `deadline` was
    # derived AFTER `create_connection` returned, by reading the budget BACK off
    # the socket (`q.gettimeout()`), while `create_connection` itself was still
    # handed a bare `timeout=10` literal, not the remaining budget. `deadline` is
    # now derived FIRST (`time.time() + 10`, no `t_open` variable needed any more
    # -- deadline itself is now the anchor), and `create_connection`'s own timeout
    # is `max(0, deadline - time.time())` -- the remaining budget, same as every
    # recv in the read loop below. There is still exactly one `10` in this block
    # (3.501's duplicate-constant hazard); `q.gettimeout()` is no longer read at
    # all. NOTE for the census: an EARLIER draft anchored a `t_open` variable
    # first and added the literal to THAT name one line down -- which makes the
    # `10` INVISIBLE to scripts/fixed-window-census.py, because no dialect
    # matches a plain-variable addition, only the literal text `time.time() + N`
    # (`py-deadline`) or a `timeout=N` keyword (`py-timeout-kw`). The line below
    # derives `deadline` straight off a fresh `time.time()` call instead, so the
    # `py-deadline` match holds and the driver class stays visible at its
    # measured count; see the 3.667 audit-doc amendment. Measured
    # differentially (scripts/qmpconnect-falsify-3657.py arm F, listener armed as
    # late as the harness allows, ~1.7 s): NO observable wall-clock differential
    # -- see SESSION_PLAN.md 3.667. Expected: `deadline` is derived and
    # `create_connection` is called on adjacent lines, so the elapsed time
    # between them is microseconds, and `max(0, deadline - time.time())`
    # evaluates to ~10 either way. The fix is a correctness/symmetry improvement
    # (one source of truth, `deadline`, feeds both the connect and every read),
    # not a behaviour change in the currently-constructible test range.
    NL = b"\n"
    q = None
    try:
        deadline = time.time() + 10
        q = socket.create_connection(("127.0.0.1", qmp_port),
                                      timeout=max(0, deadline - time.time()))
        pending = bytearray()

        def qmp_line():
            """One reply line, bounded by the WHOLE block's deadline."""
            while pending.find(NL) < 0:
                left = deadline - time.time()
                if left <= 0:
                    raise TimeoutError("qmp deadline exceeded")
                q.settimeout(left)
                chunk = q.recv(4096)
                if not chunk:
                    raise ConnectionError("qmp peer closed before a reply")
                pending.extend(chunk)
            i = pending.find(NL)
            line = bytes(pending[:i + 1])
            del pending[:i + 1]
            return line

        qmp_line()                                    # the QMP greeting
        q.sendall(b'{"execute":"qmp_capabilities"}' + NL)
        qmp_line()                                    # capabilities reply
        # "dx,dy[;dx,dy...]": move the pointer out of the picture first, one move after another (the desk
        # clamps each to the screen, so a big move then a small one lands a set distance from an edge)
        for step in filter(None, mouse_rel.split(";")):
            dx, dy = (int(v) for v in step.split(","))
            ev = [{"type": "rel", "data": {"axis": "x", "value": dx}},
                  {"type": "rel", "data": {"axis": "y", "value": dy}}]
            q.sendall(json.dumps({"execute": "input-send-event", "arguments": {"events": ev}}).encode() + NL)
            print("mouse:", step, qmp_line().decode().strip())
            time.sleep(1.5)  # the desk takes the move on its next pass
        # json.dumps escapes backslashes so Windows paths don't break the QMP JSON.
        msg = json.dumps({"execute": "screendump", "arguments": {"filename": out, "format": "png"}})
        q.sendall(msg.encode() + NL)
        print("screendump:", qmp_line().decode().strip())
    except Exception as e:
        print(f"shell-demo: screendump skipped ({type(e).__name__}: {e})",
              file=sys.stderr)

    sys.stdout.write(log.decode(errors="replace"))
    ser.close()
    if q is not None:
        q.close()


if __name__ == "__main__":
    main()
