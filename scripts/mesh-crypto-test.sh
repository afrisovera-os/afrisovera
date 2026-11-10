#!/usr/bin/env bash
# AFR-0009 mesh-crypto host suite: the derivation + wire-format layer (HMAC-SHA256, HKDF-SHA256,
# contact root, recipient/beacon tags, SAS, identity-record + bundle layouts) checked against the
# architect's vectors in docs/spec/vectors/afr0009_vectors.py plus the RFC 5869 / FIPS known-answers.
#
# Host-only: the repo's default build target (.cargo/config.toml) is bare-metal aarch64-unknown-none, so
# this crate is built for the detected HOST triple instead. It has no dependencies, so no network is needed.
# This does NOT build the kernel and does NOT change the ARM image (mesh-crypto is a detached workspace).
set -euo pipefail
# The runner reads "<NAME> TEST PASSED" / "<NAME> TEST FAILED" (scripts/lib/verdict.sh).
trap 'rc=$?; [ $rc -eq 0 ] || echo "MESH-CRYPTO TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
HOST=$(rustc -vV | sed -n 's/host: //p')
echo "mesh-crypto-test: host triple ${HOST}"
cargo test --manifest-path mesh-crypto/Cargo.toml --target "${HOST}" "$@"
echo "MESH-CRYPTO TEST PASSED"
