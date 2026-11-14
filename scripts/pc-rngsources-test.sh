#!/usr/bin/env bash
# PC track, ADR 0008 step 1 (12 Nov 2026): diagnostics for counted entropy on PCs without RDSEED or RDRAND. No
# counting changes (drbg.rs is untouched); the ARM image does not change.
#   `rng sources`          the CPU class, the CPUID flags (rdrand, rdseed, invariant TSC, hypervisor) and whether the
#                          ACPI XSDT lists a TCPA (TPM 1.2) or TPM2 (TPM 2.0) table, read-only
#   `rng jitter dump <n>`  n raw deltas of the jitter loop step 2 will assess, in hex, with a sha256 over them;
#                          scripts/lib/jitter-log.py checks the log and writes the ea_non_iid input
#
# Two power-ons with qemu-system-x86_64 DIRECTLY:
#   boot 1: -cpu qemu64,+hypervisor, no TPM tables: sources says hypervisor yes, both tables absent, the "counted now"
#           line agrees with the rdrand/rdseed flags; a 4096-sample dump passes jitter-log.py (count, min, max, mean,
#           distinct low bytes, sha256 all agree) and carries the VM warning; `rng jitter dump 0` gives the usage
#           line; plain `rng` still runs the shared code (kat=PASS).
#   boot 2: -cpu qemu64,-hypervisor, plus two synthetic tables from QEMU's -acpitable: TCPA (rev 2, 50 B) and TPM2
#           (rev 4, 52 B, start method 7 = CRB): sources says hypervisor no and lists both tables with those values.
#   So the hypervisor answer is graded both ways, and the table answers are graded present and absent.
# F3 step 2a (13 Nov 2026): plain `rng` also prints the counted bits of the last seed and, per source, its claimed
#   entropy and the health-test cutoffs that claim gives. Boot 1 grades both PC sources at the unchanged claim
#   (H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512, the old constants) and 0 counted bits without RDRAND.
# F3 step 3 (13 Nov 2026): the TPM is a counted source now. `rng sources` gains a `tpm:` line, and the "counted now"
#   line names what is counted ("jitter is not counted yet (ADR 0008 step 2b)"). Boot 1 (no tables) grades
#   `[tpm] none` and `tpm: none`; boot 2's synthetic TPM2 table says CRB, so the TPM must be off for that reason
#   and nothing may be counted. The TPM itself is graded by scripts/pc-tpm-test.sh (swtpm).
#   scripts/pc-rngsources-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-rngsources-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python
N=4096

R_CPU='^rng sources: cpu [^ ]+ family 0x[0-9a-f]+ model 0x[0-9a-f]+ stepping [0-9]+$'
R_FLAGS='^rng sources: rdrand (yes|no)  rdseed (yes|no)  invariant-tsc (yes|no)  hypervisor (yes|no)$'
R_COUNTED='^rng sources: counted now: (rdseed/rdrand|rdseed/rdrand, tpm|tpm|none, so fill_secret refuses on this PC); jitter is not counted yet \(ADR 0008 step 2b\)$'
R_USAGE='^rng jitter: type rng jitter dump <n>, n from 1 to 100000000 \(e\.g\. rng jitter dump 1000000\)$'
R_VMWARN='^rng jitter dump: a hypervisor is present, so these are the VM.s timings, not the CPU.s$'

grade1() { # grade1 <serial log of boot 1>; prints FAIL lines, returns 0 when green
    local S fail=0 flags rdr rds hyp
    S="$(tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    _f() { echo "  FAIL: boot 1: $1"; fail=1; }
    if grep -aqE 'KERNEL PANIC|\[pc\] (fault|exception)' <<< "$S"; then _f "the kernel panicked or faulted"; fi
    has "$R_CPU" || _f "no 'rng sources: cpu' line"
    flags="$(grep -aE "$R_FLAGS" <<< "$S" | head -1)"
    if [ -z "$flags" ]; then _f "no 'rng sources: rdrand .. hypervisor ..' line"
    else
        rdr="$(sed -E 's/.*rdrand (yes|no) .*/\1/' <<< "$flags")"; rds="$(sed -E 's/.*rdseed (yes|no) .*/\1/' <<< "$flags")"
        hyp="$(sed -E 's/.*hypervisor (yes|no)$/\1/' <<< "$flags")"
        [ "$hyp" = yes ] || _f "QEMU was started with +hypervisor but the kernel says hypervisor $hyp"
        if [ "$rdr" = yes ] || [ "$rds" = yes ]; then
            has '^rng sources: counted now: rdseed/rdrand;' || _f "rdrand or rdseed is present but 'counted now' does not say so"
        else
            has '^rng sources: counted now: none, so fill_secret refuses on this PC;' || _f "no rdrand/rdseed but 'counted now' does not say fill_secret refuses"
        fi
    fi
    has "$R_COUNTED" || _f "no 'counted now' line in the expected words"
    has '^rng sources: acpi TCPA \(TPM 1\.2\) absent$' || _f "no TPM tables were given, but TCPA is not reported absent"
    has '^rng sources: acpi TPM2 \(TPM 2\.0\) absent$' || _f "no TPM tables were given, but TPM2 is not reported absent"
    has '^\[tpm\] none \(no TCPA or TPM2 table\)$' || _f "no TPM tables, but no '[tpm] none' boot line"
    has '^rng sources: tpm: none \(no TCPA or TPM2 table\)$' || _f "no TPM tables, but no 'rng sources: tpm: none' line"
    has "^rng jitter dump: n=$N loop=mem4096-stride67 block=1024 tsc_hz=[0-9]+ hypervisor=yes \(TSC ticks, hex, 16 per line\)\$" ||
        _f "no dump header for n=$N with hypervisor=yes"
    has "$R_VMWARN" || _f "the dump ran under a hypervisor but did not say its timings are the VM's"
    printf '%s\n' "$S" > "$D1/clean.log"
    if ! "$PY" scripts/lib/jitter-log.py "$D1/clean.log" > "$D1/jl.out" 2>&1; then
        _f "jitter-log.py rejects the dump: $(tr '\n' ' ' < "$D1/jl.out")"
    elif ! grep -qE "^jitter-log: OK, n=$N deltas" "$D1/jl.out"; then _f "jitter-log.py did not count $N deltas"; fi
    has "$R_USAGE" || _f "'rng jitter dump 0' did not give the usage line"
    has '^\[rng\] HMAC_DRBG-SHA256 kat=PASS ' || _f "plain 'rng' no longer runs the shared generator status (kat=PASS)"
    has '^\[rng\] counted bits=0 of 256 at the last \(re\)seed$' ||
        _f "no 'counted bits=0 of 256' line (no rdrand/rdseed under this CPU, so nothing may be counted)"
    for src in rdseed rdrand; do
        has "^\[rng\] source $src claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512\$" ||
            _f "source $src does not claim H=256/64 with the old cutoffs 6 and 62 (F3 step 2a changes no claim)"
    done
    return "$fail"
}

grade2() { # grade2 <serial log of boot 2>
    local S fail=0
    S="$(tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    _f() { echo "  FAIL: boot 2: $1"; fail=1; }
    if grep -aqE 'KERNEL PANIC|\[pc\] (fault|exception)' <<< "$S"; then _f "the kernel panicked or faulted"; fi
    has '^rng sources: rdrand (yes|no)  rdseed (yes|no)  invariant-tsc (yes|no)  hypervisor no$' ||
        _f "QEMU was started with -hypervisor but the kernel does not say hypervisor no"
    has '^rng sources: acpi TCPA \(TPM 1\.2\) present, length 50 rev 2$' ||
        _f "the synthetic TCPA table (50 B, rev 2) is not reported present with those values"
    has '^rng sources: acpi TPM2 \(TPM 2\.0\) present, length 52 rev 4, start method 7 \(CRB\)$' ||
        _f "the synthetic TPM2 table (52 B, rev 4, start method 7) is not reported present with those values"
    has '^rng sources: tpm: off \(CRB, not supported yet: only the TIS interface is driven\)$' ||
        _f "the TPM2 table says CRB, but the TPM is not reported off for that reason"
    has '^rng sources: counted now: none, so fill_secret refuses on this PC; jitter is not counted yet \(ADR 0008 step 2b\)$' ||
        _f "no rdrand/rdseed and the TPM is off, but 'counted now' does not say fill_secret refuses"
    return "$fail"
}

mkdump() { # mkdump <n>: a dump as the kernel prints it (header, jd lines, end line), for the selftest
    "$PY" - "$1" <<'EOP'
import hashlib, struct, sys
n = int(sys.argv[1]); d = [(i * 2654435761) % 4093 + 40 for i in range(n)]
print("rng jitter dump: n=%d loop=mem4096-stride67 block=1024 tsc_hz=2992000000 hypervisor=yes (TSC ticks, hex, 16 per line)" % n)
print("rng jitter dump: a hypervisor is present, so these are the VM's timings, not the CPU's")
for i in range(0, n, 16):
    print("jd " + " ".join("%x" % v for v in d[i:i + 16]))
h = hashlib.sha256(b"".join(struct.pack("<Q", v) for v in d)).hexdigest()
print("rng jitter end: n=%d min=%x max=%x mean=%d distinct-low-bytes=%d sha256=%s"
      % (n, min(d), max(d), sum(d) // n, len({v & 255 for v in d}), h))
EOP
}

SRC1="[tpm] none (no TCPA or TPM2 table)
afrisovera> rng sources
rng sources: cpu GenuineIntel family 0x6 model 0x6 stepping 3
rng sources: rdrand no  rdseed no  invariant-tsc no  hypervisor yes
rng sources: acpi TCPA (TPM 1.2) absent
rng sources: acpi TPM2 (TPM 2.0) absent
rng sources: tpm: none (no TCPA or TPM2 table)
rng sources: counted now: none, so fill_secret refuses on this PC; jitter is not counted yet (ADR 0008 step 2b)"
TAIL1="afrisovera> rng jitter dump 0
rng jitter: type rng jitter dump <n>, n from 1 to 100000000 (e.g. rng jitter dump 1000000)
afrisovera> rng
[rng] HMAC_DRBG-SHA256 kat=PASS seeded=true strong_bytes=0 reseeds=1 secret_ok=0 secret_refused=0
[rng] source rdseed: absent samples=0
[rng] source rdrand: absent samples=0
[rng] source tpm: absent samples=0
[rng] counted bits=0 of 256 at the last (re)seed
[rng] source rdseed claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512
[rng] source rdrand claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512
[rng] source tpm claim: H=256/64 bit per byte, rct_cutoff=6 apt_cutoff=62/512"
G2="afrisovera> rng sources
rng sources: cpu GenuineIntel family 0x6 model 0x6 stepping 3
rng sources: rdrand no  rdseed no  invariant-tsc no  hypervisor no
rng sources: acpi TCPA (TPM 1.2) present, length 50 rev 2
rng sources: acpi TPM2 (TPM 2.0) present, length 52 rev 4, start method 7 (CRB)
rng sources: tpm: off (CRB, not supported yet: only the TIS interface is driven)
rng sources: counted now: none, so fill_secret refuses on this PC; jitter is not counted yet (ADR 0008 step 2b)"

selftest() {
    local pass=0 bad=0 G1 DUMP
    D1="$(mktemp -d "$T/pc-rngsources-selftest.XXXXXX")" || return 1
    DUMP="$(printf 'afrisovera> rng jitter dump %s\n' "$N"; mkdump "$N")"
    G1="$SRC1
$DUMP
$TAIL1"
    arm() { # arm <grader> <name> <expected> <text>
        local got
        printf '%s\n' "$4" > "$D1/in"
        if "$1" "$D1/in" > "$D1/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$3" ]; then pass=$((pass+1)); echo "    arm $2: $got (expected $3) OK"
        else bad=$((bad+1)); echo "    arm $2: $got but expected $3  <-- GRADER BUG"; sed 's/^/      /' "$D1/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm grade1 b1-green green "$G1"
    arm grade1 b1-crlf green "$(printf '%s\n' "$G1" | sed 's/$/\r/')"
    arm grade1 b1-rdrand-counted green "$(printf '%s\n' "$G1" | sed 's/rdrand no /rdrand yes /; s/counted now: none, so fill_secret refuses on this PC/counted now: rdseed\/rdrand/')"
    arm grade1 b1-rdrand-but-refuses red "$(printf '%s\n' "$G1" | sed 's/rdrand no /rdrand yes /')"
    arm grade1 b1-hypervisor-no red "$(printf '%s\n' "$G1" | sed 's/hypervisor yes$/hypervisor no/')"
    arm grade1 b1-tpm2-present red "$(printf '%s\n' "$G1" | sed 's/^rng sources: acpi TPM2 (TPM 2.0) absent$/rng sources: acpi TPM2 (TPM 2.0) present, length 52 rev 4, start method 7 (CRB)/')"
    arm grade1 b1-no-cpu red "$(printf '%s\n' "$G1" | grep -v '^rng sources: cpu')"
    arm grade1 b1-no-vm-warning red "$(printf '%s\n' "$G1" | grep -v 'VM.s timings')"
    arm grade1 b1-delta-changed red "$(printf '%s\n' "$G1" | sed '0,/^jd [0-9a-f]* /s//jd fff /')"
    arm grade1 b1-line-lost red "$(printf '%s\n' "$G1" | sed '0,/^jd /{/^jd /d}')"
    arm grade1 b1-no-end red "$(printf '%s\n' "$G1" | grep -v '^rng jitter end:')"
    arm grade1 b1-short-dump red "$(printf 'afrisovera> rng jitter dump 32\n'; printf '%s\n' "$SRC1"; mkdump 32 | sed "s/n=32 loop/n=$N loop/"; printf '%s\n' "$TAIL1")"
    arm grade1 b1-no-usage red "$(printf '%s\n' "$G1" | grep -v '^rng jitter: type')"
    arm grade1 b1-shared-rng-gone red "$(printf '%s\n' "$G1" | grep -v '^\[rng\] HMAC')"
    arm grade1 b1-claim-changed red "$(printf '%s\n' "$G1" | sed 's/^\(\[rng\] source rdrand claim: H=\)256\/64 bit per byte, rct_cutoff=6 apt_cutoff=62/\164\/64 bit per byte, rct_cutoff=21 apt_cutoff=311/')"
    arm grade1 b1-counted-bits-gone red "$(printf '%s\n' "$G1" | grep -v '^\[rng\] counted bits=')"
    arm grade1 b1-tpm-line-gone red "$(printf '%s\n' "$G1" | grep -v '^rng sources: tpm:')"
    arm grade1 b1-panic red "$(printf '%s\nKERNEL PANIC\n' "$G1")"
    arm grade1 b1-empty red ""
    arm grade2 b2-green green "$G2"
    arm grade2 b2-crlf green "$(printf '%s\n' "$G2" | sed 's/$/\r/')"
    arm grade2 b2-hypervisor-yes red "$(printf '%s\n' "$G2" | sed 's/hypervisor no$/hypervisor yes/')"
    arm grade2 b2-tcpa-absent red "$(printf '%s\n' "$G2" | sed 's/^rng sources: acpi TCPA .*/rng sources: acpi TCPA (TPM 1.2) absent/')"
    arm grade2 b2-tpm2-wrong-start red "$(printf '%s\n' "$G2" | sed 's/start method 7 (CRB)/start method 6 (memory-mapped FIFO)/')"
    arm grade2 b2-tpm2-wrong-length red "$(printf '%s\n' "$G2" | sed 's/length 52 rev 4/length 76 rev 4/')"
    arm grade2 b2-crb-counted red "$(printf '%s\n' "$G2" | sed 's/^rng sources: tpm: off .*/rng sources: tpm: TPM 2.0 over TIS at 0xfed40000: vendor 0x1014 device 0x0001, GetRandom ok (32 bytes in 90 us)/; s/counted now: none, so fill_secret refuses on this PC/counted now: tpm/')"
    arm grade2 b2-panic red "$(printf '%s\nKERNEL PANIC\n' "$G2")"
    arm grade2 b2-empty red ""
    rm -rf "$D1"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 28 ]
}

echo "== PC entropy sources: rng sources + rng jitter dump (ADR 0008 step 1) =="
if ! selftest; then echo "PC RNGSOURCES TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC RNGSOURCES SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC RNGSOURCES TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC RNGSOURCES TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE_4M.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC RNGSOURCES TEST FAILED (harness: no x86-64 UEFI firmware found)"; exit 1; }

W="$(mktemp -d "$T/pc-rngsources.XXXXXX")"
D1="$W"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
# the two synthetic tables' bodies (what follows QEMU's 36-byte header): TCPA client = platform class u16, LAML u32,
# LASA u64 (14 B); TPM2 = platform class u16, reserved u16, control area u64, start method u32 = 7 (CRB) (16 B)
"$PY" - "$W" <<'EOP'
import struct, sys
open(sys.argv[1] + "/tcpa.bin", "wb").write(struct.pack("<HIQ", 0, 0x10000, 0))
open(sys.argv[1] + "/tpm2.bin", "wb").write(struct.pack("<HHQI", 0, 0, 0, 7))
EOP

boot() { # boot <log> <cpu> [extra qemu args...] -- then the commands
    local log="$1" cpu="$2"; shift 2
    local extra=()
    while [ "$#" -gt 0 ] && [ "$1" != -- ]; do extra+=("$1"); shift; done
    shift
    "$REALQ" -M q35 -m 256M -display none -cpu "$cpu" "${extra[@]}" \
        -drive "if=pflash,format=raw,readonly=on,file=$(winpath "$OVMF")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
        -serial "tcp:127.0.0.1:5760,server,nowait" -qmp "tcp:127.0.0.1:5761,server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5760 5761 "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

echo "  --- boot 1: +hypervisor, no TPM tables: rng sources, rng jitter dump $N, usage, plain rng ---"
boot "$W/boot1.log" qemu64,+hypervisor -- "rng sources" "rng jitter dump $N" "rng jitter dump 0" "rng"
tr -d '\r' < "$W/boot1.log" | grep -aE '^rng (sources|jitter)|^\[rng\]' | sed 's/^/  | /'
echo "  --- boot 2: -hypervisor, synthetic TCPA + TPM2 tables: rng sources ---"
boot "$W/boot2.log" qemu64,-hypervisor \
    -acpitable "sig=TCPA,rev=2,oem_id=AFRI,oem_table_id=AFRITEST,data=$(winpath "$W/tcpa.bin")" \
    -acpitable "sig=TPM2,rev=4,oem_id=AFRI,oem_table_id=AFRITEST,data=$(winpath "$W/tpm2.bin")" \
    -- "rng sources"
tr -d '\r' < "$W/boot2.log" | grep -aE '^rng sources' | sed 's/^/  | /'

echo "  --- grading ---"
ok=0
grade1 "$W/boot1.log" || ok=1
grade2 "$W/boot2.log" || ok=1
if [ "$ok" -eq 0 ]; then
    "$PY" scripts/lib/jitter-log.py "$W/clean.log" | sed 's/^/  /'
    rm -rf "$W"
    echo "PC RNGSOURCES TEST PASSED"
    exit 0
fi
echo "  (serial logs kept: $W/boot1.log $W/boot2.log)"
echo "PC RNGSOURCES TEST FAILED"
exit 1
