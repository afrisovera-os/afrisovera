#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) from a Windows-like NVMe disk of
# 4096-byte blocks whose recovery partition comes BEFORE the EFI partition. The same six boots as
# scripts/pc-bootnext-test.sh (read that file); bootcopy must copy to nvme partition 2, and the entry the firmware
# starts must say HD(2,GPT,<entry 2's GUID>,start 5376), counted in the disk's own 4 KiB blocks.
#   scripts/pc-bootnext-recfirst4k-test.sh [--selftest]
BOOTNEXT_KIND=recfirst4k exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
