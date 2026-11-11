#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootcopy and bootcopy remove on an NVMe Windows disk of 4096-byte blocks whose
# recovery partition comes BEFORE the EFI partition (GPT entry 1 recovery at block 256, entry 2 EFI at block 5376).
# The same four boots as scripts/pc-bootcopy-test.sh (read that file); every line must say nvme partition 2 (start
# 5376), and the image checks must find every sector outside the EFI partition, the recovery partition among them,
# unchanged.
#   scripts/pc-bootcopy-recfirst4k-test.sh [--selftest]
BOOTCOPY_KIND=recfirst4k exec bash "$(dirname "$0")/pc-bootcopy-test.sh" "$@"
