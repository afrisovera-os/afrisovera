#!/usr/bin/env bash
# PC track step 5 (10 Nov 2026): bootplan on SATA disks in IDE mode (QEMU's piix3-ide; ide0 is an empty FAT disk for the
# firmware's NvVars): the same two boots as scripts/pc-bootplan-test.sh (read that file), the disks named ide1, ide2, ide3.
#   scripts/pc-bootplan-ide-test.sh [--selftest]
BOOTPLAN_KIND=ide exec bash "$(dirname "$0")/pc-bootplan-test.sh" "$@"
