#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): the firmware starting AfriSovera once (BootNext) from a SATA disk whose ONE EFI
# partition is shared by Windows and Ubuntu, as an Ubuntu installed after Windows leaves it. The same six boots as
# scripts/pc-bootnext-test.sh (read that file); the firmware must start AfriSovera's own file, not Ubuntu's, and the
# Microsoft, BOOT and ubuntu files must stay byte for byte as they were.
#   scripts/pc-bootnext-dual-test.sh [--selftest]
BOOTNEXT_KIND=dual exec bash "$(dirname "$0")/pc-bootnext-test.sh" "$@"
