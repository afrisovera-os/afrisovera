#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): Windows and Ubuntu sharing ONE EFI partition (\EFI holds Microsoft, BOOT, ubuntu) on an
# NVMe disk of 4096-byte blocks, a blank disk on SATA port 2. The same five boots as scripts/pc-twoesp-test.sh (read
# that file): bootplan lists all three folders, bootcopy puts AfriSovera's file beside them and takes it out again, and
# the Microsoft, BOOT and ubuntu files stay byte for byte as they were.
#   scripts/pc-twoesp-dual4k-test.sh [--selftest]
TWOESP_KIND=dual4k exec bash "$(dirname "$0")/pc-twoesp-test.sh" "$@"
