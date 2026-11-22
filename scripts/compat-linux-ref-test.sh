#!/usr/bin/env bash
# compat-linux-ref (Daraja stage 0, COMPAT_STAGE0_PLAN.md section 7, steps 1 and 2): the Linux side of the first
# end-to-end test. On a Linux machine (the cloud job's), with nothing prebuilt:
#   1. fetch musl 1.2.5's release tarball and check it: against MUSL_SHA256 below once pinned, and against a second,
#      independent source (Alpine's published sha512 for the same tarball) every time;
#   2. build musl, then our own tests/compat/hello0.c, fault0.c and bench0.c with it: static, not position-independent,
#      linked at USER_BASE + 0x40_0000 (0x10000400000), so the same binaries run on Linux and in AfriSovera's
#      foreign area (AFR-0001 7.7); record each binary's sha256;
#   3. run them on Linux exactly as the bridge will: in a copy of tests/compat/bundle, `env -i`, stdin from
#      /dev/null, `hello0 one two`; keep stdout, stderr and the exit code, and the system call list from strace;
#   4. check the reference is what the programs promise (hello0 exits 7 and prints its file, fault0 gives 139, bench0
#      exits 0), and that each binary is static, ET_EXEC and inside the foreign area.
# The reference lands in build/compat-ref/ (out1, out2, exit, calls, sha256 for each program), the input of the
# AfriSovera side (pc-compat0, step 3). Graded by the checks in 4; prints the musl hash for pinning while unpinned.
#   scripts/compat-linux-ref-test.sh
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
OUT="$ROOT/build/compat-ref"
MUSL_VER=1.2.5
MUSL_URL="https://musl.libc.org/releases/musl-$MUSL_VER.tar.gz"
# Pinned 15 Nov 2026 after a cloud run confirmed it against the second source (Alpine 3.22-stable's sha512).
MUSL_SHA256="a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4"
# Second sources for the tarball's sha512, tried in turn (one site refusing automated fetches must not stop the run):
# Alpine's package recipe on its stable branches 3.22 and 3.21, which ship musl 1.2.5 (master and Gentoo have moved to
# 1.2.6), through its GitHub mirror and then its GitLab. Each publishes the sha512 of exactly this upstream file.
SECOND_SOURCES=(
    "https://raw.githubusercontent.com/alpinelinux/aports/3.22-stable/main/musl/APKBUILD"
    "https://raw.githubusercontent.com/alpinelinux/aports/3.21-stable/main/musl/APKBUILD"
    "https://gitlab.alpinelinux.org/alpine/aports/-/raw/3.22-stable/main/musl/APKBUILD"
)
LINK_AT=0x10000400000
# Daraja L1.1 (COMPAT_L1_PLAN 6 step 1): lua 5.4.7 from lua.org, pinned; Alpine's lua5.4 recipe on its 3.21 and 3.22
# stable branches publishes the sha512 of the same upstream file
LUA_VER=5.4.7
LUA_URL="https://www.lua.org/ftp/lua-$LUA_VER.tar.gz"
LUA_SHA256="9fbf5e28ef86c69858f6d3d34eccc32e911c1a28b4120ff3e84aaa70cfbf1e30"
LUA_SECOND=(
    "https://raw.githubusercontent.com/alpinelinux/aports/3.21-stable/main/lua5.4/APKBUILD"
    "https://raw.githubusercontent.com/alpinelinux/aports/3.22-stable/main/lua5.4/APKBUILD"
)

fail() { echo "  FAIL: $*"; echo "COMPAT-LINUX-REF TEST FAILED"; exit 1; }
W="$(mktemp -d "${TMPDIR:-/tmp}/compat-ref.XXXXXX")"
rm -rf "$OUT"; mkdir -p "$OUT"

echo "  --- musl $MUSL_VER: fetch and check ---"
curl -fsSL --retry 3 -o "$W/musl.tgz" "$MUSL_URL" || fail "could not fetch $MUSL_URL"
got256="$(sha256sum "$W/musl.tgz" | cut -d' ' -f1)"
got512="$(sha512sum "$W/musl.tgz" | cut -d' ' -f1)"
echo "  musl-$MUSL_VER.tar.gz sha256 $got256"
if [ -n "$MUSL_SHA256" ]; then
    [ "$got256" = "$MUSL_SHA256" ] || fail "musl tarball sha256 $got256, pinned $MUSL_SHA256"
    echo "  pinned sha256: matches"
fi
second=""; from=""
for u in "${SECOND_SOURCES[@]}"; do
    curl -fsSL --retry 2 -A "afrisovera-ci" -o "$W/second" "$u" 2> /dev/null || { echo "  second source unreachable: $u"; continue; }
    # Alpine: "<sha512>  musl-1.2.5.tar.gz"; Gentoo: "DIST musl-1.2.5.tar.gz <size> BLAKE2B <hex> SHA512 <hex>"
    second="$(grep -E "^[0-9a-f]{128}  musl-$MUSL_VER\.tar\.gz$" "$W/second" | cut -d' ' -f1 | head -1)"
    [ -n "$second" ] || second="$(grep -E "^DIST musl-$MUSL_VER\.tar\.gz " "$W/second" | sed -nE 's/.* SHA512 ([0-9a-f]{128}).*/\1/p' | head -1)"
    if [ -n "$second" ]; then from="$u"; break; fi
    echo "  second source does not list musl-$MUSL_VER.tar.gz: $u"
done
if [ -n "$second" ]; then
    [ "$second" = "$got512" ] || fail "musl tarball sha512 differs from the one published at $from"
    echo "  second source ($from): sha512 matches"
elif [ -z "$MUSL_SHA256" ]; then
    fail "not pinned and no second source for musl-$MUSL_VER.tar.gz could be read; pin by hand"
else
    echo "  second source: none readable; the pin alone is used"
fi
[ -z "$MUSL_SHA256" ] && echo "  TO PIN: MUSL_SHA256=\"$got256\" (confirmed by the second source)"

echo "  --- musl: build ---"
tar -xzf "$W/musl.tgz" -C "$W" || fail "could not unpack musl"
( cd "$W/musl-$MUSL_VER" && ./configure --prefix="$W/musl" --disable-shared > "$W/musl-configure.log" 2>&1 \
    && make -j"$(nproc)" > "$W/musl-build.log" 2>&1 && make install > /dev/null 2>&1 ) \
    || { tail -20 "$W/musl-configure.log" "$W/musl-build.log"; fail "musl did not build"; }
CC="$W/musl/bin/musl-gcc"
[ -x "$CC" ] || fail "no musl-gcc"

echo "  --- our test programs ---"
for p in hello0 fault0 bench0 mprot0 mprot1 clock1; do
    # musl-gcc's own start file is Scrt1.o (position-independent), whose reference to the undefined weak _DYNAMIC
    # (address 0) cannot reach from 0x10000400000 with a 32-bit relative relocation. A static, not
    # position-independent program takes musl's crt1.o instead, given here by hand around the program.
    L="$W/musl/lib"
    # Two more link flags for a program this high (second run, 15 Nov): crt1.o's _start also takes the address of the
    # weak _DYNAMIC (unused by crt1's _start_c, only by the static-PIE rcrt1), so it is given an address inside the
    # program; and the linker's relaxation of GOT loads into 32-bit immediates cannot work above 2 GiB, so it is off.
    "$CC" -static -no-pie -Os -nostartfiles -Wl,-Ttext-segment=$LINK_AT -Wl,--no-relax -Wl,--defsym=_DYNAMIC=$LINK_AT \
        -o "$OUT/$p" \
        "$L/crt1.o" "$L/crti.o" "tests/compat/$p.c" "$L/crtn.o" 2> "$W/$p.cc.log" \
        || { cat "$W/$p.cc.log"; fail "$p did not build"; }
    sha256sum "$OUT/$p" | cut -d' ' -f1 > "$OUT/$p.sha256"
    hdr="$(readelf -h "$OUT/$p")"
    grep -q 'Type: *EXEC' <<< "$hdr" || fail "$p is not ET_EXEC"
    grep -q 'Machine: *Advanced Micro Devices X86-64' <<< "$hdr" || fail "$p is not x86-64"
    # a here-string, not a pipe: under pipefail a `readelf | grep -q` can lose to SIGPIPE and skip this failure
    grep -qE 'INTERP|DYNAMIC' <<< "$(readelf -l "$OUT/$p")" && fail "$p is not static"
    low="$(readelf -lW "$OUT/$p" | awk '$1=="LOAD"{print $3}' | sort | head -1)"
    [ $(( low )) -ge $(( 0x10000000000 )) ] || fail "$p loads at $low, below the foreign area"
    echo "  $p: $(stat -c %s "$OUT/$p") bytes, sha256 $(cat "$OUT/$p.sha256"), lowest segment $low"
done

echo "  --- lua $LUA_VER (Daraja L1.1): fetch, check, build ---"
# lua.org's release tarball, pinned by sha256 and checked against a second source (Alpine's sha512 for the same file)
curl -fsSL --retry 3 -o "$W/lua.tgz" "$LUA_URL" || fail "could not fetch $LUA_URL"
lua256="$(sha256sum "$W/lua.tgz" | cut -d' ' -f1)"; lua512="$(sha512sum "$W/lua.tgz" | cut -d' ' -f1)"
echo "  lua-$LUA_VER.tar.gz sha256 $lua256"
lsecond=""; lfrom=""
for u in "${LUA_SECOND[@]}"; do
    curl -fsSL --retry 2 -A "afrisovera-ci" -o "$W/lsecond" "$u" 2> /dev/null || { echo "  second source unreachable: $u"; continue; }
    lsecond="$(grep -E "^[0-9a-f]{128}  lua-$LUA_VER\.tar\.gz$" "$W/lsecond" | cut -d' ' -f1 | head -1)"
    if [ -n "$lsecond" ]; then lfrom="$u"; break; fi
    echo "  second source does not list lua-$LUA_VER.tar.gz: $u"
done
if [ -n "$lsecond" ]; then
    [ "$lsecond" = "$lua512" ] || fail "lua tarball sha512 differs from the one published at $lfrom"
    echo "  second source ($lfrom): sha512 matches"
fi
if [ "$lua256" != "$LUA_SHA256" ]; then
    [ -n "$lsecond" ] && echo "  TO PIN: LUA_SHA256=\"$lua256\" (confirmed by the second source)"
    fail "lua tarball sha256 $lua256, pinned $LUA_SHA256"
fi
echo "  pinned sha256: matches"
tar -xzf "$W/lua.tgz" -C "$W" || fail "could not unpack lua"
mkdir -p "$W/luao"
for c in "$W/lua-$LUA_VER"/src/*.c; do
    case "$(basename "$c")" in luac.c) continue ;; esac   # the interpreter only (lua.c), not the compiler
    "$CC" -std=gnu99 -O2 -DLUA_USE_POSIX -c "$c" -o "$W/luao/$(basename "${c%.c}").o" 2> "$W/lua-cc.log" \
        || { cat "$W/lua-cc.log"; fail "lua: $(basename "$c") did not compile"; }
done
L="$W/musl/lib"
"$CC" -static -no-pie -nostartfiles -Wl,-Ttext-segment=$LINK_AT -Wl,--no-relax -Wl,--defsym=_DYNAMIC=$LINK_AT \
    -o "$OUT/lua" "$L/crt1.o" "$L/crti.o" "$W"/luao/*.o -lm "$L/crtn.o" 2> "$W/lua-ld.log" \
    || { cat "$W/lua-ld.log"; fail "lua did not link"; }
sha256sum "$OUT/lua" | cut -d' ' -f1 > "$OUT/lua.sha256"
hdr="$(readelf -h "$OUT/lua")"
grep -q 'Type: *EXEC' <<< "$hdr" || fail "lua is not ET_EXEC"
grep -qE 'INTERP|DYNAMIC' <<< "$(readelf -l "$OUT/lua")" && fail "lua is not static"
low="$(readelf -lW "$OUT/lua" | awk '$1=="LOAD"{print $3}' | sort | head -1)"
[ $(( low )) -ge $(( 0x10000000000 )) ] || fail "lua loads at $low, below the foreign area"
echo "  lua: $(stat -c %s "$OUT/lua") bytes, sha256 $(cat "$OUT/lua.sha256"), lowest segment $low"

echo "  --- run on Linux, as the bridge will ---"
command -v strace > /dev/null || (DEBIAN_FRONTEND=noninteractive apt-get install -y -qq strace > /dev/null 2>&1)
# The program is started exactly as the bridge starts it: argv[0] is its bare name (as in the bundle's daraja/argv), no
# environment at all. A tiny execve wrapper does that; its own calls come before the program's execve in strace's
# list, and the judge reads from the last execve on (first pc-compat0 run, 15 Nov: argv[0] was the full path).
EXECV='import os, sys; os.execve(sys.argv[1], sys.argv[2:], {})'
run() { # run <name> <args...>: in a copy of the bundle, empty environment, stdin /dev/null, argv[0] = <name>
    local p="$1"; shift
    rm -rf "$W/run"; cp -r tests/compat/bundle "$W/run"
    ( cd "$W/run" && python3 -c "$EXECV" "$OUT/$p" "$p" "$@" < /dev/null > "$OUT/$p.out1" 2> "$OUT/$p.out2" ); echo $? > "$OUT/$p.exit"
    if command -v strace > /dev/null; then
        ( cd "$W/run" && strace -f -qq -o "$OUT/$p.strace" python3 -c "$EXECV" "$OUT/$p" "$p" "$@" < /dev/null > /dev/null 2>&1 )
        sed -E 's/^[0-9]+ +//; s/\(.*$//' "$OUT/$p.strace" | grep -vE '^(\+\+\+|---)' > "$W/all.calls"
        # keep from the last execve (the program's own) on
        awk '{ l[NR] = $0; if ($0 == "execve") last = NR } END { for (i = last; i <= NR; i++) print l[i] }' "$W/all.calls" > "$OUT/$p.calls"
    fi
}
run hello0 one two
run fault0
run bench0
run mprot0
run mprot1
run lua data/lua1.lua
run clock1
for p in hello0 fault0 bench0 mprot0 mprot1 lua clock1; do
    echo "  $p: exit $(cat "$OUT/$p.exit"), stdout $(wc -c < "$OUT/$p.out1") bytes, stderr $(wc -c < "$OUT/$p.out2") bytes, calls: $(tr '\n' ' ' < "$OUT/$p.calls" 2> /dev/null)"
done

echo "  --- checks ---"
[ "$(cat "$OUT/hello0.exit")" = 7 ] || fail "hello0 exit $(cat "$OUT/hello0.exit"), want 7"
grep -qx 'hello from a Linux program' "$OUT/hello0.out1" || fail "hello0 did not greet"
grep -qx 'argv\[0\] hello0' "$OUT/hello0.out1" || fail "hello0's argv[0] is not its bare name"
grep -qx 'argv\[2\] two' "$OUT/hello0.out1" || fail "hello0 did not print its arguments"
grep -qF "$(cat tests/compat/bundle/data/greeting.txt)" "$OUT/hello0.out1" || fail "hello0 did not print its file"
grep -qx 'this line is on stderr' "$OUT/hello0.out2" || fail "hello0's stderr line is missing"
[ "$(cat "$OUT/fault0.exit")" = 139 ] || fail "fault0 exit $(cat "$OUT/fault0.exit"), want 139"
[ "$(cat "$OUT/bench0.exit")" = 0 ] || fail "bench0 exit $(cat "$OUT/bench0.exit"), want 0"
[ -s "$OUT/hello0.calls" ] || fail "no strace call list for hello0"
# L1.1: mprot0 removes rights (0 on both systems); mprot1 adds them, which Linux allows (exit 1: all three allowed)
[ "$(cat "$OUT/mprot0.exit")" = 0 ] || fail "mprot0 exit $(cat "$OUT/mprot0.exit"), want 0"
[ "$(cat "$OUT/mprot1.exit")" = 1 ] || fail "mprot1 exit $(cat "$OUT/mprot1.exit"), want 1 (Linux allows adding rights)"
[ "$(cat "$OUT/lua.exit")" = 3 ] || fail "lua exit $(cat "$OUT/lua.exit"), want 3 (lua1.lua's os.exit(3))"
grep -qxF "$(printf 'sum of squares 1..1000\t333833500')" "$OUT/lua.out1" || fail "lua1.lua's first line is not as expected"
# call 79: clock1 checks its own clocks and prints only verdicts; on Linux all seven hold (exit 0)
[ "$(cat "$OUT/clock1.exit")" = 0 ] || { cat "$OUT/clock1.out1"; fail "clock1 exit $(cat "$OUT/clock1.exit"), want 0"; }
grep -q WRONG "$OUT/clock1.out1" && fail "clock1 printed WRONG on Linux"
sed 's/^/  | /' "$OUT/hello0.out1"
rm -rf "$W"
echo "COMPAT-LINUX-REF TEST PASSED"
exit 0
