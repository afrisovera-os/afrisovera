#!/usr/bin/env bash
# PC track step 3 (13 Nov 2026): `battery`, the battery, AC adapter, lid and embedded controller the firmware
# describes, READ ONLY. The first step towards an AML interpreter: the DSDT and SSDTs are walked by the ACPI grammar
# (Scope, Device, Name, Method, OperationRegion, Field ...), each device gets its full path, its _HID and its
# methods. Nothing is run. `battery selftest` runs nine small checks on any machine: eight AML blocks built in the
# kernel (nested names, an unknown form, an If block, a table cut short, status values, controller ports and
# fields) and a pretend controller's reads.
# Second slice (13 Nov 2026): each device's status (_STA) is worked out when the tables give it as a constant,
# `Return(constant)` or `Return(NAME)` of a constant Name; the rest is counted as needing the interpreter.
# The status line must agree across the boots: boot 2 has exactly 3 more worked out and the same number needing
# the interpreter as boot 1.
# Third slice (13 Nov 2026): the embedded controller's map: its data and command ports from its _CRS, and the named
# units of every Field over its EmbeddedControl region (byte, bit, width), one Field in the device and one in a
# later Scope(\_SB_.EC0_). Boot 2 must list exactly those; boot 1 has no controller lines at all.
# Fourth slice (13 Nov 2026): `battery ec` reads those fields through the controller's ports (read command 0x80
# only, every wait time-limited). QEMU has no embedded controller, so boot 2 must say nothing answers at 0x66 (or
# that it did not answer in time) and print no value; boot 1 must say none is described. The read handshake itself
# is checked by selftest case 9 against a pretend controller (good reads, nothing there, stuck busy, a byte left
# for another reader).
# Fifth slice (13 Nov 2026): `battery ec` also runs each AC adapter's _PSR with a small read-only evaluator (Return,
# If, Else, And, Or, LAnd, LEqual, LNot and the like, constant Names, embedded controller fields). The test SSDT's
# ADP1 reads ^^EC0_.ACIN, which QEMU cannot answer, so boot 2 must say why it was not worked out; ADP2 tests a
# constant Name (PSRC) and must say "on mains power". Selftest case 10 runs seven _PSR bodies against a pretend
# reader. Boot 1 has no adapter lines.
# Sixth slice (13 Nov 2026): Store and Local0 to Local7 (And, Or, XOr may also put their result in a Local; a store
# into anything else stops). ADP2's _PSR now stores PSRC in Local0, copies it to Debug and returns Local0, so boot 2
# says "on mains power" only when Store and the Locals work. Selftest case 10 now runs ten _PSR bodies (a store
# into a field must stop with nothing read, an unset Local must stop, a field read once is used twice).
# Seventh slice (13 Nov 2026): Packages of integers, Store into Index(LocalN, i) and DerefOf(Index(LocalN, i)), so
# `battery ec` runs each battery's _BST read only. The test SSDT's BAT0 _BST builds a Package in Local0, sets the
# state and the remaining capacity (from a constant Name) through Index and returns it, so boot 2 must say
# "state 0x2 (charging), rate unknown, remaining 3000, voltage 12000 mV". Selftest case 11 runs eight _BST bodies
# (an Index past the end, of a named Package, of an unset Local, a short Package, a Package used as a number).
# Eighth slice (13 Nov 2026): named Packages, as most firmware writes its _BST. Each run gets its own copy of a
# Name(X, Package) of numbers; the namespace is never changed. The test SSDT's BAT0 now keeps its _BST result in
# Name(PBST, Package(4) {...}) beside the method, stores the state and BSTR through Index(PBST, i) and returns PBST,
# so boot 2's line above is printed only when the named copy works. Selftest case 11 now runs twelve _BST bodies
# (a named copy changed and returned, the tables' values unchanged on the next run, a full path and a short name
# reaching the same copy, an Index of an Arg, a Package holding a string not kept).
# Ninth slice (13 Nov 2026): strings, so each battery's _BIF or _BIX is read (units, design and last full capacity,
# design voltage, model, serial, type). The test SSDT's BAT0 _BIF stores BFCC (4500) into Index(PBIF, 2) of a named
# Package holding four strings and returns it, so boot 2 must give the _BST line in mAh, an info line and
# "charge: 66% of the last full charge". Selftest case 12 runs six batteries' _BIF/_BIX (a method changing its
# named copy, a Name(_BIX, Package), _BIX before _BIF, strings in a run's Package, a string as a number, none) and
# the percentage arithmetic. Boot 1 has no info or charge lines.
# Tenth slice (13 Nov 2026): method calls with Args, as most firmware shares one helper between its batteries. Every
# Method's path, argument count and body (up to 1 KiB, 256 KiB in all) is kept; a call works out its arguments, then
# runs the body with its own Locals and Args (copies), reading fields and sharing the run's named Package copies.
# The test SSDT's BAT0 _BST is now Return(GBST(0x02, PBST)), GBST (2 args) in \_SB_ filling Index(Arg1, i) and
# returning Arg1, so boot 2's _BST line is printed only when calls work. Selftest case 13 runs eight calls (a helper
# filling a Package, a call as a statement, an Arg not given, calls without end, no Return, a helper too long to
# keep, the caller's Locals kept, a call inside an expression), so the selftest is 13 of 13.
# Eleventh slice (13 Nov 2026): arithmetic (Add, Subtract, Multiply, ShiftLeft, ShiftRight, Divide, Mod, Not), as
# helpers scale the controller's raw values. The test SSDT's BSTR is now 300 and GBST stores
# ShiftRight(Multiply(BSTR, 20), One) into Index(Arg1, 2), so boot 2's "remaining 3000 mAh" is printed only when
# arithmetic works. Selftest case 14 runs nine sums (wrapping, a shift by 64, Divide's two targets, by Zero, Mod,
# Not, a controller field scaled), so the selftest is 14 of 14.
# Twelfth slice (13 Nov 2026): loops (While, Break, Continue) with Increment and Decrement of a Local or an Arg, as
# helpers loop over bytes and wait on the controller; at most 65535 rounds a run. The test SSDT's GBST now counts
# BSTR (300) down to Zero in a While, adding 20 to Local0 each round, then stores ShiftRight(Local0, One) into
# Index(Arg1, 2), so boot 2's "remaining 3000 mAh" is printed only when loops work. Selftest case 15 runs eleven
# loops (counting up and down, Break, Continue, a Return inside, one without end, a Break outside a While, an
# Increment of a controller field, a loop reading the controller, Decrement wrapping, a Break in a called method),
# so the selftest is 15 of 15.
#
# QEMU's q35 has no battery, so two power-ons with qemu-system-x86_64 DIRECTLY:
#   boot 1: plain q35: QEMU's own DSDT is walked (named objects counted) and "none described" is said.
#   boot 2: the same plus a test SSDT built here (-acpitable): Device BAT0 (PNP0C0A, _STA _BIF _BST), ADP1
#           ("ACPI0003", _PSR), LID0 (PNP0C0D, _LID) and EC0_ (PNP0C09 with one EmbeddedControl region, ports
#           0x62 and 0x66 in its _CRS, four named field units) under
#           \_SB_. Each must be named by its full path with its methods, and the charge said to need the
#           interpreter.
#   Both: battery selftest 15 of 15, `battery x` gives the usage line, acpiinfo still answers afterwards.
#   scripts/pc-battery-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-battery-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

R_HEAD='^battery: what the firmware describes, read from its ACPI tables \(nothing is run or changed\)$'
R_USAGE='^battery: type battery, battery ec, or battery selftest$'
R_TABLES='^  tables: the DSDT and [0-9]+ SSDT\(s\) read: ([0-9]+) named objects, [0-9]+ conditional block\(s\) not entered, [0-9]+ cut short$'
R_NONE='^  battery: none described \(a desktop PC or a virtual machine has none\)$'
R_BAT='^  battery: \\_SB_\.BAT0, methods _STA _BIF _BST, status 0x1F \(present\)$'
R_AC='^  AC adapter: \\_SB_\.ADP1, methods _PSR, status 0x0F \(present\)$'
R_LID='^  lid: \\_SB_\.LID0, methods _LID, no _STA \(present by the ACPI rules\)$'
R_EC='^  embedded controller: \\_SB_\.EC0_, methods _STA, 1 region\(s\) in its own address space, status 0x0F \(present\)$'
R_STA='^  status \(_STA\): ([0-9]+) device\(s\) have one: ([0-9]+) worked out without running anything, ([0-9]+) need the interpreter$'
sta() { # sta <cleaned text> <field 1 total, 2 known, 3 run>
    grep -aE "$R_STA" <<< "$1" | head -1 | sed -E "s/$R_STA/\\$2/"
}
R_PORTS='^    ports \(from its _CRS\): data 0x62, command and status 0x66$'
R_FIELDS='^    fields: 4 named in its own address space, bytes 0x00 to 0x04$'
R_U1='^      BST0: byte 0x00, 8 bits$'
R_U2='^      BRC0: byte 0x02, 16 bits$'
R_U3='^      ACIN: byte 0x04 bit 4, 1 bit$'
R_U4='^      B1ST: byte 0x00, 8 bits$'
R_ECHEAD='^battery ec: the embedded controller.s named fields, read through its ports \(read command 0x80 only, nothing is written to its memory\)$'
R_ECNONE='^  embedded controller: none described$'
R_ECDEV='^  embedded controller \\_SB_\.EC0_:$'
R_ECQUIET='^    (not read: nothing answers at port 0x66 \(it reads 0xFF\), so nothing was sent|stopped at byte 0x00: the controller did not (take the read command|take the address|answer) in time)$'
R_ECVAL='^      [A-Z0-9_]{4} = '
R_AC2='^  AC adapter: \\_SB_\.ADP2, methods _PSR, no _STA \(present by the ACPI rules\)$'
R_PSR1='^  AC adapter \\_SB_\.ADP1: power source not worked out: (ACIN lives in the embedded controller, and nothing answers at its command port \(it reads 0xFF\)|reading ACIN from the embedded controller did not finish in time)$'
R_PSR2='^  AC adapter \\_SB_\.ADP2: on mains power \(_PSR gives 0x1\)$'
R_CHARGE='^  charge: not read here; the battery gives it through its _BST method, which battery ec runs read only$'
R_BST='^  battery \\_SB_\.BAT0: state 0x2 \(charging\), rate unknown, remaining 3000 mAh, voltage 12000 mV$'
R_INFO='^    info \(_BIF\): design capacity 6000 mAh, last full charge 4500 mAh, design voltage 12000 mV, model AFRI-TEST, serial 0001, type LION$'
R_PCT='^    charge: 66% of the last full charge$'

common() { # common <cleaned text> <boot name>; prints FAIL lines
    local S="$1" b="$2" n
    has() { grep -aqE "$1" <<< "$S"; }
    if has 'KERNEL PANIC|\[pc\] (fault|exception|EXCEPTION|PANIC)'; then echo "  FAIL: $b: the kernel panicked or faulted"; fi
    has "$R_HEAD" || echo "  FAIL: $b: battery did not answer"
    has "$R_TABLES" || echo "  FAIL: $b: no tables line (the DSDT was not read)"
    n="$(grep -aE "$R_TABLES" <<< "$S" | head -1 | sed -E 's/.*read: ([0-9]+) named.*/\1/')"
    [ -n "$n" ] && [ "$n" -lt 20 ] && echo "  FAIL: $b: only $n named objects read from QEMU's DSDT (the walk stopped early)"
    has '^battery selftest: 15 of 15$' || echo "  FAIL: $b: battery selftest is not 15 of 15"
    has "$R_ECHEAD" || echo "  FAIL: $b: battery ec did not answer"
    has "$R_ECVAL" && echo "  FAIL: $b: battery ec printed a value, but QEMU has no embedded controller"
    has "$R_STA" || echo "  FAIL: $b: no status (_STA) line"
    if has '^battery selftest: .*: FAILED$'; then echo "  FAIL: $b: a selftest case failed"; fi
    has "$R_USAGE" || echo "  FAIL: $b: 'battery x' did not give the usage line"
    has '^acpi: ' || echo "  FAIL: $b: the shell did not answer acpiinfo after battery"
}

grade() { # grade <boot 1 log> <boot 2 log>
    local fail=0 S1 S2 out r
    for f in "$1" "$2"; do [ -r "$f" ] || { echo "  FAIL: missing serial log $f"; return 1; }; done
    clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    S1="$(clean "$1")"; S2="$(clean "$2")"
    out="$(common "$S1" "boot 1"; common "$S2" "boot 2")"
    [ -n "$out" ] && { printf '%s\n' "$out"; fail=1; }
    grep -aqE "$R_NONE" <<< "$S1" || { echo "  FAIL: boot 1: plain q35 was not reported 'none described'"; fail=1; }
    if grep -aqE '^  (battery|AC adapter|lid|embedded controller): \\' <<< "$S1"; then echo "  FAIL: boot 1: a device was invented on plain q35"; fail=1; fi
    if grep -aqE '^  charge: ' <<< "$S1"; then echo "  FAIL: boot 1: a charge line with no battery"; fail=1; fi
    if grep -aqE '^    (ports|fields)' <<< "$S1"; then echo "  FAIL: boot 1: controller lines with no controller"; fail=1; fi
    if grep -aqE '^    (info|charge)' <<< "$S1"; then echo "  FAIL: boot 1: battery info or charge lines with no battery"; fail=1; fi
    grep -aqE "$R_ECNONE" <<< "$S1" || { echo "  FAIL: boot 1: battery ec did not say none is described"; fail=1; }
    if grep -aqE '^  embedded controller \\' <<< "$S1"; then echo "  FAIL: boot 1: battery ec found a controller on plain q35"; fail=1; fi
    if grep -aqE '^  AC adapter \\' <<< "$S1"; then echo "  FAIL: boot 1: battery ec ran a _PSR on plain q35"; fail=1; fi
    if grep -aqE '^  battery \\' <<< "$S1"; then echo "  FAIL: boot 1: battery ec ran a _BST on plain q35"; fail=1; fi
    if grep -aqE '^  AC adapter \\_SB_\.ADP1: on ' <<< "$S2"; then echo "  FAIL: boot 2: ADP1's _PSR gave a value, but QEMU has no embedded controller"; fail=1; fi
    for r in "$R_BAT" "$R_AC" "$R_AC2" "$R_PSR1" "$R_PSR2" "$R_LID" "$R_EC" "$R_PORTS" "$R_FIELDS" "$R_U1" "$R_U2" "$R_U3" "$R_U4" "$R_CHARGE" "$R_BST" "$R_INFO" "$R_PCT" "$R_ECDEV" "$R_ECQUIET"; do
        grep -aqE "$r" <<< "$S2" || { echo "  FAIL: boot 2: missing line matching $r"; fail=1; }
    done
    if grep -aqE "$R_NONE" <<< "$S2"; then echo "  FAIL: boot 2: 'none described' with the test battery present"; fail=1; fi
    if grep -aqE '\(partly read\)$' <<< "$S2"; then echo "  FAIL: boot 2: a test device was only partly read"; fail=1; fi
    local t1 k1 r1 t2 k2 r2
    t1="$(sta "$S1" 1)"; k1="$(sta "$S1" 2)"; r1="$(sta "$S1" 3)"
    t2="$(sta "$S2" 1)"; k2="$(sta "$S2" 2)"; r2="$(sta "$S2" 3)"
    if [ -n "$t1" ] && [ -n "$t2" ]; then
        [ "$t1" -eq $((k1 + r1)) ] && [ "$t2" -eq $((k2 + r2)) ] || { echo "  FAIL: a status total is not worked out plus interpreter"; fail=1; }
        [ "$k1" -ge 1 ] || { echo "  FAIL: boot 1: no status worked out from QEMU's own DSDT"; fail=1; }
        [ "$k2" -eq $((k1 + 3)) ] || { echo "  FAIL: boot 2 worked out $k2 statuses, expected boot 1's $k1 plus the test SSDT's 3"; fail=1; }
        [ "$r2" -eq "$r1" ] || { echo "  FAIL: boot 2 has $r2 statuses needing the interpreter, boot 1 had $r1 (the test SSDT adds none)"; fail=1; }
    fi
    return "$fail"
}

SELF="battery selftest: three devices: ok
battery selftest: nested controller: ok
battery selftest: unknown form: ok
battery selftest: conditional: ok
battery selftest: cut off: ok
battery selftest: root and parent names: ok
battery selftest: status values: ok
battery selftest: controller ports and fields: ok
battery selftest: controller reads: ok
battery selftest: power source: ok
battery selftest: battery status: ok
battery selftest: battery info: ok
battery selftest: method calls: ok
battery selftest: arithmetic: ok
battery selftest: loops: ok
battery selftest: 15 of 15
battery: type battery, battery ec, or battery selftest"
B1="[pc] MANSA-PC BOOT OK
afrisovera> battery
battery: what the firmware describes, read from its ACPI tables (nothing is run or changed)
  tables: the DSDT and 0 SSDT(s) read: 180 named objects, 0 conditional block(s) not entered, 0 cut short
  status (_STA): 9 device(s) have one: 4 worked out without running anything, 5 need the interpreter
  battery: none described (a desktop PC or a virtual machine has none)
afrisovera> battery ec
battery ec: the embedded controller's named fields, read through its ports (read command 0x80 only, nothing is written to its memory)
  embedded controller: none described
afrisovera> battery selftest
$SELF
afrisovera> acpiinfo
acpi: 1 core(s), local-APIC ids [0], I/O APIC at 0xfec00000"
B2="[pc] MANSA-PC BOOT OK
afrisovera> battery
battery: what the firmware describes, read from its ACPI tables (nothing is run or changed)
  tables: the DSDT and 1 SSDT(s) read: 196 named objects, 0 conditional block(s) not entered, 0 cut short
  status (_STA): 12 device(s) have one: 7 worked out without running anything, 5 need the interpreter
  battery: \\_SB_.BAT0, methods _STA _BIF _BST, status 0x1F (present)
  AC adapter: \\_SB_.ADP1, methods _PSR, status 0x0F (present)
  AC adapter: \\_SB_.ADP2, methods _PSR, no _STA (present by the ACPI rules)
  lid: \\_SB_.LID0, methods _LID, no _STA (present by the ACPI rules)
  embedded controller: \\_SB_.EC0_, methods _STA, 1 region(s) in its own address space, status 0x0F (present)
    ports (from its _CRS): data 0x62, command and status 0x66
    fields: 4 named in its own address space, bytes 0x00 to 0x04
      BST0: byte 0x00, 8 bits
      BRC0: byte 0x02, 16 bits
      ACIN: byte 0x04 bit 4, 1 bit
      B1ST: byte 0x00, 8 bits
  charge: not read here; the battery gives it through its _BST method, which battery ec runs read only
afrisovera> battery ec
battery ec: the embedded controller's named fields, read through its ports (read command 0x80 only, nothing is written to its memory)
  embedded controller \\_SB_.EC0_:
    not read: nothing answers at port 0x66 (it reads 0xFF), so nothing was sent
  AC adapter \\_SB_.ADP1: power source not worked out: ACIN lives in the embedded controller, and nothing answers at its command port (it reads 0xFF)
  AC adapter \\_SB_.ADP2: on mains power (_PSR gives 0x1)
  battery \\_SB_.BAT0: state 0x2 (charging), rate unknown, remaining 3000 mAh, voltage 12000 mV
    info (_BIF): design capacity 6000 mAh, last full charge 4500 mAh, design voltage 12000 mV, model AFRI-TEST, serial 0001, type LION
    charge: 66% of the last full charge
afrisovera> battery selftest
$SELF
afrisovera> acpiinfo
acpi: 1 core(s), local-APIC ids [0], I/O APIC at 0xfec00000"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-battery-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <boot 1 text> <boot 2 text>
        local got
        printf '%s\n' "$3" > "$D/1"; printf '%s\n' "$4" > "$D/2"
        if grade "$D/1" "$D/2" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm no-battery-cmd red "$(printf '%s\n' "$B1" | grep -v '^battery: what')" "$B2"
    arm no-tables red "$B1" "$(printf '%s\n' "$B2" | grep -v '^  tables:')"
    arm walk-stopped red "$(printf '%s\n' "$B1" | sed 's/read: 180 named/read: 3 named/')" "$B2"
    arm selftest-short red "$(printf '%s\n' "$B1" | sed 's/^battery selftest: 15 of 15/battery selftest: 14 of 15/')" "$B2"
    arm case-failed red "$B1" "$(printf '%s\n' "$B2" | sed 's/^battery selftest: conditional: ok/battery selftest: conditional: FAILED/')"
    arm no-usage red "$B1" "$(printf '%s\n' "$B2" | grep -v '^battery: type')"
    arm invented red "$(printf '%s\n' "$B1" | sed 's/^  battery: none described.*/  battery: \\_SB_.BAT0/')" "$B2"
    arm no-battery-line red "$B1" "$(printf '%s\n' "$B2" | grep -v '^  battery: \\')"
    arm wrong-methods red "$B1" "$(printf '%s\n' "$B2" | sed 's/methods _STA _BIF _BST/methods _STA _BST/')"
    arm no-ec-region red "$B1" "$(printf '%s\n' "$B2" | sed 's/1 region(s)/0 region(s)/')"
    arm partly red "$B1" "$(printf '%s\n' "$B2" | sed 's/rules)$/rules) (partly read)/')"
    arm no-charge red "$B1" "$(printf '%s\n' "$B2" | grep -v '^  charge:')"
    arm none-with-battery red "$B1" "$(printf '%s\n  battery: none described (a desktop PC or a virtual machine has none)\n' "$B2")"
    arm shell-stopped red "$B1" "$(printf '%s\n' "$B2" | grep -v '^acpi:')"
    arm panic red "$B1" "$(printf '%s\nKERNEL PANIC\n' "$B2")"
    arm no-sta-line red "$B1" "$(printf '%s\n' "$B2" | grep -v '^  status (_STA)')"
    arm wrong-status red "$B1" "$(printf '%s\n' "$B2" | sed 's/BST, status 0x1F/BST, status 0x01/')"
    arm sta-not-worked red "$B1" "$(printf '%s\n' "$B2" | sed 's/_PSR, status 0x0F (present)/_PSR, status needs the interpreter/')"
    arm sta-delta red "$B1" "$(printf '%s\n' "$B2" | sed 's/12 device(s) have one: 7 worked out/12 device(s) have one: 6 worked out/')"
    arm sta-run-grew red "$B1" "$(printf '%s\n' "$B2" | sed 's/12 device(s) have one: 7 worked out without running anything, 5 need/13 device(s) have one: 7 worked out without running anything, 6 need/')"
    arm sta-sum red "$(printf '%s\n' "$B1" | sed 's/9 device(s) have one/8 device(s) have one/')" "$B2"
    arm sta-none-qemu red "$(printf '%s\n' "$B1" | sed 's/9 device(s) have one: 4 worked out/5 device(s) have one: 0 worked out/')" "$(printf '%s\n' "$B2" | sed 's/12 device(s) have one: 7 worked out/8 device(s) have one: 3 worked out/')"
    arm no-ports red "$B1" "$(printf '%s\n' "$B2" | grep -v '^    ports')"
    arm wrong-port red "$B1" "$(printf '%s\n' "$B2" | sed 's/status 0x66$/status 0x62/')"
    arm no-fields red "$B1" "$(printf '%s\n' "$B2" | grep -v '^    fields')"
    arm wrong-offset red "$B1" "$(printf '%s\n' "$B2" | sed 's/BRC0: byte 0x02/BRC0: byte 0x01/')"
    arm wrong-bit red "$B1" "$(printf '%s\n' "$B2" | sed 's/ACIN: byte 0x04 bit 4/ACIN: byte 0x04/')"
    arm missing-unit red "$B1" "$(printf '%s\n' "$B2" | grep -v 'B1ST')"
    arm ec-lines-boot1 red "$(printf '%s\n    fields: none named in its own address space\n' "$B1")" "$B2"
    arm ec-timeout green "$B1" "$(printf '%s\n' "$B2" | sed 's/^    not read: nothing answers.*/    stopped at byte 0x00: the controller did not answer in time/')"
    arm no-ec-head red "$B1" "$(printf '%s\n' "$B2" | grep -v '^battery ec:')"
    arm ec-value-invented red "$B1" "$(printf '%s\n' "$B2" | sed 's/^    not read: nothing answers.*/      BST0 = 0x2 (2), byte 0x00, 8 bit(s)/')"
    arm ec-not-quiet red "$B1" "$(printf '%s\n' "$B2" | grep -v '^    not read: nothing answers')"
    arm ec-boot1-found red "$(printf '%s\n' "$B1" | sed 's/^  embedded controller: none described/  embedded controller \\\\_SB_.EC0_:/')" "$B2"
    arm psr-mains-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v 'ADP2: on mains')"
    arm psr-adp1-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v 'ADP1: power source')"
    arm psr-adp1-invented red "$B1" "$(printf '%s\n' "$B2" | sed 's/ADP1: power source not worked out.*/ADP1: on mains power (_PSR gives 0x1)/')"
    arm psr-boot1 red "$(printf '%s\n  AC adapter \\\\_SB_.ADP2: on mains power (_PSR gives 0x1)\n' "$B1")" "$B2"
    arm psr-timeout green "$B1" "$(printf '%s\n' "$B2" | sed 's/ADP1: power source not worked out.*/ADP1: power source not worked out: reading ACIN from the embedded controller did not finish in time/')"
    arm no-adp2 red "$B1" "$(printf '%s\n' "$B2" | grep -v 'ADP2, methods')"
    arm bst-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v 'BAT0: state')"
    arm bst-boot1 red "$(printf '%s\n  battery \\\\_SB_.BAT0: state 0x0 (neither charging nor discharging), rate 0, remaining 0, voltage 0 mV\n' "$B1")" "$B2"
    arm bst-wrong-state red "$B1" "$(printf '%s\n' "$B2" | sed 's/state 0x2 (charging)/state 0x1 (discharging)/')"
    arm bst-no-units red "$B1" "$(printf '%s\n' "$B2" | sed 's/remaining 3000 mAh/remaining 3000 mWh or mAh/')"
    arm info-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v '^    info (_BIF)')"
    arm info-wrong-full red "$B1" "$(printf '%s\n' "$B2" | sed 's/last full charge 4500 mAh/last full charge 0 mAh/')"
    arm pct-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v '^    charge: 66%')"
    arm pct-wrong red "$B1" "$(printf '%s\n' "$B2" | sed 's/charge: 66%/charge: 67%/')"
    arm info-boot1 red "$(printf '%s\n    charge: 66%% of the last full charge\n' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 52 ]
}

echo "== PC battery, AC adapter, lid: battery (read only) =="
if ! selftest; then echo "PC BATTERY TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BATTERY SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BATTERY TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BATTERY TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE_4M.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC BATTERY TEST FAILED (harness: no x86-64 UEFI firmware found)"; exit 1; }

W="$(mktemp -d "$T/pc-battery.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

# the test SSDT: the same AML forms a laptop's firmware uses, worked out byte by byte
"$PY" - "$W/ssdt.aml" << 'EOF'
import struct, sys
def enc(op, rest):
    c = len(rest)
    if c + 1 < 64:
        pl = bytes([c + 1])
    else:
        l = c + 2
        pl = bytes([0x40 | (l & 0x0F), l >> 4])
    return op + pl + rest
dev = lambda name, body: enc(b'\x5b\x82', name + body)
method = lambda name: enc(b'\x14', name + b'\x00\xa4\x01')
ret = lambda name, val: enc(b'\x14', name + b'\x00\xa4' + val)
hid_eisa = lambda b: b'\x08_HID\x0c' + bytes(b)
hid_str = lambda s: b'\x08_HID\x0d' + s + b'\x00'
pkg = lambda n, els: enc(b'\x12', bytes([n]) + els)
# Name (PBST, Package (4) {Zero, 0xFFFFFFFF, Zero, 0x2EE0}) in BAT0, as real firmware keeps it, then
# _BST: Store (0x02, Index (PBST, Zero)) Store (BSTR, Index (PBST, 0x02)) Return (PBST),
#       with Name (BSTR, 0x0BB8) in the _SB_ scope
pbst = b'\x08PBST' + pkg(4, b'\x00\x0c\xff\xff\xff\xff\x00\x0b\xe0\x2e')
# since the tenth slice through a helper in \_SB_, as most firmware writes it: _BST: Return (GBST (0x02, PBST)) and
# Method (GBST, 2) {Store (Arg0, Index (Arg1, Zero)) Store (BSTR, Index (Arg1, 0x02)) Return (Arg1)}
# since the eleventh slice the helper scales a raw value, as firmware does: BSTR is 300 (0x012C) and GBST stores
# ShiftRight (Multiply (BSTR, 0x14), One) into Index (Arg1, 0x02), still 3000
# since the twelfth slice the helper loops, as firmware does over bytes: Store (Zero, Local0) Store (BSTR, Local1)
# While (Local1) {Decrement (Local1) Add (Local0, 0x14, Local0)} Store (ShiftRight (Local0, One), Index (Arg1, 0x02)),
# still 3000
bst = enc(b'\x14', b'_BST\x00' + b'\xa4GBST\x0a\x02PBST')
loop = enc(b'\xa2', b'\x61\x76\x61\x72\x60\x0a\x14\x60')
gbst = enc(b'\x14', b'GBST\x02' + b'\x70\x68\x88\x69\x00\x00' + b'\x70\x00\x60\x70BSTR\x61' + loop + b'\x70\x7a\x60\x01\x00\x88\x69\x0a\x02\x00' + b'\xa4\x69')
# Name (PBIF, Package (13) {One, 0x1770, Zero, One, 0x2EE0, 0x258, 0x12C, 0x40, 0x40, "AFRI-TEST", "0001", "LION",
# "AFRI"}) in BAT0, then _BIF: Store (BFCC, Index (PBIF, 0x02)) Return (PBIF), with Name (BFCC, 0x1194) in _SB_
st = lambda s: b'\x0d' + s + b'\x00'
pbif = b'\x08PBIF' + pkg(13, b'\x01\x0b\x70\x17\x00\x01\x0b\xe0\x2e\x0b\x58\x02\x0b\x2c\x01\x0a\x40\x0a\x40' + st(b'AFRI-TEST') + st(b'0001') + st(b'LION') + st(b'AFRI'))
bif = enc(b'\x14', b'_BIF\x00' + b'\x70BFCC\x88PBIF\x0a\x02\x00' + b'\xa4PBIF')
bat0 = dev(b'BAT0', hid_eisa([0x41, 0xD0, 0x0C, 0x0A]) + b'\x08_UID\x01' + ret(b'_STA', b'\x0a\x1f') + bif + pbif + pbst + bst)
psr = lambda body: enc(b'\x14', b'_PSR\x00' + body)
# ADP1: If (LEqual (^^EC0_.ACIN, One)) { Return (One) } Return (Zero)
adp1 = dev(b'ADP1', hid_str(b'ACPI0003') + b'\x08_STA\x0a\x0f' + psr(enc(b'\xa0', b'\x93^^\x2eEC0_ACIN\x01' + b'\xa4\x01') + b'\xa4\x00'))
# ADP2: If (LAnd (LEqual (PSRC, One), LNot (Zero))) { Return (One) } Else { Return (Zero) }, Name (PSRC, One) in \_SB_
#       now through Local0: Store (PSRC, Local0) Store (Local0, Debug), the If compares Local0 and returns it
adp2 = dev(b'ADP2', hid_str(b'ACPI0003') + psr(b'\x70PSRC\x60' + b'\x70\x60\x5b\x31' + enc(b'\xa0', b'\x90\x93\x60\x01\x92\x00' + b'\xa4\x60') + enc(b'\xa1', b'\xa4\x00')))
lid0 = dev(b'LID0', hid_eisa([0x41, 0xD0, 0x0C, 0x0D]) + method(b'_LID'))
region = b'\x5b\x80ECOR\x03\x00\x0a\xff'
# BST0 8 bits, Offset(2), BRC0 16 bits, 4 bits reserved, ACIN 1 bit
field = enc(b'\x5b\x81', b'ECOR\x01' + b'BST0\x08' + b'\x00\x08' + b'BRC0\x10' + b'\x00\x04' + b'ACIN\x01')
io = lambda p: bytes([0x47, 0x01, p, 0x00, p, 0x00, 0x00, 0x01])
res = io(0x62) + io(0x66) + b'\x79\x00'
crs = b'\x08_CRS' + enc(b'\x11', bytes([0x0a, len(res)]) + res)
ec0 = dev(b'EC0_', hid_eisa([0x41, 0xD0, 0x0C, 0x09]) + crs + ret(b'_STA', b'ECST') + region + field)
body = enc(b'\x10', b'\\_SB_' + gbst + bat0 + adp1 + adp2 + lid0 + ec0 + b'\x08ECST\x0a\x0f' + b'\x08PSRC\x01' + b'\x08BSTR\x0b\x2c\x01' + b'\x08BFCC\x0b\x94\x11')
# a second Field over the same region, from a later Scope, as firmware often splits them
body += enc(b'\x10', b'\\\x2e_SB_EC0_' + enc(b'\x5b\x81', b'ECOR\x01B1ST\x08'))
hdr = b'SSDT' + struct.pack('<I', 36 + len(body)) + bytes([2, 0]) + b'AFRISV' + b'BATTTEST' + struct.pack('<I', 1) + b'AFRI' + struct.pack('<I', 1)
t = bytearray(hdr + body)
t[9] = (-sum(t)) & 0xFF
open(sys.argv[1], 'wb').write(t)
EOF
[ -s "$W/ssdt.aml" ] || { echo "PC BATTERY TEST FAILED (harness: the test SSDT was not built)"; exit 1; }

boot() { # boot <log> <extra qemu args, as one string or empty> <commands...>
    local log="$1" extra="$2"; shift 2
    # shellcheck disable=SC2086
    "$REALQ" -M q35 -m 256M -display none $extra \
        -drive "if=pflash,format=raw,readonly=on,file=$(winpath "$OVMF")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
        -serial "tcp:127.0.0.1:5826,server,nowait" -qmp "tcp:127.0.0.1:5827,server,nowait" 2> "$log.qemu" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5826 5827 "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    [ -s "$log.qemu" ] && sed 's/^/  qemu says: /' "$log.qemu"
}

CMDS=("battery" "battery ec" "battery selftest" "battery x" "acpiinfo")
echo "  --- boot 1: plain q35, no battery ---"
boot "$W/boot1.log" "" "${CMDS[@]}"
tr -d '\r' < "$W/boot1.log" | grep -aE '^battery|^  (tables|status|battery|AC|lid|embedded|thermal|charge|\(|  )' | sed 's/^/  | /'
echo "  --- boot 2: q35 plus a test SSDT with a battery, AC adapter, lid and embedded controller ---"
boot "$W/boot2.log" "-acpitable file=$(winpath "$W/ssdt.aml")" "${CMDS[@]}"
tr -d '\r' < "$W/boot2.log" | grep -aE '^battery|^  (tables|status|battery|AC|lid|embedded|thermal|charge|\(|  )' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC BATTERY TEST PASSED"
    exit 0
fi
echo "  (serial logs kept: $W/boot1.log $W/boot2.log)"
echo "PC BATTERY TEST FAILED"
exit 1
