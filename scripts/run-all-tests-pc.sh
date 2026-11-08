#!/usr/bin/env bash
# The regression registry against the PC kernel (PC track step 2, 4 Nov 2026).
#
# Builds the x86-64 kernel (mansa-pc.efi), puts scripts/lib/pc-arch first on PATH -- its
# `qemu-system-aarch64` turns every suite's ARM launch into a q35 + UEFI boot of the PC
# kernel (read that file) -- and runs scripts/run-all-tests.sh with its own results file and
# log dir, so a PC walk never overwrites an ARM one. Suites are not edited: the same grader
# judges the same commands, only the machine underneath changes.
# Every argument is passed on to run-all-tests.sh (e.g. --only "rot13,base64", --timeout 300).
#   scripts/run-all-tests-pc.sh --only "rot13,factor"
set -uo pipefail
cd "$(dirname "$0")/.."

echo "[run-all-pc] building the PC kernel"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
EFI="$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$EFI" ] || { echo "[run-all-pc] $EFI was not built" >&2; exit 2; }
command -v qemu-system-x86_64 > /dev/null || {
    for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
        [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ] && { PATH="$d:$PATH"; break; }
    done
}
command -v qemu-system-x86_64 > /dev/null || { echo "[run-all-pc] qemu-system-x86_64 not found" >&2; exit 2; }

export PC_EFI="$EFI"
# PC firmware can be silent for >9 s before the kernel starts; the suite driver must not
# take that silence for "booted" and type blind (lost first command, measured 4 Nov 2026)
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"; export PATH

# A full walk leaves out the suites in scripts/lib/pc-arch/arm-only.txt (they test ARM
# hardware a PC does not have; each line says which). --with-arm-only runs them anyway.
args=()
WITH_ARM=0
for a in "$@"; do [ "$a" = --with-arm-only ] && WITH_ARM=1 || args+=("$a"); done
case " $* " in
    *" --only "*) ;;
    *) if [ "$WITH_ARM" = 0 ]; then
           ARM_ONLY=$(grep -v '^#' scripts/lib/pc-arch/arm-only.txt | awk 'NF{print $1}')
           ALL=$(sed -n 's/^SUITES=(\(.*\))$/\1/p' scripts/run-all-tests.sh)
           KEEP=$(tr ' ' '\n' <<< "$ALL" | grep -vxF -f <(echo "$ARM_ONLY") | paste -sd, -)
           echo "[run-all-pc] $(wc -w <<< "$ARM_ONLY") ARM-hardware suites left out (scripts/lib/pc-arch/arm-only.txt)"
           args+=(--only "$KEEP")
       fi ;;
esac
case " $* " in *" --results "*) ;; *) args+=(--results .rat-pc-results.tsv) ;; esac
case " $* " in *" --logdir "*) ;; *) args+=(--logdir .rat-pc-logs) ;; esac
scripts/run-all-tests.sh "${args[@]}"
rc=$?
# A full PC walk leaves the ARM-hardware suites out on purpose, and the roll-up (rightly)
# counts them "not run" and exits 1. When every suite that WAS run passed and the only
# suites without a verdict are the ones arm-only.txt names, the PC walk is green: say so,
# with the count, and exit 0. Anything else keeps the runner's own exit code.
if [ "$rc" -ne 0 ] && [ -n "${KEEP:-}" ]; then
    RES=.rat-pc-results.tsv
    for ((i = 0; i < ${#args[@]}; i++)); do [ "${args[$i]}" = --results ] && RES="${args[$((i + 1))]}"; done
    want="$(tr ',' '\n' <<< "$KEEP" | grep -c .)"
    got="$(awk -F'\t' '$2 == "PASS"' "$RES" 2> /dev/null | cut -f1 | grep -cxF -f <(tr ',' '\n' <<< "$KEEP"))"
    if [ "$got" -eq "$want" ]; then
        echo "[run-all-pc] PC walk GREEN: all $want suites that apply to a PC passed; the $(wc -w <<< "$ARM_ONLY") ARM-hardware suites in scripts/lib/pc-arch/arm-only.txt were left out on purpose"
        exit 0
    fi
    echo "[run-all-pc] PC walk not green: $got of $want suites that apply to a PC passed"
fi
exit "$rc"
