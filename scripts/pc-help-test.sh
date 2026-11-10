#!/usr/bin/env bash
# PC track (9 Nov 2026, HP ENVY field item F7, second half): `help` split by word and by page.
#
# On the laptop the shared `help` (over 600 lines) ran off the top of the screen. On the PC, plain
# `help` still prints the whole list and now ends with one line saying how many commands there are and
# how to see part of them; `help <word>` shows only the lines with that word (any case) and `help <n>`
# shows page n of 36 lines (kernel/src/pc/helpview.rs filters the shared list, so the ARM image is the
# same). One QEMU boot types: help, help sha, help 2, help 999, help zzqq. Graded against the full list
# from the same boot: the full list arrives whole (its count agrees with the summary, P = ceil(N/36));
# `help sha` shows exactly the full list's lines holding "sha", in order, and says how many; `help 2`
# shows exactly lines 37-72 and points at page 3; `help 999` and `help zzqq` show nothing but the title
# and say why.
#   scripts/pc-help-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-help-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

read -r -d '' GRADER <<'EOF'
import re, sys
raw = open(sys.argv[1], encoding="utf-8", errors="replace", newline="").read()
S = [re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", l) for l in raw.replace("\r", "").split("\n")]
fail = []
if any("KERNEL PANIC" in l or re.match(r"^\[pc\] (EXCEPTION|PANIC)", l) for l in S):
    fail.append("the kernel panicked")
blocks = {}
cur = None
for l in S:
    m = re.match(r"^afrisovera> (.*)$", l)
    if m:
        cur = m.group(1).strip()
        blocks[cur] = []
    elif cur is not None:
        blocks[cur].append(l)
def summary(b):
    s = [l for l in b if l.startswith("help: ")]
    return s[-1] if s else None
def body(b):  # the lines before the summary, the title first
    out = []
    for l in b:
        if l.startswith("help: "):
            break
        out.append(l)
    while out and out[-1] == "":
        out.pop()
    return out
full = blocks.get("help")
N = P = 0
lst = []
if full is None:
    fail.append("plain help was not typed")
else:
    b = body(full)
    sm = summary(full)
    m = re.match(r"^help: (\d+) commands above \(Shift\+PgUp scrolls back\); help <word> shows only the lines with that word, help 1 to help (\d+) a page at a time$", sm or "")
    if not b or b[0] != "commands:":
        fail.append("plain help did not start with 'commands:'")
    if "  help            this list" not in b:
        fail.append("plain help is missing its own line")
    if not m:
        fail.append("plain help did not end with the summary line (got %r)" % sm)
    else:
        N, P = int(m.group(1)), int(m.group(2))
        lst = b[1:]
        if len(lst) != N:
            fail.append("plain help printed %d lines but its summary says %d" % (len(lst), N))
        if N < 300:
            fail.append("plain help has only %d lines (the shared list is over 600)" % N)
        if P != (N + 35) // 36:
            fail.append("%d lines should be %d pages of 36, the summary says %d" % (N, (N + 35) // 36, P))
if lst:
    w = blocks.get("help sha")
    want = [l for l in lst if "sha" in l.lower()]
    if w is None:
        fail.append("help sha was not typed")
    else:
        b, sm = body(w), summary(w)
        if b[:1] != ["commands:"]:
            fail.append("help sha did not start with 'commands:'")
        if b[1:] != want:
            fail.append("help sha showed %d lines, not the %d lines of the full list holding 'sha'" % (len(b) - 1, len(want)))
        if not want:
            fail.append("the full list has no line with 'sha' (sha256 was expected)")
        if sm != "help: %d of %d lines have 'sha'; help 1 to help %d shows them all a page at a time" % (len(want), N, P):
            fail.append("help sha summary wrong (got %r)" % sm)
    p = blocks.get("help 2")
    if p is None:
        fail.append("help 2 was not typed")
    else:
        b, sm = body(p), summary(p)
        if b != ["commands:"] + lst[36:72]:
            fail.append("help 2 did not show exactly lines 37-72 of the full list (%d lines shown)" % (len(b) - 1))
        if sm != "help: page 2 of %d (lines 37-72 of %d); help 3 for the next" % (P, N):
            fail.append("help 2 summary wrong (got %r)" % sm)
    x = blocks.get("help 999")
    if x is None:
        fail.append("help 999 was not typed")
    elif body(x) != ["commands:"] or summary(x) != "help: there is no page 999; the %d lines are pages 1 to %d" % (N, P):
        fail.append("help 999 did not say there is no such page (got %r)" % summary(x))
    z = blocks.get("help zzqq")
    if z is None:
        fail.append("help zzqq was not typed")
    elif body(z) != ["commands:"] or summary(z) != "help: no line of the %d has 'zzqq'; help 1 to help %d shows them a page at a time" % (N, P):
        fail.append("help zzqq did not say no line has it (got %r)" % summary(z))
for f in fail:
    print("  FAIL: " + f)
if not fail:
    print("  help: %d lines, %d pages; 'sha' %d lines; page 2 and the misses as expected" % (N, P, len([l for l in lst if 'sha' in l.lower()])))
sys.exit(1 if fail else 0)
EOF

grade() { [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }; "$PY" -c "$GRADER" "$1"; }

# a synthetic boot: an 80-line list (3 pages), 2 lines with "sha"
mklist() { echo "  help            this list"; echo "  sha256 <text>   hash it"; for i in $(seq 3 79); do echo "  cmd$i           thing $i"; done; echo "  SHAsum          upper case"; }
L="$(mklist)"
G="[pc] MANSA-PC BOOT OK
afrisovera> help
commands:
$L
help: 80 commands above (Shift+PgUp scrolls back); help <word> shows only the lines with that word, help 1 to help 3 a page at a time
afrisovera> help sha
commands:
$(printf '%s\n' "$L" | grep -i sha)
help: 2 of 80 lines have 'sha'; help 1 to help 3 shows them all a page at a time
afrisovera> help 2
commands:
$(printf '%s\n' "$L" | sed -n '37,72p')
help: page 2 of 3 (lines 37-72 of 80); help 3 for the next
afrisovera> help 999
commands:
help: there is no page 999; the 80 lines are pages 1 to 3
afrisovera> help zzqq
commands:
help: no line of the 80 has 'zzqq'; help 1 to help 3 shows them a page at a time
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-help-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <capture text>
        local got
        printf '%s\n' "$3" > "$D/c"
        if grade "$D/c" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    # the grader's own minimum is 300 lines, so the synthetic list is graded with that check relaxed
    local GR="$GRADER"
    GRADER="${GRADER//N < 300/N < 30}"
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$G"
    arm crlf green "$(printf '%s\n' "$G" | sed 's/$/\r/')"
    arm the-8nov-output red "$(printf '%s\n' "$G" | grep -v '^help: ')"
    arm full-list-short red "$(printf '%s\n' "$G" | sed '0,/cmd20 /{/cmd20 /d}')"
    arm wrong-page-count red "$(printf '%s\n' "$G" | sed 's/help 1 to help 3 a page/help 1 to help 4 a page/')"
    arm word-extra-line red "$(printf '%s\n' "$G" | sed '/^afrisovera> help sha$/,/^help: 2 of/{s/^  SHAsum .*/&\n  cmd5           thing 5/}')"
    arm word-missed-line red "$(printf '%s\n' "$G" | sed '/^afrisovera> help sha$/,/^help: 2 of/{/SHAsum/d}')"
    arm word-bad-count red "$(printf '%s\n' "$G" | sed "s/^help: 2 of 80 lines have/help: 3 of 80 lines have/")"
    arm page-wrong-lines red "$(printf '%s\n' "$G" | sed '/^afrisovera> help 2$/,/^help: page/{/cmd38 /d}')"
    arm page-no-next red "$(printf '%s\n' "$G" | sed 's/; help 3 for the next$//')"
    arm miss-shows-lines red "$(printf '%s\n' "$G" | sed '/^afrisovera> help zzqq$/{n;s/^commands:$/commands:\n  cmd3           thing 3/}')"
    arm nopage-unsaid red "$(printf '%s\n' "$G" | grep -v '^help: there is no page')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$G")"
    arm empty red ""
    GRADER="$GR"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC help split by word and by page (HP ENVY field item F7) =="
if ! selftest; then echo "PC HELP TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC HELP SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC HELP TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC HELP TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && OVMF="$d/edk2-x86_64-code.fd"
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd; do
    [ -z "$OVMF" ] && [ -r "$f" ] && OVMF="$f"
done
[ -n "$OVMF" ] || { echo "PC HELP TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-help.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
SP=5472
echo "  --- boot: help, help sha, help 2, help 999, help zzqq ---"
"$REALQ" -M q35 -m 256M -display none \
    -drive "if=pflash,format=raw,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:$SP,server,nowait" -qmp "tcp:127.0.0.1:$((SP+1)),server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$SP" "$((SP+1))" "$W/shot.png" \
    "help" "help sha" "help 2" "help 999" "help zzqq" > "$W/serial.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^afrisovera> help|^help: ' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC HELP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC HELP TEST FAILED"
exit 1
