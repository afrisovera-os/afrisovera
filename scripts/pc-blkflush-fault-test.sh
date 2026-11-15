#!/usr/bin/env bash
# AFR-0011 V2: pc-blkflush's fault cases (nvme-werr: QEMU's blkdebug fails the writes, so blkflush answers failed
# then ok; and the rig's 30 s hang on NVMe, IDE and SATA). The same script as pc-blkflush, split off so each suite
# stays inside the runner's 300 s.
exec env BLKFLUSH_SET=fault "$(dirname "$0")/pc-blkflush-test.sh" "$@"
