#!/usr/bin/env bash
# AFR-0009 mesh-hpke suite: the audited-crate sealing primitives (X25519, Ed25519 strict sign/verify,
# ChaCha20-Poly1305), checked against RFC 7748 6.1, RFC 8032 TEST 1 and RFC 8439 2.8.2.
#
# Host-only (repo default target is bare-metal aarch64): build for the detected HOST triple. The first run
# fetches the pinned crates from crates.io; later runs are offline. mesh-hpke is a detached workspace, and the
# kernel uses it as a path dependency (kernel/src/mesh/, ADR 0007).
set -euo pipefail
# The runner reads "<NAME> TEST PASSED" / "<NAME> TEST FAILED" (scripts/lib/verdict.sh).
trap 'rc=$?; [ $rc -eq 0 ] || echo "MESH-HPKE TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
HOST=$(rustc -vV | sed -n 's/host: //p')
echo "mesh-hpke-test: host triple ${HOST}"
cargo test --manifest-path mesh-hpke/Cargo.toml --target "${HOST}" "$@"
# Since the mesh Phase 0 window the kernel links mesh-hpke, so prove it is no_std for the kernel target too.
echo "mesh-hpke-test: no_std build for aarch64-unknown-none"
cargo build --manifest-path mesh-hpke/Cargo.toml --target aarch64-unknown-none
echo "MESH-HPKE TEST PASSED"
