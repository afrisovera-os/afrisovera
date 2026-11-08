#!/usr/bin/env bash
# TCP check: boot with virtio-net + a packet dump, drive `net tcp` (active-open
# SYN to 10.0.2.2:80), and confirm a VALID TCP SYN (IP proto 6, flags SYN, correct
# pseudo-header checksum) was transmitted on the wire. A SYN/ACK or RST reply from
# SLIRP (if any) is reported by the kernel but not required to pass. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
PCAP="${TMPDIR:-/tmp}/tcp-test.pcap"
rm -f "$PCAP"

# --- host-side precondition (2.834 a4) -------------------------------------
# The guest dials 10.0.2.2:80, which is slirp's alias for the HOST's
# 127.0.0.1:80. With nothing bound there, slirp on this box does not RST, it
# stays SILENT, and the suite FAILs for a reason outside the kernel. So the
# suite now stands up its own listener instead of hoping one exists.
# TCP_TEST_NO_LISTENER=1 suppresses it - that is the falsifier arm.
LPORT=80
LLOG="${TMPDIR:-/tmp}/tcp-test-listener.log"
LPID=""
# LOWNS=1 only when THIS suite owns the socket, i.e. only then can $LLOG see an
# accept. On every other branch the accept count is NOT MEASURED - printing a 0
# there reads like "the guest never got through" when the handshake may well
# have reached a listener whose log this suite does not hold (2.859 finding).
LOWNS=0
LREASON="not measured this run - no listener of ours"
cleanup_listener() { [ -n "$LPID" ] && kill "$LPID" 2>/dev/null; }
trap cleanup_listener EXIT
if [ "${TCP_TEST_NO_LISTENER:-0}" = "1" ]; then
    echo "HOST-LISTENER=suppressed (TCP_TEST_NO_LISTENER=1, falsifier arm)"
    LREASON="not measured this run - listener suppressed (falsifier arm)"
else
    rm -f "$LLOG"; : > "$LLOG"
    python3 "$(dirname "$0")/lib/tcp-host-listener.py" 127.0.0.1 "$LPORT" "$LLOG" &
    LPID=$!
    for _ in $(seq 1 50); do
        grep -qE '^(READY|INUSE|BIND-FAIL)' "$LLOG" 2>/dev/null && break
        sleep 0.1
    done
    case "$(head -1 "$LLOG" 2>/dev/null)" in
        READY*) echo "HOST-LISTENER=own (this suite bound 127.0.0.1:$LPORT)"; LOWNS=1 ;;
        INUSE*) echo "HOST-LISTENER=foreign (something already listens on 127.0.0.1:$LPORT)"; LPID=""
                LREASON="not measured this run - a FOREIGN listener owns 127.0.0.1:$LPORT and keeps its own log" ;;
        *)      echo "HOST-LISTENER=none  $(head -1 "$LLOG" 2>/dev/null)"
                echo "TCP TEST FAILED  (precondition: no host listener on 127.0.0.1:$LPORT; this is a HOST fault, not the kernel)"
                exit 1 ;;
    esac
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4540,server,nowait \
    -qmp tcp:127.0.0.1:4541,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4540 4541 "${TMPDIR:-/tmp}/tcp-test.png" "net arp" "net tcp" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "$SER" | grep -iE "TCP SYN sent|SYN/ACK received|ESTABLISHED|TCP RST received|no reply" || true
# the listener's own view - independent of the guest's serial claim
ACC=0
if [ "$LOWNS" = "1" ]; then
    ACC="$(grep -c '^ACCEPTED' "$LLOG" 2>/dev/null || true)"
    case "$ACC" in ''|*[!0-9]*) ACC=0 ;; esac
    echo "  host listener accepted: $ACC"
else
    echo "  host listener accepted: $LREASON"
fi

# 2.861: on the OWN branch that count is now GRADED, not just printed - a
# handshake that completes on the wire but leaves our own listener with 0
# accepts is a contradiction, and the suite must go red on it.
python3 - "$PCAP" "$LOWNS" "$ACC" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
off = 24
def csum(b):
    s = 0
    for i in range(0, len(b) - 1, 2):
        s += (b[i] << 8) | b[i+1]
    if len(b) & 1:
        s += b[-1] << 8
    while s >> 16:
        s = (s & 0xffff) + (s >> 16)
    return (~s) & 0xffff

our_syn_ok = False   # our SYN with valid checksum
peer_synack = False  # peer SYN/ACK
our_ack_ok = False   # our final ACK with valid checksum
peer_rst = False     # peer RST (closed-port fallback)

while off + 16 <= len(d):
    incl = struct.unpack("<I", d[off+8:off+12])[0]
    fr = d[off+16:off+16+incl]; off += 16 + incl
    if len(fr) < 54: continue
    if (fr[12] << 8 | fr[13]) != 0x0800: continue    # IPv4
    if fr[23] != 6: continue                          # proto TCP
    src = ".".join(str(x) for x in fr[26:30])
    ihl = (fr[14] & 0x0f) * 4
    toff = 14 + ihl
    flags = fr[toff+13]
    tcp = fr[toff:toff+20]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", 20)
    ck_ok = (csum(pseudo + tcp) == 0)
    if src == "10.0.2.15" and (flags & 0x02) and not (flags & 0x10):
        print(f"  SYN     {src} -> {'.'.join(str(x) for x in fr[30:34])} checksum {'OK' if ck_ok else 'BAD'}")
        if ck_ok: our_syn_ok = True
    elif src == "10.0.2.2" and (flags & 0x12) == 0x12:
        print(f"  SYN/ACK {src} (peer accepted)")
        peer_synack = True
    elif src == "10.0.2.15" and (flags & 0x10) and not (flags & 0x02):
        print(f"  ACK     {src} (completes handshake) checksum {'OK' if ck_ok else 'BAD'}")
        if ck_ok: our_ack_ok = True
    elif src == "10.0.2.2" and (flags & 0x04):
        print(f"  RST     {src} (port closed)")
        peer_rst = True

if not our_syn_ok:
    print("TCP TEST FAILED  (no valid TCP SYN on the wire)")
    sys.exit(1)
if peer_synack and our_ack_ok:
    # only assertable when THIS suite owns the listener (LOWNS=1); on every
    # other branch the count was never measured, so there is nothing to grade.
    own = len(sys.argv) > 2 and sys.argv[2] == "1"
    acc = int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3].isdigit() else 0
    if own and acc < 1:
        print(f"TCP TEST FAILED  (handshake completed on the wire but OUR host "
              f"listener logged {acc} accepts - the two views disagree)")
        sys.exit(1)
    print("TCP TEST PASSED  (full 3-way handshake: SYN -> SYN/ACK -> ACK, checksums OK)")
    sys.exit(0)
if peer_rst:
    print("TCP TEST PASSED  (SYN (valid checksum) + RST round-trip; port was closed this run)")
    sys.exit(0)
print("TCP TEST FAILED  (SYN sent but no SYN/ACK+ACK or RST observed)")
sys.exit(1)
PY
