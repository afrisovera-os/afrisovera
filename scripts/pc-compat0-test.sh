#!/usr/bin/env bash
# pc-compat0 (Daraja stage 0 step 3, COMPAT_STAGE0_PLAN section 7): the end-to-end proof. One Linux program built
# from source by us runs on Linux and on AfriSovera, and the two must agree.
#   1. scripts/compat-linux-ref-test.sh: musl 1.2.5 (pinned, checked against Alpine), our hello0, fault0 and bench0
#      built static at 0x10000400000, run on Linux; their stdout, stderr, exit code and strace call list kept in
#      build/compat-ref/ as the reference;
#   2. one bundle per program (cpio newc: the program, data/greeting.txt, daraja/argv), packed here;
#   3. the Linux bridge (userland/daraja-linux) built for x86_64-unknown-none, size-built, flattened; at most 64 KiB
#      (proc.rs CODE_PAGES), refused with a named reason past that;
#   4. the PC kernel built with the compat0 feature (it includes the bridge and the bundles);
#   5. one QEMU boot (-cpu max: the bridge's AT_RANDOM needs RDRAND or RDSEED): linux hello0, linux fault0, linux
#      bench0;
#   6. scripts/lib/compat0-compare.py: per program, stdout and stderr byte for byte, the exit code, and the forwarded
#      call numbers against strace's list (bench0: its exit only; its cycles-per-call line is printed for the
#      performance engineer).
# The judge's selftest runs first. Written for the cloud job (cloud/job.sh); never on the owner's PC.
#   scripts/pc-compat0-test.sh
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
T="${TMPDIR:-/tmp}"
fail() { echo "  FAIL: $*"; echo "PC COMPAT0 TEST FAILED"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- the judge's selftest ---"
"$PY" scripts/lib/compat0-compare.py --selftest || fail "the judge's selftest"

echo "  --- 1. the Linux reference ---"
bash scripts/compat-linux-ref-test.sh > "$T/compat0-ref.log" 2>&1 || { tail -30 "$T/compat0-ref.log"; fail "compat-linux-ref"; }
grep -aE '^  (hello0|fault0|bench0): ' "$T/compat0-ref.log" | sed 's/^/  | /'
REF="$ROOT/build/compat-ref"

echo "  --- 2. the bundles ---"
B="$ROOT/build/compat0"
rm -rf "$B"; mkdir -p "$B/bundles"
"$PY" - "$REF" "$ROOT/tests/compat/bundle" "$B/bundles" <<'EOF' || fail "could not pack the bundles"
import os, sys
ref, files, out = sys.argv[1], sys.argv[2], sys.argv[3]
ARGS = {"hello0": ["hello0", "one", "two"], "fault0": ["fault0"], "bench0": ["bench0"]}

def entry(buf, name, mode, data):
    n = name.encode() + b"\0"
    h = "070701" + "".join("%08x" % v for v in (1, mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(n), 0))
    buf += h.encode() + n
    buf += b"\0" * (-len(buf) % 4)
    buf += data
    buf += b"\0" * (-len(buf) % 4)

for prog, argv in ARGS.items():
    buf = bytearray()
    entry(buf, "data", 0o040755, b"")
    for f in sorted(os.listdir(os.path.join(files, "data"))):
        entry(buf, "data/" + f, 0o100644, open(os.path.join(files, "data", f), "rb").read())
    entry(buf, "daraja", 0o040755, b"")
    entry(buf, "daraja/argv", 0o100644, b"\0".join(a.encode() for a in argv) + b"\0")
    entry(buf, prog, 0o100755, open(os.path.join(ref, prog), "rb").read())
    entry(buf, "TRAILER!!!", 0, b"")
    open(os.path.join(out, prog + ".cpio"), "wb").write(buf)
    print("  %s.cpio: %d bytes" % (prog, len(buf)))
EOF

echo "  --- 3. the bridge ---"
rustup target add x86_64-unknown-none > /dev/null 2>&1 || true
# position-independent code with its GOT filled at link time, as scripts/build-userapp.sh builds rustapp
RUSTFLAGS="-C link-arg=--apply-dynamic-relocs" cargo build --release \
    --manifest-path userland/daraja-linux/Cargo.toml --target x86_64-unknown-none --bin daraja-linux > "$T/bridge-build.log" 2>&1 \
    || { grep -E -A12 '^error' "$T/bridge-build.log" | head -60; fail "the bridge did not build"; }
OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
"$OC" -O binary userland/daraja-linux/target/x86_64-unknown-none/release/daraja-linux "$B/daraja-linux.bin" \
    || fail "llvm-objcopy"
size=$(stat -c %s "$B/daraja-linux.bin")
echo "  daraja-linux.bin: $size bytes, sha256 $(sha256sum "$B/daraja-linux.bin" | cut -d' ' -f1)"
[ "$size" -le 65536 ] || fail "the bridge is $size bytes, more than a native program's 64 KiB (proc.rs CODE_PAGES)"

echo "  --- 4. the kernel with compat0 ---"
DARAJA_BRIDGE_BIN="$B/daraja-linux.bin" DARAJA_BUNDLE_DIR="$B/bundles" \
    cargo build -p mansa-kernel --bin mansa-pc --features pc,compat0 --target x86_64-unknown-uefi > "$T/kernel-build.log" 2>&1 \
    || { grep -E -A20 '^error' "$T/kernel-build.log" | head -60; fail "the kernel did not build with compat0"; }
PC_EFI="$ROOT/target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$PC_EFI" ] || fail "$PC_EFI was not built"

echo "  --- 5. boot: linux hello0, fault0, bench0 ---"
for d in "${QEMU_DIR:-}" "$HOME/qemu/bin"; do
    [ -n "$d" ] || continue
    [ -x "$d/qemu-system-x86_64" ] && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || fail "harness: qemu-system-x86_64 not on PATH"
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
done
[ -n "$OVMF" ] || fail "harness: no x86-64 UEFI firmware"
W="$(mktemp -d "$T/pc-compat0.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-120}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-240}"
"$REALQ" -M q35 -cpu max -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$OVMF" \
    -drive "if=pflash,format=raw,unit=1,file=$W/vars.fd" \
    -drive "format=raw,file=fat:rw:$W/stick" \
    -serial "tcp:127.0.0.1:5478,server,nowait" -qmp "tcp:127.0.0.1:5479,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5478 5479 "$W/shot.png" \
    "linux hello0" "linux fault0" "linux bench0" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[daraja\] (linux|exit|unknown|bridge stopped)|^\[daraja\] P[0-9] (personality|ended|memory)|NOT (given|spawned)' | sed 's/^/  | /'

echo "  --- 6. AfriSovera against Linux ---"
grep -aE '^\[daraja\] out1 ' "$W/boot.log" > /dev/null || echo "  (no stdout lines at all)"
# the unknown-call limit on the fault path (the architect, 21 Nov): fault0 makes getpriority (140) 5 times, which the
# bridge does not answer; its section shows 3 lines, then the summary for the other 2 before the fault's exit line.
# A here-string, not a pipe, for the greps (the pipefail SIGPIPE trap).
F0="$(tr -d '\r' < "$W/boot.log" | sed -n '/linux fault0: begin/,/linux fault0: end/p')"
f0_lines="$(grep -cE '^\[daraja\] unknown call 140$' <<< "$F0")"
f0_sum="$(grep -nE '^\[daraja\] unknown call 140: 2 more not shown$' <<< "$F0" | cut -d: -f1 | head -1)"
f0_exit="$(grep -nE '^\[daraja\] exit 139 ' <<< "$F0" | cut -d: -f1 | head -1)"
echo "  fault0: $f0_lines unknown-call lines, summary at line ${f0_sum:-none}, exit at line ${f0_exit:-none}"
if [ "$f0_lines" != 3 ] || [ -z "$f0_sum" ] || [ -z "$f0_exit" ] || [ "$f0_sum" -gt "$f0_exit" ]; then
    echo "  FAIL: fault0's unknown-call summary is missing, or not before its exit line"
    echo "  logs kept in $W"
    echo "PC COMPAT0 TEST FAILED"
    exit 1
fi
if "$PY" scripts/lib/compat0-compare.py "$W/boot.log" "$REF" hello0 fault0 bench0; then
    # the serial log's lines end in CR: strip it first, or the end-of-line match below finds nothing (first runs)
    cyc="$(tr -d '\r' < "$W/boot.log" | sed -n '/linux bench0: begin/,/linux bench0: end/p' | sed -nE 's/^\[daraja\] out1 ([0-9a-f]+)$/\1/p' | head -1)"
    [ -n "$cyc" ] && echo "  bench0 through the bridge: $("$PY" -c "import sys; print(bytes.fromhex(sys.argv[1]).decode().strip())" "$cyc")" \
        && echo "  bench0 on Linux: $(cat "$REF/bench0.out1" | tr -d '\n')"
    rm -rf "$W"
    echo "PC COMPAT0 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC COMPAT0 TEST FAILED"
exit 1
