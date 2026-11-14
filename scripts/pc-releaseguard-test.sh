#!/usr/bin/env bash
# Safe updates (AFR-0010 section 7, 14 Nov 2026): a build marked for release must trust no demo key. The PC kernel
# with `--features pc,release` refuses to COMPILE while the update key table holds a demo key or the app publisher
# key is still the demo one (pc/update.rs, const checks). This suite proves the check is armed:
#   1. `cargo check --features pc` (the control) passes, so a failure below is the guard and nothing else;
#   2. `cargo check --features pc,release` fails, naming BOTH demo keys (the update key by its flag AND by its
#      bytes), with no other error;
#   3. `cargo check --features pc,releaseguard-plant` (a test-only planted row: the demo update key's bytes marked
#      `demo: false`) fails naming the planted row, so the byte check catches what the flag check cannot.
# When the owner's real keys land, step 2's expectation flips to "passes" in the same change.
#   scripts/pc-releaseguard-test.sh              # grader selftest, then the three checks
#   scripts/pc-releaseguard-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

UPD='release build refused: it trusts the DEMO update key \(AFR-0010 section 7\)'
APP='release build refused: it trusts the DEMO app publisher key \(AFR-0010 section 7\)'
BYTES="release build refused: a key table row holds the DEMO update key's bytes \(AFR-0010 section 7\)"
PLANT="release build refused: the planted row \(demo: false\) holds the DEMO update key's bytes \(AFR-0010 section 7\)"


others() { # the errors in a log other than the guard's own refusals
    grep -aE '^error(\[E[0-9]+\])?:' "$1" | grep -avE '^error\[E0080\]|^error: could not compile' || true
}

grade() { # grade <control rc> <control log> <release rc> <release log> <plant rc> <plant log>
    local fail=0 other
    [ "$1" -eq 0 ] || { echo "  FAIL: the control build (--features pc) did not pass"; fail=1; }
    grep -aqE "release build refused" "$2" && { echo "  FAIL: the control build names a release refusal"; fail=1; }
    [ "$3" -ne 0 ] || { echo "  FAIL: the release build COMPILED while it trusts demo keys"; fail=1; }
    grep -aqE "$UPD" "$4" || { echo "  FAIL: the release build does not name the demo update key (flag)"; fail=1; }
    grep -aqE "$BYTES" "$4" || { echo "  FAIL: the release build does not name the demo update key's bytes"; fail=1; }
    grep -aqE "$APP" "$4" || { echo "  FAIL: the release build does not name the demo app publisher key"; fail=1; }
    grep -aqE "$PLANT" "$4" && { echo "  FAIL: the release build names the planted row (the test feature leaked)"; fail=1; }
    other="$(others "$4")"
    [ -z "$other" ] || { echo "  FAIL: the release build has another error: $(head -1 <<< "$other")"; fail=1; }
    [ "$5" -ne 0 ] || { echo "  FAIL: the planted-row build COMPILED"; fail=1; }
    grep -aqE "$PLANT" "$6" || { echo "  FAIL: the byte check did not catch the planted row (demo: false)"; fail=1; }
    other="$(others "$6")"
    [ -z "$other" ] || { echo "  FAIL: the planted-row build has another error: $(head -1 <<< "$other")"; fail=1; }
    return $fail
}

selftest() {
    local W pass=0 bad=0
    W="$(mktemp -d "$T/pc-releaseguard-st.XXXXXX")"
    printf '    Finished `dev` profile\n' > "$W/ok.log"
    {
        echo 'error[E0080]: evaluation panicked: release build refused: it trusts the DEMO update key (AFR-0010 section 7)'
        echo "error[E0080]: evaluation panicked: release build refused: a key table row holds the DEMO update key's bytes (AFR-0010 section 7)"
        echo 'error[E0080]: evaluation panicked: release build refused: it trusts the DEMO app publisher key (AFR-0010 section 7)'
        echo "error: could not compile \`mansa-kernel\` (bin \"mansa-pc\") due to 3 previous errors"
    } > "$W/rel.log"
    { cat "$W/rel.log"; echo "error[E0080]: evaluation panicked: release build refused: the planted row (demo: false) holds the DEMO update key's bytes (AFR-0010 section 7)"; } > "$W/plant.log"
    sed '/app publisher/d' "$W/rel.log" > "$W/noapp.log"
    sed '/bytes/d' "$W/rel.log" > "$W/nobytes.log"
    { cat "$W/rel.log"; echo 'error[E0425]: cannot find value `x` in this scope'; } > "$W/other.log"
    { cat "$W/plant.log"; echo 'error[E0425]: cannot find value `x` in this scope'; } > "$W/plantother.log"
    arm() { # arm <name> <expect green|red> <control rc> <log> <release rc> <log> <plant rc> <log>
        local got=red
        grade "$3" "$W/$4" "$5" "$W/$6" "$7" "$W/$8" > "$W/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$W/out"; fi
    }
    arm all-good        green 0 ok.log  101 rel.log     101 plant.log
    arm release-built   red   0 ok.log  0   ok.log      101 plant.log
    arm control-red     red   1 ok.log  101 rel.log     101 plant.log
    arm no-app-key      red   0 ok.log  101 noapp.log   101 plant.log
    arm no-bytes-check  red   0 ok.log  101 nobytes.log 101 plant.log
    arm other-error     red   0 ok.log  101 other.log   101 plant.log
    arm control-names   red   0 rel.log 101 rel.log     101 plant.log
    arm plant-built     red   0 ok.log  101 rel.log     0   ok.log
    arm plant-missed    red   0 ok.log  101 rel.log     101 rel.log
    arm plant-leaked    red   0 ok.log  101 plant.log   101 plant.log
    arm plant-other     red   0 ok.log  101 rel.log     101 plantother.log
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== PC safe updates: release guard against demo keys (AFR-0010 section 7) =="
if ! selftest; then echo "PC RELEASEGUARD TEST FAILED (grader selftest); no build spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC RELEASEGUARD SELFTEST ONLY; no measurement taken"; exit 0; }

W="$(mktemp -d "$T/pc-releaseguard.XXXXXX")"
# its own target folder, so the main build's artifacts are left as they are
TD=target/releaseguard
chk() { cargo check -p mansa-kernel --bin mansa-pc --features "$1" --target x86_64-unknown-uefi --target-dir "$TD" > "$W/$2.log" 2>&1; }
chk pc control; crc=$?
chk pc,release release; rrc=$?
chk pc,releaseguard-plant plant; prc=$?
echo "  control (pc): exit $crc; release (pc,release): exit $rrc; planted row (pc,releaseguard-plant): exit $prc"
grep -aE '^error' "$W/release.log" | sed 's/^/  release | /'
grep -aE 'planted row' "$W/plant.log" | sed 's/^/  plant | /'
if grade "$crc" "$W/control.log" "$rrc" "$W/release.log" "$prc" "$W/plant.log"; then
    rm -rf "$W"
    echo "PC RELEASEGUARD TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
tail -20 "$W/control.log" | sed 's/^/  control | /'
echo "PC RELEASEGUARD TEST FAILED"
exit 1
