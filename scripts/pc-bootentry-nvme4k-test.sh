#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): AfriSovera's firmware boot entry pointing at the EFI partition of a Windows-like NVMe disk
# with 4096-byte blocks, as many laptop SSDs have. The same five boots as scripts/pc-bootentry-test.sh (read that file);
# the entry's HD() node must say start 256 and 66560 blocks, counted in the disk's own 4 KiB blocks.
#   scripts/pc-bootentry-nvme4k-test.sh [--selftest]
BOOTENTRY_KIND=nvme4k exec bash "$(dirname "$0")/pc-bootentry-test.sh" "$@"
