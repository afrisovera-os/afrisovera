#!/usr/bin/env bash
# mesh-noise suite: the hand-written no_std Noise XX/KK driver for the kernel port. Proves correctness by
# interop with snow (the reference Noise implementation): handshake completes, handshake hashes agree, and
# transport messages round-trip, for XX in both roles and for KK; plus a self-consistency handshake.
# Also builds the crate for the kernel bare-metal target to prove it is no_std.
#
# Host-only tests (repo default target is bare-metal aarch64). The snow dev-dependency pulls proc-macro build
# scripts; on this locked-down PC Windows Application Control can transiently block one (os error 4551) - if the
# first run fails with that, run it again (it is not a code failure).
set -euo pipefail
# The runner reads "<NAME> TEST PASSED" / "<NAME> TEST FAILED" (scripts/lib/verdict.sh).
trap 'rc=$?; [ $rc -eq 0 ] || echo "MESH-NOISE TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
HOST=$(rustc -vV | sed -n 's/host: //p')
echo "mesh-noise-test: host triple ${HOST}"
cargo test --manifest-path mesh-noise/Cargo.toml --target "${HOST}" "$@"
echo "mesh-noise-test: no_std build for aarch64-unknown-none"
cargo build --manifest-path mesh-noise/Cargo.toml --target aarch64-unknown-none
echo "MESH-NOISE TEST PASSED"
