#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootcopy and bootcopy remove on a Windows disk whose recovery partition comes BEFORE
# the EFI partition, as some OEM disks have it (GPT entry 1 recovery, entry 2 EFI at sector 43008). The same four boots
# as scripts/pc-bootcopy-test.sh (read that file); every line must say partition 2 (start 43008), and the image checks
# must find every sector outside the EFI partition, the recovery partition among them, unchanged.
#   scripts/pc-bootcopy-recfirst-test.sh [--selftest]
BOOTCOPY_KIND=recfirst exec bash "$(dirname "$0")/pc-bootcopy-test.sh" "$@"
