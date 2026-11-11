#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) from a 4 KiB-block NVMe disk whose
# ONE EFI partition is shared by Windows and Ubuntu. The same six boots as scripts/pc-bootnext-test.sh (read that
# file); the entry the firmware starts must say HD(1,GPT,...,start 256) and name AfriSovera's own file, and the
# Microsoft, BOOT and ubuntu files must stay byte for byte as they were.
#   scripts/pc-bootnext-dual4k-test.sh [--selftest]
BOOTNEXT_KIND=dual4k exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
