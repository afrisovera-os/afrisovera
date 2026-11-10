#!/usr/bin/env bash
# AFR-0009 mesh-link suite (link layer B1): Noise XX/KK via snow with the AFR-0009 prologue, record framing,
# SAS tie-in and a sealed bundle carried over the established link. Round-trip tests (no fixed Noise vector
# exists, handshakes being ephemeral): XX pairing + SAS agreement + bundle-over-link, KK contacts handshake,
# and wrong-peer-key rejection.
#
# Host-only (repo default target is bare-metal aarch64): build for the detected HOST triple. First run fetches
# the pinned crates (snow + its audited resolver deps). Does NOT build the kernel and does NOT change the ARM
# image (mesh-link is a detached workspace, not a kernel dependency).
set -euo pipefail
# The runner reads "<NAME> TEST PASSED" / "<NAME> TEST FAILED" (scripts/lib/verdict.sh).
trap 'rc=$?; [ $rc -eq 0 ] || echo "MESH-LINK TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
HOST=$(rustc -vV | sed -n 's/host: //p')
echo "mesh-link-test: host triple ${HOST}"
cargo test --manifest-path mesh-link/Cargo.toml --target "${HOST}" "$@"
echo "MESH-LINK TEST PASSED"
