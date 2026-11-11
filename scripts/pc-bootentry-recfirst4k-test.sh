#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): AfriSovera's firmware boot entry on an NVMe Windows disk of 4096-byte blocks whose
# recovery partition comes BEFORE the EFI partition (GPT entry 1 recovery at block 256, entry 2 EFI at block 5376).
# The same five boots as scripts/pc-bootentry-test.sh (read that file); bootcopy, bootentry and the entry's HD() node
# must say nvme partition 2, start 5376 (in 4 KiB blocks) and the EFI partition's own GUID.
#   scripts/pc-bootentry-recfirst4k-test.sh [--selftest]
BOOTENTRY_KIND=recfirst4k exec bash "$(dirname "$0")/pc-bootentry-test.sh" "$@"
