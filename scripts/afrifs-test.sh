#!/usr/bin/env bash
# AFR-0011 VFS v1 (step V1): the afrifs crate's host tests. The vectors (the Rust writer makes the Python reference's
# bytes), the model scripts, the crash harness (every power cut at every write, torn sectors, reordered writes), the
# rules of names and space, a 16 MiB file, and the fuzz entry points on seeded random inputs. No QEMU.
# Bigger runs: AFRIFS_SCRIPTS, AFRIFS_CRASH_SCRIPTS, AFRIFS_REORDERS, AFRIFS_FUZZ_ROUNDS, AFRIFS_BIG=1 (256 MiB file).
set -euo pipefail
trap 'rc=$?; [ $rc -eq 0 ] || echo "AFRIFS TEST FAILED (exit $rc)"' EXIT
cd "$(dirname "$0")/.."
HOST=$(rustc -vV | sed -n 's/host: //p')
cargo test --release --manifest-path afrifs/Cargo.toml --target "$HOST" -- --nocapture --test-threads=4 2>&1 | grep -v "^running 0 tests"
python3 docs/spec/vectors/afr0011_vectors.py
echo "AFRIFS TEST PASSED"
