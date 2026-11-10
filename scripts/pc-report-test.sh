#!/usr/bin/env bash
# The field report (8 Nov 2026, docs/design/field-report.md): `report <ip>[:port]` sends this
# PC's details ONCE to tools/report-receiver.py on the owner's main PC.
#
# Two boots of the PC kernel (q35 + UEFI), each with ONLY a real-card model on user-mode
# networking: an Intel 82574L (`e1000e`, pc/e1000.rs) and a Realtek RTL8139C+ (`rtl8139`,
# pc/rtl8169.rs). In each, the REAL receiver (tools/report-receiver.py --once) listens on the
# host, and the guest types:
#   report                     -> the plain-words usage
#   report 10.0.2.2:<closed>   -> refused in plain words (nothing listens there)
#   report 10.0.2.2:<port>     -> ARP for the receiver itself (same subnet), a windowed
#                                 multi-segment send, a clean close
# Graded by scripts/lib/report-grade.py (its own 20-arm selftest runs first): the receiver's
# saved file must be byte-for-byte the size the guest says it sent, its end line must count
# the bytes before it exactly (nothing lost or doubled), every probe section in order, the
# card named in its probe and in the boot log.
#   scripts/pc-report-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-report-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

echo "== PC field report (report <ip> -> tools/report-receiver.py) =="
echo "  --- grader selftest (no QEMU) ---"
if ! python3 scripts/lib/report-grade.py --selftest; then
    echo "PC REPORT TEST FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "PC REPORT SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC REPORT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC REPORT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

# a port nothing listens on: bound, read, closed
closed_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

fail=0
W="$(mktemp -d "$T/pc-report.XXXXXX")"
for card in e1000 rtl; do
    case "$card" in
        e1000) DEV=e1000e ;;
        rtl) DEV=rtl8139 ;;
    esac
    D="$W/$card"
    mkdir -p "$D/reports"
    python3 tools/report-receiver.py --bind 127.0.0.1 --port 0 --once --timeout 60 --out "$D/reports" \
        > "$D/receiver.log" 2>&1 &
    RPID=$!
    PORT=""
    for _ in $(seq 1 40); do
        PORT="$(sed -n 's/.*listening on port \([0-9][0-9]*\)$/\1/p' "$D/receiver.log" 2> /dev/null | head -1)"
        [ -n "$PORT" ] && break
        sleep 0.25
    done
    [ -n "$PORT" ] || { kill "$RPID" 2> /dev/null; echo "PC REPORT TEST FAILED (harness: the receiver never listened)"; exit 1; }
    CLOSED="$(closed_port)"
    echo "  [$card] receiver on 127.0.0.1:$PORT (10.0.2.2:$PORT from the guest); closed port $CLOSED"

    qemu-system-aarch64 -M virt -m 256M -display none \
        -netdev user,id=n0 -device "$DEV",netdev=n0 \
        -serial "tcp:127.0.0.1:5146,server,nowait" -qmp "tcp:127.0.0.1:5147,server,nowait" &
    QPID=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5146 5147 "$D/shot.png" \
        "report" "report 10.0.2.2:$CLOSED" "report 10.0.2.2:$PORT" > "$D/serial.log" 2>&1
    kill "$QPID" 2> /dev/null
    wait "$QPID" 2> /dev/null
    for _ in $(seq 1 20); do kill -0 "$RPID" 2> /dev/null || break; sleep 0.5; done
    kill "$RPID" 2> /dev/null
    wait "$RPID" 2> /dev/null
    tr -d '\r' < "$D/serial.log" | grep -aE '^\[report\]|^report: (NOT|send)|DHCP: IP|PANIC|EXCEPTION' | sed 's/^/  | /'
    sed 's/^/  receiver| /' "$D/receiver.log" | grep -v '^  receiver|     report '
    echo "  --- grading ($card) ---"
    if ! python3 scripts/lib/report-grade.py "$D/serial.log" "$D/receiver.log" "$D/reports" "$card" "$PORT" "$CLOSED"; then
        fail=1
    fi
done

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC REPORT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC REPORT TEST FAILED"
exit 1
