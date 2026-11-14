#!/usr/bin/env bash
# PC track, mesh F3 step 3 (13 Nov 2026): the TPM's random number generator as a counted strong source (ADR 0008
# Decision 1a; kernel/src/pc/tpm.rs over the TIS FIFO, command bytes and parsing in kernel/src/pc/tpm_wire.rs).
# The CPU has no RDRAND or RDSEED (-cpu qemu64,-rdrand,-rdseed), so without a TPM `fill_secret` refuses; with a
# TPM switched on, it must succeed from the TPM alone. Four power-ons, each with `rng sources`, `rng secret 32`, `rng`:
#   boot 1: swtpm --tpm2 behind -tpmdev emulator -device tpm-tis: the `[tpm] TPM 2.0 over TIS` line, counted now:
#           tpm, `rng secret: ok` with counted bits=256, source tpm healthy with >= 1024 samples (start-up test).
#   boot 2: swtpm in TPM 1.2 mode, prepared with swtpm_setup (enabled and activated): the same through TCPA.
#   boot 3: swtpm in TPM 1.2 mode, switched off before QEMU starts (TSC_PhysicalPresence, then TPM_PhysicalDisable,
#           sent by tpm12_disable below; it reads the permanent `disable` flag back and stops if it is not set):
#           `[tpm] off (switched off in the firmware setup; ...)`, nothing counted, `rng secret: refused` (Q2).
#           Measured on os2 43464292626: a FRESH swtpm 1.2 state comes up enabled and activated, so a fresh state
#           is not an "off" TPM; the first version of this boot assumed it was, and boot 3 went green-for-on.
#   boot 4: a synthetic TCPA table with NO TPM behind it (the table lies): `[tpm] off (table but no TIS answer)`,
#           nothing counted, refused.
# Architect's condition 7: `rng secret 32` prints exactly one line, which has no room for a byte of the secret.
#   scripts/pc-tpm-test.sh              # grader selftest, then the four QEMU boots (needs swtpm and swtpm_setup)
#   scripts/pc-tpm-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

R_ON='^\[tpm\] TPM (1\.2|2\.0) over TIS at 0xfed40000: vendor 0x[0-9a-f]{4} device 0x[0-9a-f]{4}, GetRandom ok \(32 bytes in [0-9]+ us\)(, started by us)?$'
R_SECRET_OK='^rng secret: ok, 32 bytes, not shown \(secret_ok=[1-9][0-9]* secret_refused=0 counted bits=256\)$'
R_SECRET_NO='^rng secret: refused, no counted source \(secret_ok=0 secret_refused=[1-9][0-9]* counted bits=0\)$'
R_REFUSES='^rng sources: counted now: none, so fill_secret refuses on this PC; jitter is not counted yet \(ADR 0008 step 2b\)$'

clean() { tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }

# The lines `rng secret 32` printed: everything after its prompt line up to the next prompt (condition 7).
secret_out() { awk '/^afrisovera> rng secret 32$/{on=1; next} /^afrisovera>/{on=0} on' <<< "$1"; }

grade_on() { # grade_on <log> <boot name> <1.2|2.0>
    local S fail=0 v="$3" n
    S="$(clean "$1")"
    has() { grep -aqE "$1" <<< "$S"; }
    _f() { echo "  FAIL: $2: $1"; fail=1; }
    if grep -aqE 'KERNEL PANIC|\[pc\] (fault|exception)' <<< "$S"; then _f "the kernel panicked or faulted" "$2"; fi
    has '^rng sources: rdrand no  rdseed no ' || _f "the CPU must have neither RDRAND nor RDSEED for this suite" "$2"
    grep -aE "$R_ON" <<< "$S" | grep -aq "TPM $v over TIS" || _f "no '[tpm] TPM $v over TIS ... GetRandom ok' boot line" "$2"
    has "^rng sources: tpm: TPM ${v//./\\.} over TIS at 0xfed40000: " || _f "'rng sources' has no 'tpm: TPM $v' line" "$2"
    has '^rng sources: counted now: tpm; jitter is not counted yet \(ADR 0008 step 2b\)$' ||
        _f "the TPM is on, but 'counted now' does not say tpm" "$2"
    n="$(secret_out "$S")"
    if ! grep -aqE "$R_SECRET_OK" <<< "$n"; then _f "'rng secret 32' did not succeed from the TPM alone (got: $(head -1 <<< "$n"))" "$2"
    elif [ "$(grep -ac . <<< "$n")" -ne 1 ]; then _f "'rng secret 32' printed more than its one line (condition 7)" "$2"; fi
    has '^\[rng\] counted bits=256 of 256 at the last \(re\)seed$' || _f "the last seed did not count 256 bits" "$2"
    n="$(grep -aE '^\[rng\] source tpm: healthy samples=[0-9]+$' <<< "$S" | head -1 | sed 's/.*samples=//')"
    if [ -z "$n" ]; then _f "plain 'rng' does not show source tpm healthy" "$2"
    elif [ "$n" -lt 1024 ]; then _f "source tpm has $n samples, fewer than the 1024 of the start-up test" "$2"; fi
    has '^\[rng\] source tpm claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512$' ||
        _f "source tpm does not claim H=256/64 with cutoffs 6 and 62" "$2"
    has '^\[rng\] source rdrand: absent samples=0$' || _f "rdrand is not absent, so the TPM was not the only source" "$2"
    return "$fail"
}

grade_off() { # grade_off <log> <boot name> <exact reason>
    local S fail=0 n
    S="$(clean "$1")"
    has() { grep -aqF -x "$1" <<< "$S"; }
    _f() { echo "  FAIL: $2: $1"; fail=1; }
    if grep -aqE 'KERNEL PANIC|\[pc\] (fault|exception)' <<< "$S"; then _f "the kernel panicked or faulted" "$2"; fi
    has "[tpm] off ($3)" || _f "no '[tpm] off ($3)' boot line" "$2"
    has "rng sources: tpm: off ($3)" || _f "'rng sources' does not say 'tpm: off ($3)'" "$2"
    grep -aqE "$R_REFUSES" <<< "$S" || _f "the TPM is off, but 'counted now' does not say fill_secret refuses" "$2"
    n="$(secret_out "$S")"
    if ! grep -aqE "$R_SECRET_NO" <<< "$n"; then _f "'rng secret 32' was not refused (got: $(head -1 <<< "$n"))" "$2"
    elif [ "$(grep -ac . <<< "$n")" -ne 1 ]; then _f "'rng secret 32' printed more than its one line (condition 7)" "$2"; fi
    has '[rng] counted bits=0 of 256 at the last (re)seed' || _f "bits were counted with the TPM off" "$2"
    grep -aqE '^\[rng\] source tpm: absent samples=0$' <<< "$S" || _f "source tpm is not absent with the TPM off" "$2"
    return "$fail"
}

OFF12='switched off in the firmware setup; switch it on there to use it'
OFFLIE='table but no TIS answer'

on_fixture() { # on_fixture <1.2|2.0>
    cat <<EOF
[tpm] TPM $1 over TIS at 0xfed40000: vendor 0x1014 device 0x0001, GetRandom ok (32 bytes in 412 us)
[pc] MANSA-PC BOOT OK
afrisovera> rng sources
rng sources: cpu AuthenticAMD family 0x6 model 0x6 stepping 3
rng sources: rdrand no  rdseed no  invariant-tsc no  hypervisor no
rng sources: tpm: TPM $1 over TIS at 0xfed40000: vendor 0x1014 device 0x0001, GetRandom ok (32 bytes in 412 us)
rng sources: counted now: tpm; jitter is not counted yet (ADR 0008 step 2b)
afrisovera> rng secret 32
rng secret: ok, 32 bytes, not shown (secret_ok=1 secret_refused=0 counted bits=256)
afrisovera> rng
[rng] HMAC_DRBG-SHA256 kat=PASS seeded=true strong_bytes=64 reseeds=2 secret_ok=1 secret_refused=0
[rng] source rdseed: absent samples=0
[rng] source rdrand: absent samples=0
[rng] source tpm: healthy samples=1216
[rng] counted bits=256 of 256 at the last (re)seed
[rng] source rdseed claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512
[rng] source rdrand claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512
[rng] source tpm claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512
afrisovera>
EOF
}

off_fixture() { # off_fixture <reason>
    cat <<EOF
[tpm] off ($1)
[pc] MANSA-PC BOOT OK
afrisovera> rng sources
rng sources: rdrand no  rdseed no  invariant-tsc no  hypervisor no
rng sources: tpm: off ($1)
rng sources: counted now: none, so fill_secret refuses on this PC; jitter is not counted yet (ADR 0008 step 2b)
afrisovera> rng secret 32
rng secret: refused, no counted source (secret_ok=0 secret_refused=1 counted bits=0)
afrisovera> rng
[rng] HMAC_DRBG-SHA256 kat=PASS seeded=true strong_bytes=0 reseeds=2 secret_ok=0 secret_refused=1
[rng] source tpm: absent samples=0
[rng] counted bits=0 of 256 at the last (re)seed
afrisovera>
EOF
}

selftest() {
    local pass=0 bad=0 D G20 G12 GOFF GLIE
    D="$(mktemp -d "$T/pc-tpm-selftest.XXXXXX")" || return 1
    G20="$(on_fixture 2.0)"; G12="$(on_fixture 1.2)"; GOFF="$(off_fixture "$OFF12")"; GLIE="$(off_fixture "$OFFLIE")"
    arm() { # arm <name> <expected> <text> <grader> <args...>
        local name="$1" want="$2" text="$3" got; shift 3
        printf '%s\n' "$text" > "$D/in"
        if "$1" "$D/in" "${@:2}" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$want" ]; then pass=$((pass+1)); echo "    arm $name: $got (expected $want) OK"
        else bad=$((bad+1)); echo "    arm $name: $got but expected $want  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm 20-green green "$G20" grade_on boot1 2.0
    arm 20-crlf green "$(sed 's/$/\r/' <<< "$G20")" grade_on boot1 2.0
    arm 20-started-by-us green "$(sed 's/412 us)$/412 us), started by us/' <<< "$G20")" grade_on boot1 2.0
    arm 20-is-12 red "$G12" grade_on boot1 2.0
    arm 20-no-boot-line red "$(grep -v '^\[tpm\]' <<< "$G20")" grade_on boot1 2.0
    arm 20-not-counted red "$(sed 's/counted now: tpm;/counted now: none, so fill_secret refuses on this PC;/' <<< "$G20")" grade_on boot1 2.0
    arm 20-secret-refused red "$(sed 's/^rng secret: ok.*/rng secret: refused, no counted source (secret_ok=0 secret_refused=1 counted bits=0)/' <<< "$G20")" grade_on boot1 2.0
    arm 20-secret-shows-bytes red "$(sed 's/^rng secret: ok.*/&\n9f3a01c2e4b7d6a8/' <<< "$G20")" grade_on boot1 2.0
    arm 20-bits-short red "$(sed 's/counted bits=256 of 256/counted bits=128 of 256/' <<< "$G20")" grade_on boot1 2.0
    arm 20-few-samples red "$(sed 's/samples=1216/samples=64/' <<< "$G20")" grade_on boot1 2.0
    arm 20-tpm-failed red "$(sed 's/source tpm: healthy/source tpm: FAILED-health/' <<< "$G20")" grade_on boot1 2.0
    arm 20-claim-changed red "$(sed 's/source tpm claim: H=256/source tpm claim: H=512/' <<< "$G20")" grade_on boot1 2.0
    arm 20-rdrand-present red "$(sed 's/source rdrand: absent samples=0/source rdrand: healthy samples=1024/' <<< "$G20")" grade_on boot1 2.0
    arm 20-panic red "$(printf '%s\nKERNEL PANIC\n' "$G20")" grade_on boot1 2.0
    arm 20-empty red "" grade_on boot1 2.0
    arm 12-green green "$G12" grade_on boot2 1.2
    arm 12-is-20 red "$G20" grade_on boot2 1.2
    arm off-green green "$GOFF" grade_off boot3 "$OFF12"
    arm off-crlf green "$(sed 's/$/\r/' <<< "$GOFF")" grade_off boot3 "$OFF12"
    arm off-but-counted red "$G12" grade_off boot3 "$OFF12"
    arm off-wrong-reason red "$GLIE" grade_off boot3 "$OFF12"
    arm off-secret-ok red "$(sed 's/^rng secret: refused.*/rng secret: ok, 32 bytes, not shown (secret_ok=1 secret_refused=0 counted bits=256)/' <<< "$GOFF")" grade_off boot3 "$OFF12"
    arm off-secret-shows-bytes red "$(sed 's/^rng secret: refused.*/&\n00000000/' <<< "$GOFF")" grade_off boot3 "$OFF12"
    arm off-bits-counted red "$(sed 's/counted bits=0 of 256/counted bits=256 of 256/' <<< "$GOFF")" grade_off boot3 "$OFF12"
    arm off-tpm-present red "$(sed 's/source tpm: absent samples=0/source tpm: healthy samples=1024/' <<< "$GOFF")" grade_off boot3 "$OFF12"
    arm off-panic red "$(printf '%s\nKERNEL PANIC\n' "$GOFF")" grade_off boot3 "$OFF12"
    arm lie-green green "$GLIE" grade_off boot4 "$OFFLIE"
    arm lie-none red "$(off_fixture 'no TCPA or TPM2 table' | sed 's/^\[tpm\] off (no TCPA or TPM2 table)/[tpm] none (no TCPA or TPM2 table)/')" grade_off boot4 "$OFFLIE"
    arm lie-empty red "" grade_off boot4 "$OFFLIE"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 29 ]
}

echo "== PC TPM: the TPM's RNG as a counted source (mesh F3 step 3) =="
if ! selftest; then echo "PC TPM TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC TPM SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in /usr/local/bin /usr/bin; do
    [ -x "$d/qemu-system-x86_64" ] && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC TPM TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
for t in swtpm swtpm_setup; do
    command -v "$t" > /dev/null || { echo "PC TPM TEST FAILED (harness: $t not installed; apt-get install swtpm swtpm-tools)"; exit 1; }
done
qemu-system-x86_64 -tpmdev help 2>&1 | grep -q emulator ||
    { echo "PC TPM TEST FAILED (harness: this QEMU has no TPM emulator backend)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC TPM TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE_4M.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC TPM TEST FAILED (harness: no x86-64 UEFI firmware found)"; exit 1; }

W="$(mktemp -d "$T/pc-tpm.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
# boot 4's lying table: a TCPA client body (platform class u16, LAML u32, LASA u64) and no TPM device behind it
"$PY" - "$W" <<'EOP'
import struct, sys
open(sys.argv[1] + "/tcpa.bin", "wb").write(struct.pack("<HIQ", 0, 0x10000, 0))
EOP

SWTPM_PID=""
swtpm_start() { # swtpm_start <state dir> [--tpm2]
    mkdir -p "$1"
    swtpm socket --tpmstate "dir=$1" --ctrl "type=unixio,path=$1/sock" ${2:-} --log "file=$1/swtpm.log,level=1" &
    SWTPM_PID=$!
    for _ in $(seq 1 50); do [ -S "$1/sock" ] && return 0; sleep 0.1; done
    echo "  swtpm did not open $1/sock"; return 1
}
swtpm_stop() { [ -n "$SWTPM_PID" ] && kill "$SWTPM_PID" 2> /dev/null; wait "$SWTPM_PID" 2> /dev/null; SWTPM_PID=""; }
trap swtpm_stop EXIT

boot() { # boot <log> [extra qemu args...]
    local log="$1"; shift
    "$REALQ" -M q35 -m 256M -display none -cpu qemu64,-rdrand,-rdseed,-hypervisor "$@" \
        -drive "if=pflash,format=raw,readonly=on,file=$OVMF" \
        -drive "format=raw,file=fat:rw:$W/stick" \
        -serial "tcp:127.0.0.1:5770,server,nowait" -qmp "tcp:127.0.0.1:5771,server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5770 5771 "$W/shot.png" \
        "rng sources" "rng secret 32" "rng" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
# tpm12_disable <state dir>: run swtpm 1.2 on a TCP port (started for us), assert physical presence and send
# TPM_PhysicalDisable, then read TPM_PERMANENT_FLAGS back: the state on disk is then a TPM switched off in firmware.
tpm12_disable() {
    mkdir -p "$1"
    swtpm socket --tpmstate "dir=$1" --server type=tcp,port=5772 --ctrl type=tcp,port=5773         --flags not-need-init,startup-clear --log "file=$1/swtpm-disable.log,level=1" &
    local p=$! r=0
    "$PY" - <<'EOP' || r=1
import socket, struct, sys, time
def cmd(b):
    for _ in range(50):
        try:
            s = socket.create_connection(("127.0.0.1", 5772), timeout=5); break
        except OSError:
            time.sleep(0.1)
    else:
        sys.exit("swtpm did not listen on 5772")
    s.sendall(b)
    h = b""
    while len(h) < 10:
        h += s.recv(10 - len(h))
    n = struct.unpack(">I", h[2:6])[0]
    body = b""
    while len(body) < n - 10:
        body += s.recv(n - 10 - len(body))
    s.close()
    return struct.unpack(">I", h[6:10])[0], body
pp = lambda v: struct.pack(">HIIH", 0xC1, 12, 0x4000000A, v)          # TSC_PhysicalPresence
for name, c in [("PP cmd-enable", pp(0x0020)), ("PP present", pp(0x0008)),
                ("PhysicalDisable", struct.pack(">HII", 0xC1, 10, 0x70))]:
    rc, _ = cmd(c)
    print("  tpm12_disable: %s rc=0x%x" % (name, rc))
    if rc != 0:
        sys.exit(1)
rc, body = cmd(struct.pack(">HIIIII", 0xC1, 22, 0x65, 4, 4, 0x108))      # GetCapability(TPM_CAP_FLAG_PERMANENT)
print("  tpm12_disable: permanent flags rc=0x%x disable=%d" % (rc, body[6] if len(body) > 6 else -1))
sys.exit(0 if rc == 0 and len(body) > 6 and body[6] == 1 else 1)
EOP
    kill "$p" 2> /dev/null; wait "$p" 2> /dev/null
    return "$r"
}
tpmargs() { echo "-chardev socket,id=chrtpm,path=$1/sock -tpmdev emulator,id=tpm0,chardev=chrtpm -device tpm-tis,tpmdev=tpm0"; }
show() { clean "$1" | grep -aE '^\[tpm\]|^rng (sources: (tpm|counted)|secret)|^\[rng\] (source tpm|counted)' | sed 's/^/  | /'; }

echo "  --- boot 1: TPM 2.0 (swtpm --tpm2) ---"
swtpm_start "$W/t20" --tpm2 && boot "$W/boot1.log" $(tpmargs "$W/t20"); swtpm_stop; show "$W/boot1.log"
echo "  --- boot 2: TPM 1.2, swtpm_setup (enabled and activated) ---"
swtpm_setup --tpmstate "$W/t12on" --createek --overwrite > "$W/setup12.log" 2>&1 ||
    { mkdir -p "$W/t12on"; swtpm_setup --tpmstate "$W/t12on" --createek --overwrite > "$W/setup12.log" 2>&1; } ||
    echo "  swtpm_setup failed: $(tail -3 "$W/setup12.log" | tr '\n' ' ')"
swtpm_start "$W/t12on" && boot "$W/boot2.log" $(tpmargs "$W/t12on"); swtpm_stop; show "$W/boot2.log"
echo "  --- boot 3: TPM 1.2, switched off (TPM_PhysicalDisable before boot) ---"
if tpm12_disable "$W/t12off"; then
    swtpm_start "$W/t12off" && boot "$W/boot3.log" $(tpmargs "$W/t12off"); swtpm_stop
else
    echo "  could not switch the TPM 1.2 off; boot 3 not run" > "$W/boot3.log"
fi
show "$W/boot3.log"
echo "  --- boot 4: a TCPA table with no TPM behind it ---"
boot "$W/boot4.log" -acpitable "sig=TCPA,rev=2,oem_id=AFRI,oem_table_id=AFRITEST,data=$W/tcpa.bin"; show "$W/boot4.log"

echo "  --- grading ---"
ok=0
grade_on "$W/boot1.log" boot1 2.0 || ok=1
grade_on "$W/boot2.log" boot2 1.2 || ok=1
grade_off "$W/boot3.log" boot3 "$OFF12" || ok=1
grade_off "$W/boot4.log" boot4 "$OFFLIE" || ok=1
if [ "$ok" -eq 0 ]; then
    rm -rf "$W"
    echo "PC TPM TEST PASSED"
    exit 0
fi
for f in "$W"/boot*.log; do echo "  ---- $f (tail) ----"; clean "$f" | tail -25 | sed 's/^/  | /'; done
for f in "$W"/t*/swtpm.log "$W/setup12.log"; do [ -s "$f" ] && { echo "  ---- $f ----"; tail -10 "$f" | sed 's/^/  | /'; }; done
echo "PC TPM TEST FAILED"
exit 1
