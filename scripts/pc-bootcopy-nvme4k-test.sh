#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): bootcopy onto the EFI partition of a Windows-like NVMe disk with 4096-byte blocks, as
# many laptop SSDs have. The same four boots as scripts/pc-bootcopy-test.sh (read that file); the EFI partition is at
# block 256 and 260 MiB long, its FAT32 in 4 KiB sectors, and lsnvme in boot 2 must count the copy as big copies.
#   scripts/pc-bootcopy-nvme4k-test.sh [--selftest]
BOOTCOPY_KIND=nvme4k exec bash "$(dirname "$0")/pc-bootcopy-test.sh" "$@"
