#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootcopy onto the EFI partition of a Windows-like NVMe disk (piix3-ide holds an empty
# FAT disk for the firmware's NvVars; the blank GPT disk is on SATA port 2): the same four boots as
# scripts/pc-bootcopy-test.sh (read that file), plus lsnvme in boot 2, which must count the copy as big copies.
#   scripts/pc-bootcopy-nvme-test.sh [--selftest]
BOOTCOPY_KIND=nvme exec bash "$(dirname "$0")/pc-bootcopy-test.sh" "$@"
