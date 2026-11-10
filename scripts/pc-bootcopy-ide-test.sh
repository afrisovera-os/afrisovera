#!/usr/bin/env bash
# PC track step 5 (10 Nov 2026): bootcopy on a SATA disk in IDE mode (ide1, QEMU's piix3-ide; ide0 is an empty FAT disk for the firmware's NvVars): the same four boots
# as scripts/pc-bootcopy-test.sh (read that file), plus lside in boot 2 for the log.
#   scripts/pc-bootcopy-ide-test.sh [--selftest]
BOOTCOPY_KIND=ide exec bash "$(dirname "$0")/pc-bootcopy-test.sh" "$@"
