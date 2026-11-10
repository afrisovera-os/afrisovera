#!/usr/bin/env bash
# mesh-phase0: Sovereign Mesh Phase 0 end to end (AFR-0009 B1 + B2, plan of record section "Phase 0").
# Two AfriSovera ARM machines, no internet: B listens on TCP 4401, A dials it, they run Noise XX
# (Noise_XX_25519_ChaChaPoly_SHA256, prologue "AFRISOVERA-MESH-LINK-V1\0" + transport 0x03), A seals one
# bucket-0 AFR-0009 bundle to the static key it learned in the handshake and sends it as a BUNDLE record, B opens
# it and answers CLOSE.
#
# Rig: scripts/mesh-bridge.py. Each guest has its own `-netdev socket` wire to the bridge, which is the gateway
# for both (ARP, DHCP) and carries TCP between them with the two IP addresses swapped, so each guest sees the
# other as 10.0.2.2. A dials 10.0.2.2:4401. The architect's first choice, SLIRP + hostfwd, was measured in the
# cloud on 10 Nov 2026 and the inbound connection never reached B (no SYN in 90 s); every inbound suite here
# already uses the socket-netdev pattern instead (the 2.756 rig). Both machines get a virtio-rng device: secrets
# need a counted, health-tested hardware source and the mesh refuses to run without one (see mesh-kernel).
#
# Graded (all must hold):
#   1. B: "link UP (Noise XX responder)" and A: "link UP (Noise XX initiator)", with the SAME six-digit SAS and
#      each naming the OTHER's identity key (the key each one printed for itself);
#   2. A: "bundle SEALED ... 256 bytes, bucket 0" and B: "bundle OPENED from <A's key> (256 bytes sealed ...)"
#      carrying exactly the text A was given;
#   3. A: "peer CLOSED the link after the bundle" and "send done: delivered"; B: "listen done: bundle opened";
#   4. no "link FAILED" on either side.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
OUTB="$T/mesh-phase0-B.out"
OUTA="$T/mesh-phase0-A.out"
MSG="Habari-from-A-$$"
rm -f "$OUTB" "$OUTA"
. "$(dirname "$0")/lib/afrish-boot.sh"

python3 -I scripts/mesh-bridge.py --selftest || { echo "MESH-PHASE0 TEST FAILED (bridge selftest)"; exit 1; }
PORTFILE="$T/mesh-phase0.ports"
READY="$T/mesh-phase0.ready"
BRLOG="$T/mesh-phase0.bridge"
rm -f "$PORTFILE" "$READY" "$BRLOG"
python3 -u scripts/mesh-bridge.py "$PORTFILE" "$READY" 200 > "$BRLOG" 2>&1 &
BR=$!
for _ in $(seq 1 40); do [ -s "$PORTFILE" ] && break; sleep 0.25; done
read -r PA PB < "$PORTFILE"   # no `||` on read: a file without a final newline makes read return 1 after filling both
[ -n "${PB:-}" ] || { echo "MESH-PHASE0 TEST FAILED (bridge never bound its ports: harness, not kernel)"; kill "$BR"; exit 1; }
echo "mesh-phase0: bridge A=$PA B=$PB"

# Machine A: the sender. Attached first (the bridge accepts A, then B).
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M     -global virtio-mmio.force-legacy=false -device ramfb -display none     -device virtio-rng-device     -netdev socket,id=n0,connect=127.0.0.1:"$PA" -device virtio-net-device,netdev=n0     -serial tcp:127.0.0.1:4410,server,nowait     -qmp tcp:127.0.0.1:4411,server,nowait -kernel "$BIN" &
QA=$!
sleep 1
# Machine B: the listener.
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M     -global virtio-mmio.force-legacy=false -device ramfb -display none     -device virtio-rng-device     -netdev socket,id=n0,connect=127.0.0.1:"$PB" -device virtio-net-device,netdev=n0     -serial tcp:127.0.0.1:4408,server,nowait     -qmp tcp:127.0.0.1:4409,server,nowait -kernel "$BIN" &
QB=$!
afrish_wait_boot

# B prints its key, then listens for up to 90 s (quiet ceiling raised: it is silent while it waits).
AFRISH_CMD_QUIET=100 AFRISH_CMD_CEILING=120   python3 -u scripts/shell-demo.py 4408 4409 "$T/mesh-phase0-B.png" "mesh id" "mesh listen 4401 90" > "$OUTB" 2>&1 &
DB=$!
# Dial only once B's listener has asked the bridge for its gateway (its tcp_ready): a signal on the wire, not
# a sleep (the 2.758 lesson).
for _ in $(seq 1 240); do [ -s "$READY" ] && break; sleep 0.5; done
[ -s "$READY" ] || echo "mesh-phase0: B never asked for its gateway; dialing anyway"
python3 -u scripts/shell-demo.py 4410 4411 "$T/mesh-phase0-A.png" "mesh id" "mesh send 10.0.2.2:4401 $MSG" > "$OUTA" 2>&1
wait "$DB" 2>/dev/null
kill "$QA" "$QB" 2>/dev/null
wait "$QA" "$QB" 2>/dev/null
kill "$BR" 2>/dev/null
wait "$BR" 2>/dev/null

echo "--- bridge"; grep -a "bridge:" "$BRLOG" | sed 's/^/  /'
echo "--- B"; grep -aE "\[mesh\]|\[net\] (serve|TCP inbound)" "$OUTB" | sed 's/^/  /'
echo "--- A"; grep -aE "\[mesh\]|\[net\] tcp dial" "$OUTA" | sed 's/^/  /'

python3 - "$OUTA" "$OUTB" "$MSG" "$BRLOG" <<'PY'
import re, sys
a = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
b = open(sys.argv[2], "rb").read().decode("utf-8", "replace")
msg = sys.argv[3]
br = open(sys.argv[4], "rb").read().decode("utf-8", "replace")
bad = []
def need(cond, what):
    if not cond:
        bad.append(what)
ida = re.search(r"\[mesh\] identity x25519 ([0-9a-f]{64})", a)
idb = re.search(r"\[mesh\] identity x25519 ([0-9a-f]{64})", b)
need(ida and idb, "an identity line is missing")
ua = re.search(r"link UP \(Noise XX initiator\) peer ([0-9a-f]{16}) SAS (\d{6})", a)
ub = re.search(r"link UP \(Noise XX responder\) peer ([0-9a-f]{16}) SAS (\d{6})", b)
need(ua, "A: no 'link UP (Noise XX initiator)'")
need(ub, "B: no 'link UP (Noise XX responder)'")
if ua and ub:
    need(ua.group(2) == ub.group(2), "SAS differs: A %s vs B %s" % (ua.group(2), ub.group(2)))
    if ida and idb:
        need(ua.group(1) == idb.group(1)[:16], "A did not learn B's identity key")
        need(ub.group(1) == ida.group(1)[:16], "B did not learn A's identity key")
need(re.search(r"bundle SEALED to [0-9a-f]{16} \(256 bytes, bucket 0\)", a), "A: no 'bundle SEALED ... 256 bytes, bucket 0'")
ob = re.search(r"bundle OPENED from ([0-9a-f]{16}) \(256 bytes sealed, type 1, created \d+\): \"([^\"]*)\"", b)
need(ob, "B: no 'bundle OPENED'")
if ob:
    need(ob.group(2) == msg, "B opened %r, A sent %r" % (ob.group(2), msg))
    if ida:
        need(ob.group(1) == ida.group(1)[:16], "B says the bundle came from someone other than A")
need("peer CLOSED the link after the bundle" in a, "A: no CLOSE from B")
need("send done: delivered" in a, "A: send not confirmed")
need("listen done: bundle opened" in b, "B: listen did not report an opened bundle")
need("link FAILED" not in a and "link FAILED" not in b, "a 'link FAILED' line was printed")
need(re.search(r"A->B TCP \d+->4401 flags 0x02", br), "bridge: no SYN from A to B:4401 on the wire")
need(re.search(r"B->A TCP 4401->\d+ flags 0x12", br), "bridge: no SYN/ACK from B on the wire")
if bad:
    print("MESH-PHASE0 TEST FAILED")
    for x in bad:
        print("  - " + x)
    sys.exit(1)
print("MESH-PHASE0 TEST PASSED  (XX link up both ways, SAS %s agrees, sealed bundle opened by B with the exact text, CLOSE back)" % ua.group(2))
PY
