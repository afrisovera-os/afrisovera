#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): Windows and Ubuntu sharing ONE EFI partition with too little room left for
# AfriSovera's boot file, on an NVMe disk of 4096-byte blocks. The same boot as scripts/pc-twoesp-full-test.sh (read
# that file): bootplan plans nothing, bootcopy and bootentry refuse, and the disk stays byte for byte as it was.
#   scripts/pc-twoesp-full4k-test.sh [--selftest]
TWOESPFULL_KIND=dual4k exec bash "$(dirname "$0")/pc-twoesp-full-test.sh" "$@"
