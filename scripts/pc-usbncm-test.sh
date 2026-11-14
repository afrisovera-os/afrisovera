#!/usr/bin/env bash
# CDC-NCM (13 Nov 2026, CDC_NCM_PLAN.md; architect's OK with conditions, ARCHITECT_REVIEWS 13 Nov): newer phones'
# USB tethering and 2.5G USB dongles.
#
# QEMU has no NCM device, and GitHub's runner kernel has no dummy_hcd for a Linux NCM gadget (step 0, os2 run
# 43340543746), so this is plan B (architect's N2): scripts/ci/qemu-usbnet-ncm.patch gives QEMU's usb-net an NCM
# mode (ncm=on: an NCM configuration beside the CDC-ECM one), applied when the CI builds QEMU. It is TEST
# EQUIPMENT, never part of AfriSovera, and the weaker kind of evidence (the same engineer wrote both ends), so it
# is graded with the block parser's selftest, built from the NCM 1.0 spec, hostile blocks included.
#   Boot 1: the driver must take NCM over the ECM configuration of the same device (N1), read the device's
#           block layout (tables on 16 bytes, frames at 2 mod 4) and keep to it (the patched device prints a
#           `usbnet:` line for any block that breaks it), and carry DHCP, a ping and a TCP payload to a peer on
#           the host. The host then sends 4 bursts of 40 raw broadcast frames into the same QEMU network hub;
#           the device gathers frames for up to 2 ms, so blocks carry several. `usbnettest` must show blocks in
#           and out, more than one frame in a block, no bad block and no stopped receive buffer; `usbnettest
#           ncm` (the parser's selftest) must come out all as expected.
#   Boot 2: ncm-badparams=on: the device reports an out divisor of 3, which the driver must refuse with one
#           line and then set the same device up as CDC-ECM without a replug; DHCP and a ping work over it.
#   scripts/pc-usbncm-test.sh              # grader selftest, then the two boots
#   scripts/pc-usbncm-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PAYLOAD="AFRISOVERA-NCM-TCP"
FRAMES='[0-9]+ frames in, [0-9]+ out'

grade1() { # grade1 <serial> <peer log> <QEMU's stderr>
    local fail=0 S P Q
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log (boot 1)"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(cat "$2" 2> /dev/null)"
    Q="$(cat "$3" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: USB Ethernet adapter 0525:a4a1 \(CDC-NCM, [a-z]+ speed\), slot [0-9]+, MAC ([0-9a-f]{2}:){5}[0-9a-f]{2}$' ||
        _f "boot 1: the device was not set up as CDC-NCM with its MAC (NCM must win over the ECM configuration beside it)"
    has '^\[xhci\] NCM: blocks up to 8 KiB in \(the device offers 16 KiB\), 16 KiB out, table alignment 16, frame divisor 4 remainder 2, 16-bit blocks$' ||
        _f "boot 1: no NCM block-size line with the device's layout (8 KiB asked of 16, alignment 16, divisor 4 remainder 2)"
    printf '%s\n' "$Q" | grep -q '^usbnet:' && _f "boot 1: the device complained: $(printf '%s\n' "$Q" | grep '^usbnet:' | head -1)"
    has 'CDC-NCM not used' && _f "boot 1: the NCM setup failed: $(grep -aE 'CDC-NCM not used' <<< "$S" | head -1)"
    has '^\[virtio\] slot [0-9]+: virtio-net \(id=1\), emulated over the USB Ethernet adapter \(CDC-NCM\)$' ||
        _f "boot 1: the adapter was not presented to the network stack as CDC-NCM"
    has '^\[net\] DHCP: IP 10\.0\.2\.15, mask 255\.255\.255\.0, gw 10\.0\.2\.2' || _f "boot 1: no DHCP lease over NCM"
    has '^\[net\] ICMP echo reply from 10\.0\.2\.2 \(ping OK\)$' || _f "boot 1: no ping reply over NCM"
    has '^\[net\] TCP data round-trip OK' || _f "boot 1: the TCP data round-trip did not complete"
    printf '%s\n' "$P" | grep -qF "peer received ${#PAYLOAD} bytes: $PAYLOAD" ||
        _f "boot 1: the peer did not receive the exact ${#PAYLOAD}-byte payload"
    local c
    c="$(printf '%s\n' "$S" | grep -aoE "^lsvirtio: slot [0-9]+ \(emulated over usb-ncm\) id 1 \(virtio-net\), $FRAMES$" | tail -1 |
        sed -E 's/.*, ([0-9]+) frames in, ([0-9]+) out$/\1 \2/')"
    if [ -z "$c" ]; then
        _f "boot 1: lsvirtio did not report the adapter as usb-ncm"
    else
        local fin fout
        read -r fin fout <<< "$c"
        { [ "$fin" -gt 0 ] && [ "$fout" -gt 0 ]; } || _f "boot 1: no frames in or out ($fin in, $fout out)"
    fi
    local u
    u="$(printf '%s\n' "$S" | grep -aE '^usbnettest: NCM [0-9]+ block\(s\) in' | tail -1)"
    if [ -z "$u" ]; then
        _f "boot 1: no usbnettest counter line"
    else
        local bin most bout bad stop
        bin="$(sed -nE 's/^usbnettest: NCM ([0-9]+) block.*/\1/p' <<< "$u")"
        most="$(sed -nE 's/.*most frames in one block ([0-9]+),.*/\1/p' <<< "$u")"
        bout="$(sed -nE 's/.*, ([0-9]+) block\(s\) out,.*/\1/p' <<< "$u")"
        bad="$(sed -nE 's/.*bad blocks ([0-9]+) .*/\1/p' <<< "$u")"
        stop="$(sed -nE 's/.*receive buffers stopped ([0-9]+)$/\1/p' <<< "$u")"
        [ "${bin:-0}" -gt 0 ] && [ "${bout:-0}" -gt 0 ] || _f "boot 1: no NCM blocks in or out: $u"
        [ "${most:-0}" -gt 1 ] || _f "boot 1: never more than one frame in a block (the bursts were not gathered): $u"
        [ "${bad:-1}" = 0 ] || _f "boot 1: a clean run counted bad blocks: $u"
        [ "${stop:-1}" = 0 ] || _f "boot 1: a receive buffer was stopped: $u"
    fi
    local st
    st="$(printf '%s\n' "$S" | grep -aE '^ncm selftest: [0-9]+ of [0-9]+ cases as expected$' | tail -1)"
    if [ -z "$st" ]; then
        _f "boot 1: the NCM block parser's selftest did not run"
    else
        local a b
        a="$(sed -nE 's/^ncm selftest: ([0-9]+) of ([0-9]+) .*/\1/p' <<< "$st")"
        b="$(sed -nE 's/^ncm selftest: ([0-9]+) of ([0-9]+) .*/\2/p' <<< "$st")"
        { [ "$a" = "$b" ] && [ "$b" -ge 20 ]; } || _f "boot 1: selftest $st (want all of at least 20)"
    fi
    has 'NOT as expected' && _f "boot 1: $(grep -aE 'NOT as expected' <<< "$S" | head -1)"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|refused|no reply)'; then
        _f "boot 1: $(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|refused|no reply)' | head -1)"
    fi
    return "$fail"
}

grade2() { # grade2 <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log (boot 2)"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: CDC-NCM not used: the out divisor is not a power of two; trying the device as CDC-ECM or RNDIS$' ||
        _f "boot 2: no line saying the device's NCM layout was refused and ECM tried"
    has '^\[xhci\] port [0-9]+: USB Ethernet adapter 0525:a4a1 \(CDC-ECM, [a-z]+ speed\), slot [0-9]+, MAC ([0-9a-f]{2}:){5}[0-9a-f]{2}$' ||
        _f "boot 2: the same device was not set up as CDC-ECM"
    has '^\[xhci\] NCM: blocks' && _f "boot 2: NCM was set up although refused"
    has '^\[virtio\] slot [0-9]+: virtio-net \(id=1\), emulated over the USB Ethernet adapter \(CDC-ECM\)$' ||
        _f "boot 2: the adapter was not presented as CDC-ECM"
    has '^\[net\] DHCP: IP 10\.0\.2\.15, mask 255\.255\.255\.0, gw 10\.0\.2\.2' || _f "boot 2: no DHCP lease over the ECM fallback"
    has '^\[net\] ICMP echo reply from 10\.0\.2\.2 \(ping OK\)$' || _f "boot 2: no ping reply over the ECM fallback"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)'; then
        _f "boot 2: $(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)' | head -1)"
    fi
    return "$fail"
}

GREEN1="[xhci] port 5: USB Ethernet adapter 0525:a4a1 (CDC-NCM, full speed), slot 1, MAC 52:54:00:12:34:56
[xhci] NCM: blocks up to 8 KiB in (the device offers 16 KiB), 16 KiB out, table alignment 16, frame divisor 4 remainder 2, 16-bit blocks
[virtio] slot 0: virtio-net (id=1), emulated over the USB Ethernet adapter (CDC-NCM)
[net] DHCP: IP 10.0.2.15, mask 255.255.255.0, gw 10.0.2.2, dns 10.0.2.3
afrisovera> ping
[net] ICMP echo reply from 10.0.2.2 (ping OK)
afrisovera> net tcp send 40000 $PAYLOAD
[net] TCP data round-trip OK
afrisovera> lsvirtio
lsvirtio: slot 0 (emulated over usb-ncm) id 1 (virtio-net), 30 frames in, 12 out
afrisovera> usbnettest
usbnettest: NCM 9 block(s) in, most frames in one block 7, 12 block(s) out, bad blocks 0 (last: none), skipped entries 0, frames with no room 3, receive buffers stopped 0
afrisovera> usbnettest ncm
ncm selftest: three frames, one table: Got { frames: 3 } (as expected)
ncm selftest: 20 of 20 cases as expected"
PEER_OK="peer received ${#PAYLOAD} bytes: $PAYLOAD"
GREEN2="[xhci] port 5: CDC-NCM not used: the out divisor is not a power of two; trying the device as CDC-ECM or RNDIS
[xhci] port 5: USB Ethernet adapter 0525:a4a1 (CDC-ECM, full speed), slot 1, MAC 52:54:00:12:34:56
[virtio] slot 0: virtio-net (id=1), emulated over the USB Ethernet adapter (CDC-ECM)
[net] DHCP: IP 10.0.2.15, mask 255.255.255.0, gw 10.0.2.2, dns 10.0.2.3
afrisovera> ping
[net] ICMP echo reply from 10.0.2.2 (ping OK)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbncm-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot 1 serial> <peer> <boot 2 serial> [QEMU's stderr in boot 1]
        local got
        printf '%s\n' "$3" > "$D/s1"; printf '%s\n' "$4" > "$D/p"; printf '%s\n' "$5" > "$D/s2"; printf '%s\n' "${6:-}" > "$D/q"
        if grade1 "$D/s1" "$D/p" "$D/q" > "$D/log" 2>&1 && grade2 "$D/s2" >> "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN1" "$PEER_OK" "$GREEN2"
    arm crlf green "$(printf '%s\n' "$GREEN1" | sed 's/$/\r/')" "$PEER_OK" "$(printf '%s\n' "$GREEN2" | sed 's/$/\r/')"
    arm ecm-instead-of-ncm red "$(printf '%s\n' "$GREEN1" | sed 's/(CDC-NCM, high/(CDC-ECM, high/; s/adapter (CDC-NCM)/adapter (CDC-ECM)/; s/usb-ncm/usb-ecm/; /NCM: blocks/d')" "$PEER_OK" "$GREEN2"
    arm no-block-line red "$(printf '%s\n' "$GREEN1" | grep -v 'NCM: blocks')" "$PEER_OK" "$GREEN2"
    arm ncm-failed-boot1 red "$(printf '[xhci] port 5: CDC-NCM not used: SET_NTB_INPUT_SIZE was refused; trying the device as CDC-ECM or RNDIS\n%s' "$GREEN1")" "$PEER_OK" "$GREEN2"
    arm layout-not-read red "$(printf '%s\n' "$GREEN1" | sed 's/table alignment 16, frame divisor 4 remainder 2/table alignment 4, frame divisor 4 remainder 0/')" "$PEER_OK" "$GREEN2"
    arm device-complained red "$GREEN1" "$PEER_OK" "$GREEN2" "usbnet: NCM frame from the host at 32 (60 bytes) breaks the layout"
    arm qemu-noise-ok green "$GREEN1" "$PEER_OK" "$GREEN2" "qemu-system-x86_64: warning: host doesn't support requested feature"
    arm no-dhcp red "$(printf '%s\n' "$GREEN1" | grep -v 'DHCP: IP')" "$PEER_OK" "$GREEN2"
    arm no-ping red "$(printf '%s\n' "$GREEN1" | grep -v 'ping OK')" "$PEER_OK" "$GREEN2"
    arm payload-changed red "$GREEN1" "peer received 18 bytes: AFRISOVERA-NCM-TCX" "$GREEN2"
    arm counters-still red "$(printf '%s\n' "$GREEN1" | sed 's/30 frames in, 12 out/0 frames in, 0 out/')" "$PEER_OK" "$GREEN2"
    arm one-frame-per-block red "$(printf '%s\n' "$GREEN1" | sed 's/most frames in one block 7/most frames in one block 1/')" "$PEER_OK" "$GREEN2"
    arm bad-block-counted red "$(printf '%s\n' "$GREEN1" | sed 's/bad blocks 0 (last: none)/bad blocks 1 (last: bad table index)/')" "$PEER_OK" "$GREEN2"
    arm buffer-stopped red "$(printf '%s\n' "$GREEN1" | sed 's/receive buffers stopped 0/receive buffers stopped 1/')" "$PEER_OK" "$GREEN2"
    arm no-selftest red "$(printf '%s\n' "$GREEN1" | grep -v 'cases as expected')" "$PEER_OK" "$GREEN2"
    arm selftest-short red "$(printf '%s\n' "$GREEN1" | sed 's/20 of 20 cases/19 of 20 cases/')" "$PEER_OK" "$GREEN2"
    arm selftest-case-wrong red "$(printf '%s\nncm selftest: CRC table refused: Got { frames: 1 } (NOT as expected)' "$GREEN1")" "$PEER_OK" "$GREEN2"
    arm xhci-failed red "$(printf '%s\n[xhci] control request 0x86 failed (completion code 6)' "$GREEN1")" "$PEER_OK" "$GREEN2"
    arm no-fallback-line red "$GREEN1" "$PEER_OK" "$(printf '%s\n' "$GREEN2" | grep -v 'not used')"
    arm fallback-not-ecm red "$GREEN1" "$PEER_OK" "$(printf '%s\n' "$GREEN2" | grep -v 'USB Ethernet adapter 0525')"
    arm fallback-still-ncm red "$GREEN1" "$PEER_OK" "$(printf '%s\n[xhci] NCM: blocks up to 16 KiB in' "$GREEN2")"
    arm fallback-no-dhcp red "$GREEN1" "$PEER_OK" "$(printf '%s\n' "$GREEN2" | grep -v 'DHCP')"
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 24 ]
}

echo "== PC USB CDC-NCM (newer phones' tethering; QEMU usb-net with the CI's NCM test patch) =="
if ! selftest; then echo "PC USBNCM TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBNCM SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBNCM TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
qemu-system-x86_64 -device usb-net,help 2> /dev/null | grep -q 'ncm-badparams' ||
    { echo "PC USBNCM TEST FAILED (harness: this QEMU's usb-net has no NCM mode; build it with scripts/ci/qemu-usbnet-ncm.patch)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBNCM TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbncm.XXXXXX")"
boot() { # boot <serial log> <extra qemu args...> :: <shell commands...>
    local log="$1"; shift
    local q=()
    while [ "$#" -gt 0 ] && [ "$1" != :: ]; do q+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci "${q[@]}" \
        -serial "tcp:127.0.0.1:4884,server,nowait" -qmp "tcp:127.0.0.1:4885,server,nowait" 2> "$log.qemu" &
    local qpid=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4884 4885 "$W/shot.png" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
    grep -v '^$' "$log.qemu" | sed 's/^/  qemu| /' | head -10
}

# boot 1: NCM; the device on a QEMU network hub with user networking (DHCP, the peer) and a UDP socket the
# host's burst frames come in through
BURST_IN=4890  # QEMU listens here for raw frames
BURST_FROM=4891
PORTFILE="$W/port"
python3 - "$PORTFILE" "$BURST_IN" "$BURST_FROM" > "$W/peer.log" 2>&1 <<'PY' &
import socket, sys, time
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
s.listen(1)
s.settimeout(240)
open(sys.argv[1], "w").write(str(s.getsockname()[1]))
try:
    c, a = s.accept()
except socket.timeout:
    print("peer: NO CONNECTION within 240s", flush=True)
    sys.exit(1)
c.settimeout(30)
try:
    data = c.recv(1024)
except socket.timeout:
    data = b""
print("peer received %d bytes: %s" % (len(data), data.decode("ascii", "replace")), flush=True)
if data:
    c.sendall(b"AFRISOVERA-REPLY-OK " + str(len(data)).encode() + b"\n")
time.sleep(1)
c.close()
# raw Ethernet broadcast frames (local experimental type 0x88b5) into the hub, faster than one USB transfer each
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
u.bind(("127.0.0.1", int(sys.argv[3])))
for rnd in range(4):
    for k in range(40):
        f = b"\xff" * 6 + b"\x02\xaf\x00\x00\x00\x09" + b"\x88\xb5" + (b"AFRISOVERA-NCM-BURST %03d " % k).ljust(46, b".")
        u.sendto(f, ("127.0.0.1", int(sys.argv[2])))
    time.sleep(0.5)
print("peer sent 4 bursts of 40 broadcast frames", flush=True)
PY
PEER=$!
for _ in $(seq 1 40); do [ -s "$PORTFILE" ] && break; sleep 0.25; done
PORT="$(cat "$PORTFILE" 2> /dev/null)"
[ -n "$PORT" ] || { kill "$PEER" 2> /dev/null; echo "PC USBNCM TEST FAILED (harness: the host peer never bound a port)"; exit 1; }
echo "  host peer on 127.0.0.1:$PORT (10.0.2.2:$PORT from the guest)"
boot "$W/serial1.log" \
    -netdev user,id=n0 -netdev hubport,id=h0,hubid=0,netdev=n0 \
    -netdev "dgram,id=n1,local.type=inet,local.host=127.0.0.1,local.port=$BURST_IN,remote.type=inet,remote.host=127.0.0.1,remote.port=$BURST_FROM" \
    -netdev hubport,id=h1,hubid=0,netdev=n1 \
    -netdev hubport,id=h2,hubid=0 -device usb-net,netdev=h2,ncm=on,bus=xhci.0 \
    :: "ping" "net tcp send $PORT $PAYLOAD" "lsvirtio" "usbnettest" "usbnettest ncm" "lsvirtio" "usbnettest"
wait "$PEER" 2> /dev/null
tr -d '\r' < "$W/serial1.log" | grep -aE 'USB Ethernet|NCM|ECM|^\[virtio\] slot|DHCP: IP|ping OK|TCP data|^lsvirtio: slot|^usbnettest|^ncm selftest' | sed 's/^/  1| /'
sed 's/^/  peer| /' "$W/peer.log"

# boot 2: the device reports a layout the driver must refuse; the same device as ECM
boot "$W/serial2.log" -netdev user,id=n0 -device usb-net,netdev=n0,ncm=on,ncm-badparams=on,bus=xhci.0 :: "ping" "lsvirtio"
tr -d '\r' < "$W/serial2.log" | grep -aE 'USB Ethernet|NCM|ECM|^\[virtio\] slot|DHCP: IP|ping OK|^lsvirtio: slot' | sed 's/^/  2| /'

echo "  --- grading ---"
ok=1
grade1 "$W/serial1.log" "$W/peer.log" "$W/serial1.log.qemu" || ok=0
grade2 "$W/serial2.log" || ok=0
if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC USBNCM TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBNCM TEST FAILED"
exit 1
