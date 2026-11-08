#!/usr/bin/env bash
# Verify the console cannot go DEAF under a burst of typed input (2.749).
#
# Why this suite exists. Five registry suites (hmac, histogram, anagram, slug,
# bytesum) sat red for six milestones with an identical 45-byte serial capture,
# each cut off mid-echo of its FIRST command, and each PASSED when re-run alone.
# The cause was not the five commands under them: the PL011 RX ISR drained the
# FIFO and cleared the interrupt AFTERWARDS, so a byte landing between the last
# FIFO read and the clear was acknowledged without being taken -- and nothing
# ever announces it again, because RXI re-asserts only as the FIFO count CROSSES
# the trigger level and the receive-timeout interrupt that covers a straggler is
# exactly what the clear wiped. The console then ignores every later byte
# forever: the guest keeps running, the screen keeps showing the prompt, and the
# suite dies of a missing answer that was never asked for.
#
# It is a race, so a suite that types one command would essentially never see it:
# measured on the pre-fix kernel it took 28, 106 and 212 scripted commands (three
# seeds) to wedge. This suite types 300 with varied chunking and inter-chunk
# delays -- the pattern that provokes it -- and asserts every one is answered.
# Fixed seed, so a failure is reproducible rather than a rumour.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

# 2.870 PRE-FLIGHT, and it is not paranoia -- it is measured. QEMU exits
# SILENTLY when it cannot bind -serial (stderr captured on this host: EMPTY),
# so if a stale guest still holds 4884 then THIS run's guest vanishes without a
# word and the driver below connects to the PREVIOUS one's corpse: a red that
# names the UART for a fault in process reaping, and it fails in the flattering
# direction (something answers). It made three back-to-back measurements of this
# suite unreadable before it was found. The connect is a retry loop now, which
# WIDENS that window, so the collision is refused here instead.
if ! python3 -c "import socket,sys
s = socket.socket()
try:
    s.bind(('127.0.0.1', 4884))
except OSError:
    sys.exit(1)" 2>/dev/null; then
    echo "UART-RX-STRESS TEST FAILED (127.0.0.1:4884 is already bound -- a stale"
    echo "  guest is still up, and this run would have graded ITS console)"
    exit 1
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4884,server,nowait \
    -qmp tcp:127.0.0.1:4885,server,nowait -kernel "$BIN" &
QPID=$!
# 2.870: the boot wait moved INTO the driver, which is 2.758a's rule and the
# same seam pi/dhcp/config/smpinfo took. `sleep 6` was a GUESS ABOUT THE HOST
# and it guessed SHORT -- run on this tree under a live 684-suite walk it dies
# at the connect below with ECONNREFUSED before QEMU has bound 4884, i.e. a red
# that names the UART for a fault in the wait. The driver now retries the
# connect to a CEILING, so a slow-but-correct boot becomes a slow PASS.
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uart-rx-stress.out"
python3 - 4884 300 > "$OUT" 2>&1 <<'PY'
import random, socket, sys, time

port, trials = int(sys.argv[1]), int(sys.argv[2])
random.seed(20749)                      # fixed: a wedge here is reproducible
CMD = "echo the quick brown fox jumps over the lazy dog"

# A CEILING, NOT A WINDOW (2.870). QEMU binds 4884 when it is ready, not when a
# `sleep` says so, so the connect RETRIES until BOOT_CEILING instead of being
# fired once after a fixed guess. ConnectionRefusedError before the bind is the
# guest still starting -- it is exactly what the old `sleep 6` shape produced
# under load -- so it is retried, not raised; expiry is a NAMED failure, never a
# silent pass. The per-connect timeout=10 rides inside this loop, which is what
# makes it a ceiling (shell-demo.py's connect_serial has the same shape).
BOOT_CEILING = 60.0
t_boot = time.time()
ser = None
while ser is None:
    try:
        ser = socket.create_connection(("127.0.0.1", port), timeout=10)
    except OSError as exc:
        if time.time() - t_boot >= BOOT_CEILING:
            print(f"UART-RX-STRESS TEST FAILED (nothing listening on {port} "
                  f"after {BOOT_CEILING:.0f}s: {exc})")
            sys.exit(1)
        time.sleep(0.05)
print(f"  serial socket accepted after {time.time() - t_boot:.3f}s")
ser.settimeout(0.05)
log = bytearray()


def drain(cap):
    end = time.time() + cap
    while time.time() < end:
        try:
            b = ser.recv(4096)
            if b:
                log.extend(b)
        except socket.timeout:
            pass


def irqstat(tag):
    """Ask the guest for the 2.905 counters and PRINT them (2.914).

    2.905 named THIS suite as the one whose row would name the UART site if the
    loss ever recurred, and scripts/uartrow-2914.sh measured that the claim was
    false: 300 commands were stressed and the counters were never asked for.
    This is the ask. It runs on the PASS path too, so a green row still records
    the numbers the next red row has to be compared against.

    The not-answered branch deliberately does NOT spell the counter names: the
    row must not read as carrying counters when the console is the thing that
    died.
    """
    before = len(log)
    ser.sendall(b"irqstat\r")
    drain(2.0)
    text = bytes(log[before:]).decode("utf-8", "replace")
    for ln in text.splitlines():
        if "rx-drops" in ln and "rx-overruns" in ln:
            print(f"  irqstat ({tag}): {ln.strip()}")
            return
    print(f"  irqstat ({tag}): NO COUNTER LINE in {len(text)} bytes of reply")


drain(0.2)                              # take whatever is already on the wire
# Prove the shell is there by ASKING: a bare newline must come back as a prompt.
# This is now a CEILING too, and it has to be. The connect above lands the
# instant QEMU binds, which is BEFORE the guest has booted, so the head start
# the old `sleep 6` bought is gone and a fixed 5.0s here would be the same short
# guess moved one line down. Two consequences, both handled: the `\r` is
# RE-SENT each poll, because a newline typed at a guest that is not yet reading
# is simply lost and nothing would ever ask again; and the banner is no longer
# emitted-and-dropped (we are attached in time to see it), which is harmless --
# its own trailing prompt satisfies the same check if it arrives first.
end = time.time() + BOOT_CEILING
while time.time() < end and not bytes(log).rstrip().endswith(b"afrisovera>"):
    ser.sendall(b"\r")
    drain(1.0)
if not bytes(log).rstrip().endswith(b"afrisovera>"):
    # 2.940 (8d), MEASUREMENT ONLY -- the assertion above is unchanged. Every
    # failure this rig could print started mid-word because `[-60:]` is the
    # MESSAGE's own truncation of a whole-session accumulator, so 60 bytes can
    # never answer the one question that picks the repair: does `afrisovera>`
    # appear at all and get buried by later output, or does it never arrive?
    # Dump the WHOLE log beside the suite output and state the answer directly.
    whole = bytes(log)
    hits = whole.count(b"afrisovera>")
    dump = "/tmp/uart-rx-stress-fail.log"
    try:
        with open(dump, "wb") as fh:
            fh.write(whole)
    except OSError as exc:
        dump = f"<undumped: {exc}>"
    print(f"  WIDENED CAPTURE: {len(whole)} bytes total, "
          f"{hits} occurrence(s) of b'afrisovera>'; whole log at {dump}")
    if hits:
        last = whole.rfind(b"afrisovera>") + len(b"afrisovera>")
        print(f"  prompt DID appear; {len(whole) - last} bytes came AFTER the "
              f"last one, starting {whole[last:last + 80]!r}")
    else:
        print("  prompt NEVER appeared in the whole session")
    print(f"  first 200 bytes: {whole[:200]!r}")
    print(f"UART-RX-STRESS TEST FAILED (no prompt for a bare newline; got {bytes(log)[-60:]!r})")
    sys.exit(1)

answered = 0
for trial in range(trials):
    start = len(log)
    data = (CMD + "\r").encode()
    # Vary how the command is split and how fast the pieces arrive: the lost edge
    # needs a byte to land in the ISR's acknowledge window, so one fixed rhythm
    # would sample one point of a race.
    step = random.choice([1, 2, 3, 4, 8, 16, len(data)])
    for i in range(0, len(data), step):
        ser.sendall(data[i:i + step])
        time.sleep(random.choice([0, 0.0005, 0.001, 0.002]))
    end = time.time() + 3.0
    while time.time() < end:
        try:
            b = ser.recv(4096)
            if b:
                log.extend(b)
        except socket.timeout:
            pass
        if bytes(log[start:]).rstrip().endswith(b"afrisovera>"):
            break
    got = bytes(log[start:])
    if not got.rstrip().endswith(b"afrisovera>"):
        # Deaf, or just slow? Ask three more times: a wedge never answers again.
        after = len(log)
        for _ in range(3):
            ser.sendall(b"uptime\r")
            drain(2.0)
        deaf = len(log) == after
        if not deaf:
            irqstat("after loss")       # the site-naming ask; pointless if deaf
        print(f"  command {trial + 1} of {trials} unanswered after {len(got)} bytes: {got[:60]!r}")
        print(f"  three further commands produced {len(log) - after} bytes "
              f"({'console is DEAF for good' if deaf else 'console recovered'})")
        print(f"UART-RX-STRESS TEST FAILED (console lost input after {answered} answered commands)")
        sys.exit(1)
    answered += 1

irqstat("after 300 clean")
print(f"UART-RX-STRESS TEST PASSED  ({answered} commands typed in varied bursts, "
      f"every one echoed and answered; console never went deaf)")
PY
RC=$?
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tail -4 "$OUT"
exit $RC
