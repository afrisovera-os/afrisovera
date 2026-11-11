#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): installdisk on an NVMe disk with 4096-byte blocks, as many laptop SSDs have. The same
# three boots as scripts/pc-installdisk-test.sh (read that file); the GPT and FAT32 count in 4 KiB blocks, and the
# boot file's big copies go to the disk in whole blocks, several sectors to a command (table_write_run in nvme.rs).
#   scripts/pc-installdisk-nvme4k-test.sh [--selftest]
INSTALLDISK_KIND=nvme4k exec bash "$(dirname "$0")/pc-installdisk-test.sh" "$@"
