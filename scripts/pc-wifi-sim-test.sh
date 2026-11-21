#!/usr/bin/env bash
# pc-wifi-sim (W4b of the Wi-Fi plan, 14 Nov 2026): Wi-Fi as the PC's network card, end to end in QEMU.
# The Wi-Fi test image (feature wifi-sim, its own CARGO_TARGET_DIR so the normal image is untouched) has a pretend
# radio and a WPA2 access point written from IEEE 802.11-2020 (kernel/src/pc/wifisim.rs) whose uplink is the e1000.
# Everything above the radio is the kernel's real code: the station (wlan/mlme.rs), the supplicant, CCMP, nic.rs,
# the TCP/IP stack and DHCP.
#   1. The normal PC image has no trace of the test radio (its network names and passphrase are not in it); the test
#      image has them (so the check can see them).
#   2. One QEMU boot of the test image with only an e1000e on user networking:
#      - at start-up Wi-Fi is the card, nothing is joined, and DHCP waits;
#      - `wifi scan` lists the three test networks with their security;
#      - `wifi connect AfriSim-wpa3` is refused (WPA3 only); `more wifi connect AfriSim` and
#        `wifi connect AfriSim > f` are each refused by name by the shell (`wifi` is in ASKS_KEY), with no prompt;
#      - `wifi connect AfriSim`: the crypto self-check runs first (not run yet this boot), the passphrase is asked for
#        and typed on the serial line, keys are made, the 4-way handshake runs, and DHCP gets 10.0.2.15 over Wi-Fi;
#      - `wifi status` shows the connection with no bad MIC and no replay; `ping` gets its answer over Wi-Fi;
#      - `wifi disconnect` wipes the keys and `wifi status` says not connected.
#   The passphrase must never appear in what the PC printed, and the longest stretch with interrupts off under the
#   Wi-Fi guard, printed by `wifi status`, must stay under 20 ms (the architect's masked rule). The test image is
#   built with the release profile, as the stick is.
#   scripts/pc-wifi-sim-test.sh              # grader selftest, then the two builds and the QEMU boot
#   scripts/pc-wifi-sim-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PASS='sim-passphrase-1'   # the test image's public test value (kernel/src/pc/wifisim.rs)

grade() { # grade <serial log>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -qE -e "$1" <<< "$S"; }
    has '^\[virtio\] slot [0-9]+: virtio-net \(id=1\), emulated over Wi-Fi \(the test image.s pretend radio\)$' || _f "Wi-Fi is not the network card"
    has '^\[net\] Wi-Fi: no network joined yet' || _f "start-up did not wait for a Wi-Fi network"
    has '^wifi scan: 3 network\(s\) heard$' || _f "wifi scan did not hear the three networks"
    has '^  AfriSim +02:5a:51:4d:00:01 ch +6 +-?[0-9]+ dBm  WPA2-Personal$' || _f "scan: no WPA2 network line"
    has '^  AfriSim-open +02:5a:51:4d:00:02 ch +6 +-?[0-9]+ dBm  open \(no password\)$' || _f "scan: no open network line"
    has '^  AfriSim-wpa3 +02:5a:51:4d:00:03 ch +6 +-?[0-9]+ dBm  WPA3 only \(not yet\)$' || _f "scan: no WPA3 network line"
    has "^wifi connect: 'AfriSim-wpa3' uses WPA3 only \(not yet\); AfriSovera does not join it$" || _f "the WPA3-only network was not refused"
    [ "$(printf '%s\n' "$S" | grep -cE "^more: 'wifi' asks for a key, so it cannot run inside more or >$")" -eq 2 ] ||
        _f "more and > were not both refused by name"
    [ "$(printf '%s\n' "$S" | grep -c "passphrase for 'AfriSim'")" -eq 1 ] || _f "the passphrase prompt was shown other than once"
    local ms
    ms="$(printf '%s\n' "$S" | sed -n 's/^  longest stretch with interrupts off: \([0-9]*\)\.[0-9]* ms .*$/\1/p')"
    [ -n "$ms" ] || _f "no 'longest stretch with interrupts off' line"
    for m in $ms; do [ "$m" -lt 20 ] || _f "interrupts were off for $m ms or more (the rule is under 20 ms)"; done
    has '^wifi connect: the Wi-Fi crypto has not been checked in this boot; checking it first$' || _f "the crypto self-check did not run first"
    has '^wifi selftest: 39 of 39 vectors PASS; Wi-Fi may make keys$' || _f "the crypto self-check did not pass"
    has "passphrase for 'AfriSim' \(it is not shown; Enter ends, Esc cancels\)" || _f "no passphrase prompt"
    has '^wifi connect: making keys \(a few seconds\)$' || _f "no 'making keys' line"
    has "^wifi connect: connected to 'AfriSim' \(WPA2-Personal\)$" || _f "not connected to AfriSim"
    has '^\[net\] DHCP: IP 10\.0\.2\.15, mask 255\.255\.255\.0, gw 10\.0\.2\.2' || _f "no DHCP lease over Wi-Fi"
    has "^wifi status: connected to 'AfriSim' 02:5a:51:4d:00:01 channel 6 signal -?[0-9]+ dBm, WPA2-Personal$" || _f "status: not connected"
    has '^  frames in [1-9][0-9]* out [1-9][0-9]* \(send failures [0-9]+\); dropped: unprotected 0 bad MIC 0 replays 0 ' || _f "status: frames missing, or bad MIC, replays or plaintext seen"
    has '^  address 10\.0\.2\.15$' || _f "status: no address"
    has '^\[net\] ICMP echo reply from 10\.0\.2\.2 \(ping OK\)$' || _f "no ping reply over Wi-Fi"
    has '^wifi disconnect: disconnected; the keys are wiped$' || _f "disconnect did not say the keys are wiped"
    has '^wifi status: not connected$' || _f "status after disconnect is not 'not connected'"
    grep -qF -e "$PASS" <<< "$S" && _f "the passphrase appears in what the PC printed"
    has 'PANIC|panicked' && _f "the kernel panicked"
    [ $fail -eq 0 ] && echo "  graded: scan, refusals, WPA2 join, DHCP, ping, disconnect; passphrase never printed"
    return $fail
}

GOOD="[virtio] slot 0: virtio-net (id=1), emulated over Wi-Fi (the test image's pretend radio)
[net] Wi-Fi: no network joined yet; \`wifi connect <name>\` joins one and then gets an address
afrisovera> wifi scan
wifi scan: 3 network(s) heard
  AfriSim                  02:5a:51:4d:00:01 ch  6  -40 dBm  WPA2-Personal
  AfriSim-open             02:5a:51:4d:00:02 ch  6  -40 dBm  open (no password)
  AfriSim-wpa3             02:5a:51:4d:00:03 ch  6  -40 dBm  WPA3 only (not yet)
afrisovera> wifi connect AfriSim-wpa3
wifi connect: 'AfriSim-wpa3' uses WPA3 only (not yet); AfriSovera does not join it
afrisovera> more wifi connect AfriSim
more: 'wifi' asks for a key, so it cannot run inside more or >
afrisovera> wifi connect AfriSim > wf.txt
more: 'wifi' asks for a key, so it cannot run inside more or >
afrisovera> wifi connect AfriSim
wifi connect: the Wi-Fi crypto has not been checked in this boot; checking it first
wifi selftest: 39 of 39 vectors PASS; Wi-Fi may make keys
passphrase for 'AfriSim' (it is not shown; Enter ends, Esc cancels):
wifi connect: making keys (a few seconds)
wifi connect: joining 'AfriSim'
wifi connect: connected to 'AfriSim' (WPA2-Personal)
[net] DHCP: IP 10.0.2.15, mask 255.255.255.0, gw 10.0.2.2, dns 10.0.2.3
afrisovera> wifi status
wifi status: connected to 'AfriSim' 02:5a:51:4d:00:01 channel 6 signal -40 dBm, WPA2-Personal
  frames in 40 out 12 (send failures 0); dropped: unprotected 0 bad MIC 0 replays 0 not ours 0 malformed 0 queue full 0
  test access point: 8 to the uplink, 6 from it, 0 refused
  longest stretch with interrupts off: 3.250 ms (one received frame)
  address 10.0.2.15
afrisovera> ping
[net] ICMP echo reply from 10.0.2.2 (ping OK)
afrisovera> wifi disconnect
wifi disconnect: disconnected; the keys are wiped
afrisovera> wifi status
wifi status: not connected
  longest stretch with interrupts off: 3.250 ms (one received frame)
afrisovera> "

selftest() {
    local D pass=0 bad=0 f
    D="$(mktemp -d "$T/pc-wifi-sim-selftest.XXXXXX")" || return 1
    printf '%s' "$GOOD" > "$D/good"
    printf '%s' "$GOOD" | sed "s/^passphrase for 'AfriSim' (it is not shown; Enter ends, Esc cancels):.*/& $PASS/" > "$D/echoed"
    printf '%s' "$GOOD" | sed 's/bad MIC 0/bad MIC 2/' > "$D/badmic"
    printf '%s' "$GOOD" | sed 's/unprotected 0/unprotected 1/' > "$D/plain"
    printf '%s' "$GOOD" | grep -v 'ping OK' > "$D/noping"
    printf '%s' "$GOOD" | grep -v 'uses WPA3 only' > "$D/wpa3"
    printf '%s' "$GOOD" | grep -v 'cannot run inside more' > "$D/capture"
    printf '%s' "$GOOD" | sed '0,/cannot run inside more or >$/{//d}' > "$D/onlyone"
    printf '%s' "$GOOD" | sed 's/off: 3.250 ms/off: 25.000 ms/' > "$D/masked"
    printf '%s' "$GOOD" | grep -v 'longest stretch' > "$D/nomask"
    printf '%s' "$GOOD" | grep -v 'checking it first' > "$D/nokat"
    printf '%s' "$GOOD" | sed 's/^wifi status: not connected$/wifi status: connected/' > "$D/stillon"
    printf '%s' "$GOOD" | grep -v '^\[net\] DHCP: IP' > "$D/nodhcp"
    printf '%s' "$GOOD" | sed 's/emulated over Wi-Fi.*/emulated over the Intel NIC (e1000)/' > "$D/wired"
    echo "  --- grader selftest (no QEMU) ---"
    if grade "$D/good" > /dev/null; then pass=$((pass + 1)); else echo "  FAIL selftest: the good log was refused"; grade "$D/good"; bad=$((bad + 1)); fi
    for f in echoed badmic plain noping wpa3 capture onlyone masked nomask nokat stillon nodhcp wired; do
        if grade "$D/$f" > /dev/null; then echo "  FAIL selftest: the '$f' log was accepted"; bad=$((bad + 1)); else pass=$((pass + 1)); fi
    done
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ $bad -eq 0 ]
}

if ! selftest; then echo "PC WIFI-SIM TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC WIFI-SIM SELFTEST ONLY; no measurement taken"; exit 0; }

for d in "${QEMU_DIR:-}" "/c/Program Files/qemu" "$HOME/qemu/bin"; do
    [ -n "$d" ] || continue
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC WIFI-SIM TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build: the normal PC image, then the Wi-Fi test image in its own target folder ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
NORMAL="$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi"
SIMDIR="$PWD/target/wifi-sim"
# the release profile, as scripts/make-pc-usb.sh builds the stick: the masked time is graded on the code the owner
# runs (the debug build under QEMU's software CPU is several times slower and swings with the host: one CCMP frame
# measured 13.9 ms on a cloud runner and 23.9 ms on the cloud runner)
CARGO_TARGET_DIR="$SIMDIR" cargo build --release -p mansa-kernel --bin mansa-pc --features pc,wifi-sim --target x86_64-unknown-uefi 2>&1 | tail -2
SIM="$SIMDIR/x86_64-unknown-uefi/release/mansa-pc.efi"
[ -r "$NORMAL" ] && [ -r "$SIM" ] || { echo "PC WIFI-SIM TEST FAILED (harness: an image was not built)"; exit 1; }

echo "  --- the test radio is only in the test image ---"
for m in AfriSim-wpa3 "$PASS"; do
    if grep -qaF -e "$m" "$NORMAL"; then echo "  FAIL: the normal image contains '$m'"; echo "PC WIFI-SIM TEST FAILED"; exit 1; fi
    grep -qaF -e "$m" "$SIM" || { echo "  FAIL: the test image lacks '$m', so the check above proves nothing"; echo "PC WIFI-SIM TEST FAILED"; exit 1; }
done
echo "  normal image: no test networks, no test passphrase; test image: both"

export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
# the crypto self-check and two passphrase-to-key runs in a debug build under QEMU's software CPU: long silences
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-150}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-400}"
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] || { echo "PC WIFI-SIM TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-wifi-sim.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$SIM" "$W/stick/EFI/BOOT/BOOTX64.EFI"

# RDRAND and RDSEED on: the random generator makes secret bytes (the SNonce) only from a strong source and refuses
# otherwise (QEMU's default CPU has neither; without them the handshake ends "no secret random numbers")
echo "  --- boot: the test image, an e1000e on user networking as the access point's uplink ---"
"$REALQ" -M q35 -m 256M -display none -cpu qemu64,+rdrand,+rdseed \
    -netdev user,id=n0 -device e1000e,netdev=n0 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5474,server,nowait" -qmp "tcp:127.0.0.1:5475,server,nowait" &
Q=$!
# `wifi connect AfriSim` and the passphrase go raw (no wait for a prompt: the command is waiting for the passphrase);
# the next command waits for its own prompt, which comes after the join and DHCP
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5474 5475 "$W/shot.png" \
    "wifi scan" "wifi connect AfriSim-wpa3" "more wifi connect AfriSim" "wifi connect AfriSim > wf.txt" \
    ":wifi connect AfriSim\r" ":$PASS\r" \
    "wifi status" "ping" "wifi disconnect" "wifi status" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
# the passphrase was sent raw by the driver; only what the PC printed is graded (the driver does not log what it sends)
tr -d '\r' < "$W/boot.log" | grep -aE '^(wifi |  [A-Za-z0-9]|passphrase|\[net\]|\[virtio\])' | grep -vF -e "$PASS" | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC WIFI-SIM TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC WIFI-SIM TEST FAILED"
exit 1
