#!/usr/bin/env bash
# Build Dira, the ring-3 web browser for the PC (userland/dira, Dira M3 S4), and write what the PC kernel includes:
#   kernel/src/pc/dira-x86_64.bin   the raw program, linked at the PC's user code address
#   kernel/src/pc/dira_bin.rs       its SHA-256, the SHA-256 of the source it was built from, and the toolchain
# The architect's rule (S4 verdict, Q1): no hand-committed bytes. These two files are only ever written by this script,
# and suite dira-bin rebuilds them from source with the pinned toolchain and fails if the committed ones differ.
#   scripts/build-dira.sh            # write into kernel/src/pc
#   scripts/build-dira.sh DIR        # write into DIR instead (dira-bin compares DIR with the committed files)
# Needs the pinned toolchain with the x86_64-unknown-none target and llvm-tools (cloud/job.sh installs both).
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-kernel/src/pc}"
TC="${RUSTUP_TOOLCHAIN:-1.97.1}"
mkdir -p "$OUT"
rustup target add x86_64-unknown-none --toolchain "$TC" > /dev/null 2>&1 || true
OC="$(rustc +"$TC" --print sysroot)/lib/rustlib/$(rustc +"$TC" -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
[ -x "$OC" ] || OC="$OC.exe"
# The same bytes on every machine: paths in panic locations are relative to the crate (it is its own workspace), and
# anything absolute is mapped away. --apply-dynamic-relocs fills the GOT at link time (it only loads at USER_BASE).
ROOT="$PWD"
(
    cd userland/dira
    RUSTFLAGS="-C link-arg=--apply-dynamic-relocs --remap-path-prefix=$ROOT=. --remap-path-prefix=$HOME=~" \
        cargo +"$TC" build --target x86_64-unknown-none -q
)
ELF=userland/dira/target/x86_64-unknown-none/debug/dira
"$OC" -O binary "$ELF" "$OUT/dira-x86_64.bin"
BYTES="$(wc -c < "$OUT/dira-x86_64.bin" | tr -d ' ')"
HASH="$(sha256sum "$OUT/dira-x86_64.bin" | cut -d' ' -f1)"
# the source, as the files themselves (the build may run from an export without .git)
SRC="$(cat userland/dira/Cargo.toml userland/dira/build.rs userland/dira/linker-x86_64.ld userland/dira/src/main.rs \
    kernel/src/dira/url.rs kernel/src/dira/http.rs kernel/src/dira/html.rs | sha256sum | cut -d' ' -f1)"
RUSTC="$(rustc +"$TC" -V)"
{
    echo "// Written by scripts/build-dira.sh: do not edit. Suite dira-bin rebuilds the program and compares."
    echo "/// SHA-256 of dira-x86_64.bin; the kernel starts no other bytes."
    printf 'pub const DIRA_SHA256: [u8; 32] = ['
    for i in $(seq 0 2 62); do printf '0x%s,' "${HASH:$i:2}"; done
    echo '];'
    echo "/// The program's size in bytes."
    echo "pub const DIRA_BYTES: usize = $BYTES;"
    echo "/// SHA-256 of the source it was built from (userland/dira and the three parsers in kernel/src/dira)."
    echo "pub const DIRA_SOURCE: &str = \"$SRC\";"
    echo "/// The toolchain that built it."
    echo "pub const DIRA_TOOLCHAIN: &str = \"$RUSTC\";"
} > "$OUT/dira_bin.rs"
echo "dira: $BYTES bytes, sha256 $HASH, source $SRC, $RUSTC -> $OUT"
[ "$BYTES" -le $((128 * 1024)) ] || { echo "dira: the program is past its 128 KiB code cap"; exit 1; }
