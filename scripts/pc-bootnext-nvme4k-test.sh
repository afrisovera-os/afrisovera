#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) from the EFI partition of a Windows-like
# NVMe disk with 4096-byte blocks, as many laptop SSDs have. The same six boots as scripts/pc-bootnext-test.sh (read that
# file); the entry the firmware starts must say HD(1,GPT,...,start 256), counted in the disk's own 4 KiB blocks.
#   scripts/pc-bootnext-nvme4k-test.sh [--selftest]
BOOTNEXT_KIND=nvme4k exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
