#!/usr/bin/env bash
# Safe updates U4 (AFR-0010 section 8, 14 Nov 2026): `updatetry` starts a staged version ONCE as a trial; on trial the
# data is read only and the person is told; a healthy trial (self-tests, the window, the person's own input) keeps
# itself; a failed one goes back; `updateback` returns to the version before.
#
# Real kernels: this build (version 1) and, built here with compile-time test switches into their own target folders,
# version 2 (AFRI_RELEASE=2), version 2 that fails its self-tests (AFRI_TEST_FAIL=selftest) and version 2 that
# triple-faults right after its boot line (AFRI_TEST_FAIL=triplefault). The release guard refuses both switches in a
# release build (pc-releaseguard). Every boot runs edk2 with a writable variable store, the test rig's
# opt/afrisovera/trial-short (the 2-minute window becomes 5 seconds; never on a real PC) and -no-reboot (a restart
# ends the QEMU run; the next boot is the next run). Disks: the boot stick (IDE FAT folder, the firmware's first
# choice), a 16 MiB data disk (AFRIVFS1) on NVMe, the Windows-like disk on SATA, the update stick on USB.
# UPTRY_KIND=keep (the default):
#   Boots 1-4: bootcopy, bootentry, bootnext (as pc-updatestage)
#   Boot 5 (main): updatestage (version 2; its code)          Boot 6 (main): updatestage yes; updatetry (its code;
#                                                                            the "if version 2 fails" line)
#   Boot 7 (main): updatetry yes: BootNext = the trial entry, restart
#   Boot 8 (trial): "TRIAL of version 2"; a file written; NO input from the person; then `restart`: nothing saved
#   Boot 9 (stick): the file is not there; bootnext yes      Boot 10 (main): "ended without being kept"; updatetry
#   Boot 11 (main): updatetry yes                              Boot 12 (trial): a file written, the mouse moved: kept
#                   ("updated to version 2; updateback returns to version 1", "your changes ... are now saved"),
#                   whenever the polled mouse reaches it during the run
#   Boot 13 (stick): the file is there; no trial entry; the main entry names afrisovera-b.efi; bootnext (its code)
#   Boot 14 (stick): bootnext yes                              Boot 15 (main): release 2; updateback (its code);
#                                                                              bootnext yes
#   Boot 16 (main, version 2): updateback yes; bootnext (its code)   Boot 17 (stick): bootnext yes
#   Boot 18 (main): release 1 again
# UPTRY_KIND=nvme4k, recfirst, dual (scripts/pc-updatetry-<kind>-test.sh): the keep boots on the Windows-like disk of
# pc-bootnext's kinds (4 KiB-block NVMe, with the data disk then on SATA; a recovery partition first; one EFI partition
# shared with Ubuntu).
# UPTRY_KIND=more (scripts/pc-updatetry-more-test.sh), boots 1-7 as keep, then:
#   Boot 8 (trial): a file written, then the POWER CUT on the trial (QEMU killed 4 s after "TRIAL of version")
#   Boot 9 (stick): the file is not there; bootnext yes
#   Boot 10 (main): "ended without being kept"; updatesetting ask; updatetry (its code)
#   Boot 11 (main): updatetry yes        Boot 12 (trial, the mouse moved): NOT kept by itself (ask); updatekeep yes:
#                   kept, the file saved  Boot 13 (stick): the file is there, no trial entry; bootnext yes
#   Boot 14 (main): release 2; updatesetting auto again
# UPTRY_KIND=keepcut (scripts/pc-updatetry-keepcut-test.sh): version 2 built with AFRI_TEST_FAIL=keepcut, which restarts
# between the keep's two writes (the main entry rewritten, the trial entry not yet removed), boots 1-7 as keep, then:
#   Boot 8 (trial, the mouse moved): the keep's first write, then the restart
#   Boot 9 (stick): the main entry names afrisovera-b.efi and the trial entry is still there; bootnext (its code)
#   Boot 10 (stick): bootnext yes        Boot 11 (main, version 2): "a trial entry naming the running file was left by a
#                                         cut-off keep; taken out"; no trial entry
# UPTRY_KIND=fmt (scripts/pc-updatetry-fmt-test.sh), the data-format guard (the architect, 15 Nov 2026), boots 1-7 as
# keep, then:
#   Boot 8 (trial, the mouse moved): kept         Boot 9 (stick): bootnext (its code)     Boot 10 (stick): bootnext yes
#   The harness turns the data disk's first sector into AFRIVFS2 (VFS V3's new file system, format 3).
#   Boot 11 (main, version 2): updateback is REFUSED: "this disk now holds the new file system, which version 1
#           cannot read"; the main entry still names afrisovera-b.efi
# UPTRY_KIND=fail (scripts/pc-updatetry-fail-test.sh):
#   Boots 1-7 as above with the version 2 that fails its self-tests
#   Boot 8 (trial): the self-tests FAILED; not kept; BootNext = the main entry; restart
#   Boot 9 (main): "the trial of version 2 failed its self-tests; this is still version 1"; updatestage of the
#                  version 2 that triple-faults (its code)
#   Boot 10 (main): updatestage yes; updatetry (its code)      Boot 11 (main): updatetry yes
#   Boot 12 (trial): triple fault right after the boot line: QEMU ends
#   Boot 13 (stick, the firmware's next choice): bootnext yes  Boot 14 (main): "version 2 did not start; this is still
#                  version 1"; its trial entry taken out
# Every boot after 4 also checks the data disk changed only where a save is allowed (never on a trial before keep).
#   scripts/pc-updatetry-test.sh [--selftest]
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KIND="${UPTRY_KIND:-keep}"
FLOW=keep; ESPK=gpt-esp-fsinfo; FOURK=0
case "$KIND" in
    keep) UP=""; PORT=7200 ;;
    fail) UP=" FAIL"; PORT=7300; FLOW=fail ;;
    nvme4k) UP=" NVME4K"; PORT=7400; ESPK=gpt-esp-fsinfo-4k; FOURK=1 ;;
    recfirst) UP=" RECFIRST"; PORT=7500; ESPK=gpt-esp-recfirst ;;
    dual) UP=" DUAL"; PORT=7600; ESPK=gpt-esp-dual ;;
    more) UP=" ASK AND CUT"; PORT=7700; FLOW=more ;;
    keepcut) UP=" KEEPCUT"; PORT=7800; FLOW=keepcut ;;
    fmt) UP=" FORMAT GUARD"; PORT=7900; FLOW=fmt ;;
    *) echo "PC UPDATETRY TEST FAILED (harness: unknown UPTRY_KIND $KIND)"; exit 1 ;;
esac
NAME="PC UPDATETRY${UP}"

# --- the grader: one regex per boot that must be there (K: keep kind, F: fail kind), and lines that must NOT be ---
declare -A KW=(
    [6]='  if version 2 fails to start, this PC starts ".*" instead; choose AfriSovera in the firmware.s boot menu to come back'
    [7]='updatetry: BootNext = the trial entry, read back; restarting now'
    [8]='\[pc\] TRIAL of version 2: nothing you change is saved until this version is kept \(updatekeep\); a restart now goes back'
    [10]='\[pc\] updates: the trial of version 2 ended without being kept \(a restart or a power cut\); this is still version 1; what was done during it was not saved; updatetry tries it again'
    [12]='\[pc\] trial: updated to version 2; updateback returns to version 1'
    [13]='kept by the trial'
    [15]='\[pc\] updates: release 2; trusts the DEMO release key: a test build, not for release'
    [16]='updateback: AfriSovera.s boot entry now names \\EFI\\afrisovera\\afrisovera\.efi, read back; it takes effect at the next restart'
    [18]='\[pc\] updates: release 1; trusts the DEMO release key: a test build, not for release'
)
declare -A KNOT=(
    [9]='lost on trial'
    [12]='TRIAL .*FAILED'
)
declare -A FW=(
    [7]='updatetry: BootNext = the trial entry, read back; restarting now'
    [8]='\[pc\] TRIAL of version 2: the self-tests FAILED; not kept; going back to the main system'
    [9]='\[pc\] updates: the trial of version 2 failed its self-tests; this is still version 1 \(updatestage remove takes it out\)'
    [11]='updatetry: BootNext = the trial entry, read back; restarting now'
    [12]='\[pc\] TEST BUILD: AFRI_TEST_FAIL=triplefault, faulting now'
    [14]='\[pc\] updates: version 2 did not start; this is still version 1\. Its trial entry is taken out; the file stays staged \(updatetry tries again, updatestage remove takes it out\)'
)
STARTED='^  Boot[0-9A-F]{4}  active  "AfriSovera"  .*  <- started this time$'
TRIALSTART='^  Boot[0-9A-F]{4}  active  "AfriSovera trial"  .*  <- started this time$'
TRIALANY='^  Boot[0-9A-F]{4}  active  "AfriSovera trial"  '

grade_more() { # grade_more <dir>
    local d=$1 fail=0 k L
    for k in $(seq 1 14); do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        case $k in
            8) grep -aqE "$TRIALSTART" <<< "$L" || { echo "  FAIL: boot 8: not started by the trial entry"; fail=1; }
               grep -aqE '^\[pc\] trial: updated' <<< "$L" && { echo "  FAIL: boot 8: kept with no input"; fail=1; } ;;
            9) grep -aqx 'cut on trial' <<< "$L" && { echo "  FAIL: boot 9: a file written on the cut trial was saved"; fail=1; } ;;
            10) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot 10: not started by the main entry"; fail=1; }
                grep -aqE '^\[pc\] updates: the trial of version 2 ended without being kept \(a restart or a power cut\); this is still version 1' <<< "$L" || { echo "  FAIL: boot 10: the cut trial was not named"; fail=1; }
                grep -aqx 'updatesetting: now ask' <<< "$L" || { echo "  FAIL: boot 10: the setting was not written"; fail=1; } ;;
            11) grep -aqx 'updatetry: BootNext = the trial entry, read back; restarting now' <<< "$L" || { echo "  FAIL: boot 11: updatetry did not restart into the trial"; fail=1; } ;;
            12) grep -aqE "$TRIALSTART" <<< "$L" || { echo "  FAIL: boot 12: not started by the trial entry"; fail=1; }
                grep -aqE '^\[pc\] trial: updated' <<< "$L" && { echo "  FAIL: boot 12: kept by itself although the setting is ask"; fail=1; }
                grep -aqx 'updatekeep: updated to version 2; updateback returns to version 1' <<< "$L" || { echo "  FAIL: boot 12: updatekeep did not keep"; fail=1; }
                grep -aqx 'updatekeep: your changes since this start are now saved' <<< "$L" || { echo "  FAIL: boot 12: updatekeep did not save the changes"; fail=1; } ;;
            13) grep -aqx 'kept with ask' <<< "$L" || { echo "  FAIL: boot 13: the file written before updatekeep was not saved"; fail=1; }
                grep -aqE "$TRIALANY" <<< "$L" && { echo "  FAIL: boot 13: the trial entry is still there"; fail=1; } ;;
            14) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot 14: not started by the main entry"; fail=1; }
                grep -aqE '^\[pc\] updates: release 2; ' <<< "$L" || { echo "  FAIL: boot 14: not version 2"; fail=1; } ;;
        esac
    done
    grep -aqx 'harness: the data disk was never written on a trial before its keep' "$d/data.txt" 2> /dev/null || { echo "  FAIL: $(head -1 "$d/data.txt" 2> /dev/null)"; fail=1; }
    return $fail
}

grade_keepcut() { # grade_keepcut <dir>
    local d=$1 fail=0 k L
    for k in $(seq 1 11); do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        case $k in
            8) grep -aqE "$TRIALSTART" <<< "$L" || { echo "  FAIL: boot 8: not started by the trial entry"; fail=1; }
               # (it can follow the prompt on the same line: the keep runs from the shell's idle loop)
               grep -aqE '\[pc\] TEST BUILD: AFRI_TEST_FAIL=keepcut, restarting between the keep.s two writes$' <<< "$L" || { echo "  FAIL: boot 8: the keep never reached its cut"; fail=1; } ;;
            9) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  HD\(.*\)/\\EFI\\afrisovera\\afrisovera-b\.efi' <<< "$L" || { echo "  FAIL: boot 9: the main entry was not rewritten first"; fail=1; }
               grep -aqE "$TRIALANY" <<< "$L" || { echo "  FAIL: boot 9: no trial entry left (the cut must leave it)"; fail=1; } ;;
            11) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot 11: not started by the main entry"; fail=1; }
                grep -aqx '\[pc\] updates: a trial entry naming the running file was left by a cut-off keep; taken out' <<< "$L" || { echo "  FAIL: boot 11: the stale trial entry was not taken out"; fail=1; }
                grep -aqE "$TRIALANY" <<< "$L" && { echo "  FAIL: boot 11: the trial entry is still there"; fail=1; } ;;
        esac
    done
    return $fail
}

grade_fmt() { # grade_fmt <dir>
    local d=$1 fail=0 k L
    for k in $(seq 1 11); do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        case $k in
            8) grep -aqE '\[pc\] trial: updated to version 2; updateback returns to version 1$' <<< "$L" || { echo "  FAIL: boot 8: not kept"; fail=1; } ;;
            11) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot 11: not started by the main entry"; fail=1; }
                grep -aqE '^\[pc\] updates: release 2; ' <<< "$L" || { echo "  FAIL: boot 11: not version 2"; fail=1; }
                grep -aqx 'updateback: REFUSED: this disk now holds the new file system, which version 1 cannot read; nothing written' <<< "$L" || { echo "  FAIL: boot 11: updateback was not refused for the new file system"; fail=1; }
                grep -aqE '^updateback: nothing written yet\. To do it' <<< "$L" && { echo "  FAIL: boot 11: updateback offered its code"; fail=1; }
                grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  HD\(.*\)/\\EFI\\afrisovera\\afrisovera-b\.efi' <<< "$L" || { echo "  FAIL: boot 11: the main entry changed"; fail=1; } ;;
        esac
    done
    grep -aqx 'harness: the data disk now holds AFRIVFS2' "$d/fmt.txt" 2> /dev/null || { echo "  FAIL: the harness did not set the disk to AFRIVFS2"; fail=1; }
    return $fail
}

selftest_extra() { # the graders of the more and keepcut kinds
    local W pass=0 bad=0 k
    W="$(mktemp -d "$T/pc-updatetry-xst.XXXXXX")"
    local m='  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi  <- started this time'
    local t='  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi  <- started this time'
    good_more() {
        mkdir -p "$1"
        for k in $(seq 1 14); do
            {
                echo "afrisovera> (boot $k)"
                case $k in
                    8) echo "$t" ;;
                    10) echo "$m"; echo '[pc] updates: the trial of version 2 ended without being kept (a restart or a power cut); this is still version 1; what was done during it was not saved; updatetry tries it again'; echo 'updatesetting: now ask' ;;
                    11) echo 'updatetry: BootNext = the trial entry, read back; restarting now' ;;
                    12) echo "$t"; echo 'updatekeep: updated to version 2; updateback returns to version 1'; echo 'updatekeep: your changes since this start are now saved' ;;
                    13) echo 'kept with ask' ;;
                    14) echo "$m"; echo '[pc] updates: release 2; trusts the DEMO release key: a test build, not for release' ;;
                esac
            } > "$1/boot$k.log"
        done
        echo 'harness: the data disk was never written on a trial before its keep' > "$1/data.txt"
    }
    good_kc() {
        mkdir -p "$1"
        for k in $(seq 1 11); do
            {
                echo "afrisovera> (boot $k)"
                case $k in
                    8) echo "$t"; echo "[pc] TEST BUILD: AFRI_TEST_FAIL=keepcut, restarting between the keep's two writes" ;;
                    9) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi'; echo '  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi' ;;
                    11) echo "$m"; echo '[pc] updates: a trial entry naming the running file was left by a cut-off keep; taken out' ;;
                esac
            } > "$1/boot$k.log"
        done
    }
    xarm() { # xarm <name> <expect> <more|kc> <file> <sed>
        local d="$W/$1"
        if [ "$3" = more ]; then good_more "$d"; else good_kc "$d"; fi
        [ -n "$4" ] && sed -i "$5" "$d/$4"
        local got=red
        if [ "$3" = more ]; then grade_more "$d" > "$d/out" 2>&1 && got=green; else grade_keepcut "$d" > "$d/out" 2>&1 && got=green; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    xarm more-good green more "" ""
    xarm cut-saved red more boot9.log '$a cut on trial'
    xarm cut-not-named red more boot10.log '/ended without being kept/d'
    xarm ask-kept-itself red more boot12.log '$a [pc] trial: updated to version 2; updateback returns to version 1'
    xarm keep-not-saved red more boot13.log '/kept with ask/d'
    xarm kc-good green kc "" ""
    xarm kc-no-cut red kc boot8.log '/keepcut/d'
    xarm kc-trial-gone-early red kc boot9.log '/AfriSovera trial/d'
    xarm kc-not-cleaned red kc boot11.log '/cut-off keep/d'
    good_fmt() {
        mkdir -p "$1"
        for k in $(seq 1 11); do
            {
                echo "afrisovera> (boot $k)"
                case $k in
                    8) echo '[pc] trial: updated to version 2; updateback returns to version 1' ;;
                    11) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi  <- started this time'
                        echo '[pc] updates: release 2; trusts the DEMO release key: a test build, not for release'
                        echo 'updateback: REFUSED: this disk now holds the new file system, which version 1 cannot read; nothing written' ;;
                esac
            } > "$1/boot$k.log"
        done
        echo 'harness: the data disk now holds AFRIVFS2' > "$1/fmt.txt"
    }
    farm() { # farm <name> <expect> <file> <sed>
        local d="$W/$1"
        good_fmt "$d"
        [ -n "$3" ] && sed -i "$4" "$d/$3"
        local got=red
        grade_fmt "$d" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    farm fmt-good green "" ""
    farm fmt-not-refused red boot11.log 's/^updateback: REFUSED.*/updateback: nothing written yet. To do it, type exactly: updateback yes 1234/'
    farm fmt-not-kept red boot8.log '/updated to version 2/d'
    farm fmt-entry-changed red boot11.log 's/afrisovera-b\.efi/afrisovera.efi/'
    rm -rf "$W"
    echo "  grader selftest (more, keepcut, fmt): $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

grade() { # grade <dir> <kind>
    local d=$1 kind=$2 fail=0 k L n
    n=18; [ "$kind" = fail ] && n=14
    for k in $(seq 1 $n); do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        if [ "$kind" = keep ]; then
            [ -n "${KW[$k]:-}" ] && { grep -aqE "${KW[$k]}" <<< "$L" || { echo "  FAIL: boot $k: no line: ${KW[$k]}"; fail=1; }; }
            [ -n "${KNOT[$k]:-}" ] && grep -aqE "${KNOT[$k]}" <<< "$L" && { echo "  FAIL: boot $k: a line that must not be there: ${KNOT[$k]}"; fail=1; }
            case $k in
                # the boots that list the entries (6, 7 and 11 show it by updatetry going ahead: it refuses on any other start)
                5|10|15|16|18) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot $k: not started by AfriSovera's main entry"; fail=1; } ;;
                8|12) grep -aqE "$TRIALSTART" <<< "$L" || { echo "  FAIL: boot $k: not started by the trial entry"; fail=1; } ;;
            esac
            case $k in
                8) grep -aqE '^\[pc\] trial: updated' <<< "$L" && { echo "  FAIL: boot 8: kept with no input from the person"; fail=1; }
                   # VFS V2's flush confirms nothing on a trial: nothing reached the disk (the architect's rule)
                   grep -aqE '^\[blk\] flush: [a-z]+, failed' <<< "$L" || { echo "  FAIL: boot 8: blkflush did not answer failed on the trial"; fail=1; }
                   grep -aqE 'the data partition is read only until this version is kept; nothing is saved' <<< "$L" || { echo "  FAIL: boot 8: the restart did not say nothing is saved"; fail=1; } ;;
                12) grep -aqE '^\[pc\] trial: your changes since this start are now saved$' <<< "$L" || { echo "  FAIL: boot 12: the keep did not save the changes"; fail=1; } ;;
                13) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  HD\(.*\)/\\EFI\\afrisovera\\afrisovera-b\.efi' <<< "$L" || { echo "  FAIL: boot 13: the main entry does not name afrisovera-b.efi"; fail=1; }
                    grep -aqE "$TRIALANY" <<< "$L" && { echo "  FAIL: boot 13: the trial entry is still there after the keep"; fail=1; } ;;
            esac
        else
            [ -n "${FW[$k]:-}" ] && { grep -aqE "${FW[$k]}" <<< "$L" || { echo "  FAIL: boot $k: no line: ${FW[$k]}"; fail=1; }; }
            case $k in
                9|14) grep -aqE "$STARTED" <<< "$L" || { echo "  FAIL: boot $k: the old system did not start from its main entry"; fail=1; } ;;
                8|12) grep -aqE '^\[pc\] trial: updated' <<< "$L" && { echo "  FAIL: boot $k: a failing version was kept"; fail=1; } ;;
            esac
            [ "$k" = 14 ] && tail -n 30 <<< "$L" | grep -aqE "$TRIALANY" && { echo "  FAIL: boot 14: the trial entry is still there"; fail=1; }
        fi
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
    done
    grep -aqx 'harness: the data disk was never written on a trial before its keep' "$d/data.txt" 2> /dev/null || { echo "  FAIL: $(head -1 "$d/data.txt" 2> /dev/null)"; fail=1; }
    return $fail
}

selftest() {
    local W pass=0 bad=0 k kind
    W="$(mktemp -d "$T/pc-updatetry-st.XXXXXX")"
    good() { # good <dir> <kind>
        mkdir -p "$1"
        local m='  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi  <- started this time'
        local t='  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi  <- started this time'
        for k in $(seq 1 18); do
            {
                echo "afrisovera> (boot $k)"
                if [ "$2" = keep ]; then
                    case $k in 5|6|7|10|11|15|16|18) echo "$m" ;; 8|12) echo "$t" ;; esac
                    [ -n "${KW[$k]:-}" ] && sed -E 's/\\(.)/\1/g; s/\.\*/Windows Boot Manager/; s/\(\.\)/'"'"'/g' <<< "${KW[$k]}"
                    case $k in
                        8) echo '[pc] trial: the data partition is read only until this version is kept; nothing is saved'
                           echo '[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)' ;;
                        12) echo '[pc] trial: your changes since this start are now saved' ;;
                        13) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi' ;;
                    esac
                else
                    case $k in 9|14) echo "$m" ;; esac
                    [ -n "${FW[$k]:-}" ] && sed -E 's/\\(.)/\1/g' <<< "${FW[$k]}"
                fi
            } > "$1/boot$k.log"
        done
        echo 'harness: the data disk was never written on a trial before its keep' > "$1/data.txt"
    }
    arm() { # arm <name> <expect> <kind> <file> <sed>
        local d="$W/$1"
        good "$d" "$3"
        [ -n "$4" ] && sed -i "$5" "$d/$4"
        local got=red
        grade "$d" "$3" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    arm keep-good green keep "" ""
    arm fail-good green fail "" ""
    arm kept-without-input red keep boot8.log '$a [pc] trial: updated to version 2; updateback returns to version 1'
    arm lost-file-saved red keep boot9.log '$a lost on trial'
    arm not-saved-at-keep red keep boot12.log '/are now saved/d'
    arm main-not-rewritten red keep boot13.log 's/afrisovera-b\.efi$/afrisovera.efi/'
    arm trial-left red keep boot13.log '$a \  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi'
    arm no-v2 red keep boot15.log '/release 2/d'
    arm back-missing red keep boot18.log '/release 1/d'
    arm no-warning red keep boot6.log '/fails to start/d'
    arm flush-ok-on-trial red keep boot8.log 's/flush: nvme, failed.*/flush: nvme, ok/'
    arm trial-not-from-entry red keep boot12.log '/AfriSovera trial.*started this time/d'
    arm fail-kept red fail boot8.log '$a [pc] trial: updated to version 2; updateback returns to version 1'
    arm fail-not-named red fail boot9.log '/failed its self-tests/d'
    arm crash-not-named red fail boot14.log '/did not start/d'
    arm crash-trial-left red fail boot14.log '$a \  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\\EFI\\afrisovera\\afrisovera-b.efi'
    arm data-written red keep data.txt 's/.*/harness: the data disk CHANGED on a trial (boot 8)/'
    arm panic red fail boot12.log '$a KERNEL PANIC'
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== PC safe updates U4: updatetry, the trial, keep and updateback (kind $KIND) =="
if ! selftest || ! selftest_extra; then echo "$NAME TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "$NAME SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "$NAME TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build: this kernel (version 1) and the test versions ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "$NAME TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
variant() { # variant <name> <AFRI_TEST_FAIL or empty>: version 2, into its own target folder
    local td="target/updatetry-$1"
    if [ -n "$2" ]; then
        AFRI_RELEASE=2 AFRI_TEST_FAIL="$2" cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi --target-dir "$td" 2>&1 | tail -1
    else
        AFRI_RELEASE=2 cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi --target-dir "$td" 2>&1 | tail -1
    fi
    [ -r "$td/x86_64-unknown-uefi/debug/mansa-pc.efi" ] || { echo "$NAME TEST FAILED (harness: the $1 kernel was not built)"; exit 1; }
    printf '%s' "$PWD/$td/x86_64-unknown-uefi/debug/mansa-pc.efi"
}
if [ "$FLOW" = keep ] || [ "$FLOW" = more ] || [ "$FLOW" = fmt ]; then
    V2="$(variant v2 '' | tail -1)"
elif [ "$FLOW" = keepcut ]; then
    V2="$(variant v2kc keepcut | tail -1)"
else
    V2="$(variant v2fail selftest | tail -1)"
    VTF="$(variant v2tf triplefault | tail -1)"
fi
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-60}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-120}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF="${PC_OVMF:-}"; VARS="${PC_OVMF_VARS:-}"
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" /usr/share/qemu/edk2-x86_64-code.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "$(dirname "$f")/edk2-i386-vars.fd" ]; then OVMF="$f"; VARS="$(dirname "$f")/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "$NAME TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-updatetry.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py "$ESPK" "$W/win.img" || { echo "$NAME TEST FAILED (harness: could not make win.img)"; exit 1; }
"$PY" -c '
import sys
d = bytearray(16 * 1024 * 1024)
d[0:8] = b"AFRIVFS1"  # an empty AfriSovera data disk: the superblock, no files
open(sys.argv[1], "wb").write(d)
' "$W/data.img"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
mk_stick() { # mk_stick <kernel> <out>
    "$PY" scripts/make-update.py file "$1" "$W/u.afu" --version 2 && "$PY" scripts/make-update.py stick "$2" "$W/u.afu"
}
mk_stick "$V2" "$W/st2.img" || { echo "$NAME TEST FAILED (harness: could not make the update stick)"; exit 1; }
[ "$KIND" = fail ] && { mk_stick "$VTF" "$W/sttf.img" || { echo "$NAME TEST FAILED (harness: could not make the triple-fault stick)"; exit 1; }; }

# the data disk on NVMe and the Windows-like disk on SATA port 2; the other way round, in 4 KiB blocks, for nvme4k
DEV=(-device nvme,serial=afri01,drive=sd,bootindex=1 -device ide-hd,drive=sb,bus=ide.2,bootindex=2)
[ "$FOURK" = 1 ] && DEV=(-device ide-hd,drive=sd,bus=ide.2,bootindex=1
    -device nvme,serial=afri02,drive=sb,logical_block_size=4096,physical_block_size=4096,bootindex=2)
: > "$W/data.bad"
boot() { # boot <n> <update stick> <commands> [mouse] [cut-after-line]: one QEMU run; a guest restart ends it
    local n=$1 st=$2 cmds=$3 mouse=${4:-} cutline=${5:-}
    local sp=$((PORT + 4 * n)) log="$W/boot$n.log"
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    cp "$W/$st" "$W/ust.img"
    cp "$W/data.img" "$W/data.before"
    : > "$W/serial.log"
    "$REALQ" -M q35 -m 256M -display none -no-reboot \
        -fw_cfg name=opt/afrisovera/trial-short,string=1 \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "if=none,format=raw,id=stk,file=fat:rw:$(winpath "$W/stick")" -device ide-hd,drive=stk,bus=ide.0,bootindex=0 \
        -drive "file=$(winpath "$W/data.img"),if=none,format=raw,id=sd" \
        -drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=sb" "${DEV[@]}" \
        -device qemu-xhci,id=xhci -drive "file=$(winpath "$W/ust.img"),if=none,format=raw,id=ust" -device usb-storage,drive=ust,bus=xhci.0 \
        -chardev "socket,id=ser,host=127.0.0.1,port=$sp,server=on,wait=off,logfile=$(winpath "$W/serial.log")" -serial chardev:ser \
        -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    if [ -n "$mouse" ]; then # the person moves the mouse once the line `$mouse` is on the serial line (QMP: the PS/2
        # mouse); "mouse" alone means once the trial has said so. A test that writes a file first waits for its save
        # line, so the keep (input + the window) can never come before the file exists.
        local pat=$mouse
        [ "$pat" = mouse ] && pat='TRIAL of version'
        ( for _ in $(seq 1 1200); do
              tr -d '\r' < "$W/serial.log" 2> /dev/null | grep -aqF "$pat" && break
              sleep 0.1
          done
          sleep 1
          "$PY" -c '
import json, socket, sys
q = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=10)
f = q.makefile("rb")
f.readline()
q.sendall(b"{\"execute\":\"qmp_capabilities\"}\n"); f.readline()
ev = [{"type": "rel", "data": {"axis": "x", "value": 40}}, {"type": "rel", "data": {"axis": "y", "value": 30}}]
q.sendall(json.dumps({"execute": "input-send-event", "arguments": {"events": ev}}).encode() + b"\n")
print("harness: the mouse moved on the trial", f.readline().decode().strip())
' $((sp+1)) >> "$W/mouse.txt" 2>&1 ) &
    fi
    if [ -n "$cutline" ]; then # the power cut: QEMU killed 4 s after the line (no flush, no shutdown)
        ( for _ in $(seq 1 1200); do
              tr -d '\r' < "$W/serial.log" 2> /dev/null | grep -aq "$cutline" && break
              sleep 0.1
          done
          sleep 4
          kill -9 "$q" 2> /dev/null && echo "harness: QEMU killed 4 s after \"$cutline\" (boot $n)" >> "$W/cut.txt" ) &
    fi
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    sleep 1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    wait 2> /dev/null
    tr -d '\r' < "$W/serial.log" >> "$log" # the whole serial line, including a restart's last words
    printf '\n' >> "$log"
}
code() { tr -d '\r' < "$1" | sed -nE "s/^$2: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\1/p" | head -1; }
data_same() { # data_same <boot>: on a trial before its keep the data disk must not change
    cmp -s "$W/data.img" "$W/data.before" || echo "harness: the data disk CHANGED on a trial (boot $1)" >> "$W/data.bad"
}

boot 1 st2.img 'bootcopy'
C1="$(code "$W/boot1.log" bootcopy)"
boot 2 st2.img "bootcopy yes ${C1:-none}|bootentry"
C2="$(code "$W/boot2.log" bootentry)"
boot 3 st2.img "bootentry yes ${C2:-none}|bootnext"
C3="$(code "$W/boot3.log" bootnext)"
boot 4 st2.img "bootnext yes ${C3:-none}"
boot 5 st2.img "bootentries|updatestage|bootnext yes ${C3:-none}"
C5="$(code "$W/boot5.log" updatestage)"
boot 6 st2.img "updatestage yes ${C5:-none}|updatetry|bootnext yes ${C3:-none}"
C6="$(code "$W/boot6.log" updatetry)"
echo "  --- boot 7 (main): updatetry yes ${C6:-(no code)} ---"
boot 7 st2.img "updatetry yes ${C6:-none}"
if [ "$FLOW" = keep ]; then
    echo "  --- boot 8 (trial): no input from the person, a file written, restart ---"
    boot 8 st2.img "bootentries|echo lost on trial > lost.txt|blkflush|sleep 7|restart"
    data_same 8
    boot 9 st2.img "cat lost.txt|bootnext yes ${C3:-none}"
    boot 10 st2.img "bootentries|updatetry|bootnext yes ${C3:-none}"
    C10="$(code "$W/boot10.log" updatetry)"
    boot 11 st2.img "updatetry yes ${C10:-none}"
    echo "  --- boot 12 (trial): a file written, the mouse moved: kept ---"
    boot 12 st2.img "bootentries|echo kept by the trial > kept.txt|sleep 7|sleep 5|bootentries" "saved to kept.txt"
    echo "  --- boot 13 (stick): the kept file, the entries, bootnext (its code) ---"
    boot 13 st2.img "cat kept.txt|bootentries|bootnext"
    C13="$(code "$W/boot13.log" bootnext)"
    boot 14 st2.img "bootnext yes ${C13:-none}"
    boot 15 st2.img "bootentries|updateback|bootnext yes ${C13:-none}"
    C15="$(code "$W/boot15.log" updateback)"
    boot 16 st2.img "bootentries|updateback yes ${C15:-none}|bootnext"
    C16="$(code "$W/boot16.log" bootnext)"
    boot 17 st2.img "bootnext yes ${C16:-none}"
    boot 18 st2.img 'bootentries'
elif [ "$FLOW" = more ]; then
    echo "  --- boot 8 (trial): a file written, then the power cut ---"
    boot 8 st2.img "bootentries|echo cut on trial > cut.txt|sleep 30|bootentries" "" "TRIAL of version"
    data_same 8
    boot 9 st2.img "cat cut.txt|bootnext yes ${C3:-none}"
    boot 10 st2.img "bootentries|updatesetting ask|updatetry|bootnext yes ${C3:-none}"
    C10="$(code "$W/boot10.log" updatetry)"
    boot 11 st2.img "updatetry yes ${C10:-none}"
    echo "  --- boot 12 (trial, setting ask): not kept by itself; updatekeep keeps ---"
    boot 12 st2.img "bootentries|echo kept with ask > ask.txt|sleep 7|sleep 7|bootentries|updatekeep yes 4b02|bootnext" "saved to ask.txt"
    C12="$(code "$W/boot12.log" bootnext)"
    boot 13 st2.img "cat ask.txt|bootentries|bootnext yes ${C12:-none}"
    boot 14 st2.img "bootentries|updatesetting auto"
elif [ "$FLOW" = fmt ]; then
    echo "  --- boot 8 (trial, the mouse moved): kept ---"
    boot 8 st2.img "bootentries|sleep 7|sleep 5|bootentries" mouse
    boot 9 st2.img "bootentries|bootnext"
    C9="$(code "$W/boot9.log" bootnext)"
    boot 10 st2.img "bootnext yes ${C9:-none}"
    "$PY" -c '
import sys
p = sys.argv[1]
d = bytearray(open(p, "rb").read())
d[0:8] = b"AFRIVFS2"  # VFS V3: a file system no version before V3 can read
open(p, "wb").write(d)
print("harness: the data disk now holds AFRIVFS2")
' "$W/data.img" > "$W/fmt.txt" 2>&1
    sed 's/^/  | /' "$W/fmt.txt"
    echo "  --- boot 11 (main, version 2): updateback refused for the new file system ---"
    boot 11 st2.img "bootentries|updateback"
elif [ "$FLOW" = keepcut ]; then
    echo "  --- boot 8 (trial, the mouse moved): the keep restarts between its two writes ---"
    boot 8 st2.img "bootentries|sleep 7|sleep 5|bootentries" mouse
    boot 9 st2.img "bootentries|bootnext"
    C9="$(code "$W/boot9.log" bootnext)"
    boot 10 st2.img "bootnext yes ${C9:-none}"
    echo "  --- boot 11 (main, version 2): the stale trial entry taken out ---"
    boot 11 st2.img 'bootentries'
else
    echo "  --- boot 8 (trial): the version that fails its self-tests ---"
    boot 8 st2.img 'bootentries'
    data_same 8
    boot 9 sttf.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    C9="$(code "$W/boot9.log" updatestage)"
    boot 10 sttf.img "updatestage yes ${C9:-none}|updatetry|bootnext yes ${C3:-none}"
    C10="$(code "$W/boot10.log" updatetry)"
    boot 11 sttf.img "updatetry yes ${C10:-none}"
    echo "  --- boot 12 (trial): the version that triple-faults ---"
    boot 12 sttf.img 'bootentries'
    data_same 12
    boot 13 sttf.img "bootnext yes ${C3:-none}"
    boot 14 sttf.img 'bootentries'
fi
if [ -s "$W/data.bad" ]; then cp "$W/data.bad" "$W/data.txt"; else echo 'harness: the data disk was never written on a trial before its keep' > "$W/data.txt"; fi
cat "$W/mouse.txt" "$W/cut.txt" 2> /dev/null | sed 's/^/  | /'
cat "$W"/boot*.log | tr -d '\r' | grep -aE '^afrisovera> |^update|^\[pc\] (TRIAL|trial|updates|TEST)|^  Boot[0-9A-F]{4} |^  if version' | sed 's/^/  | /'

echo "  --- grading ---"
case "$FLOW" in
    more) G=grade_more ;;
    keepcut) G=grade_keepcut ;;
    fmt) G=grade_fmt ;;
    *) G=grade ;;
esac
if $G "$W" "$FLOW"; then
    rm -rf "$W"
    echo "$NAME TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "$NAME TEST FAILED"
exit 1
