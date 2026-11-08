#!/usr/bin/env bash
# Phase 2.873 - the MINTED-PID reaper: kill by the pid the shim wrote, never by a
# marker sweep, and DELETE every file it consumes.
#
# 2.872 made the PATH shim mint each guest's Windows pid into $QEMU_PIDS_DIR/pids/
# (file NAME == file CONTENT == the guest's winpid), and deliberately left the file
# behind whenever the shim took its TERM/INT/HUP trap - i.e. exactly when a guest
# may have outlived its shim.  That leftover set is the suspect list.  Until it is
# consumed it only GROWS, and a Windows pid that has since been recycled is a live
# blast-radius hazard (2.872 honest gap (b)).  This script is the consumer.
#
# Against scripts/qemu-reap-2886.sh (the marker reaper) the difference is COST and
# BLAST RADIUS, both in the same direction:
#   - cost: the marker reaper pays a qemu-select-2881.sh launch (a machine-wide
#     command-line query) even when there is nothing to kill.  This reaper's empty
#     case is a directory listing: zero taskkill, zero query.
#   - radius: it can only ever consider pids THIS walk's own shim minted.  A marked
#     guest whose shim died between spawn and mint is invisible to it.  That is the
#     trade, and it is why the caller keeps the marker reaper for the walk's final
#     sweep rather than deleting it.
#
# THE RECYCLE GUARD: a leftover file is a pid that MIGHT be dead, and a dead pid may
# have been handed to something else.  So every pid is checked against tasklist and
# killed ONLY if its image name matches (default qemu-system-aarch64.exe, override
# with REAPPIDS_IMAGE).  A pid whose image does not match is REFUSED, not killed -
# and its file is still deleted, because a stale entry we refuse forever is the
# accumulation this step exists to end.
#
# Usage:  qemu-reap-pids-2873.sh <pids-dir>
# Exit:   0 = every file consumed (killed, or refused-and-dropped); no survivors
#         3 = no taskkill/tasklist on this host (nothing consumed)
#         4 = at least one pid we killed is still alive afterwards
set -u

DIR="${1:-}"
IMAGE="${REAPPIDS_IMAGE:-qemu-system-aarch64.exe}"

[ -n "$DIR" ] || { echo "REAPPIDS: usage: $0 <pids-dir>"; exit 3; }
if [ ! -d "$DIR" ]; then
  echo "REAPPIDS NO-DIR: $DIR does not exist; killing nothing."
  exit 0
fi

FILES=""
for f in "$DIR"/*; do [ -f "$f" ] && FILES="$FILES $f"; done
if [ -z "$FILES" ]; then
  echo "REAPPIDS NO-OP: 0 minted pids outstanding in $DIR."
  exit 0
fi

command -v taskkill >/dev/null 2>&1 || { echo "REAPPIDS: no taskkill on this host"; exit 3; }
command -v tasklist >/dev/null 2>&1 || { echo "REAPPIDS: no tasklist on this host"; exit 3; }

# image_of <pid> -> prints the image name, or nothing when the pid is not running.
image_of() {
  tasklist //FI "PID eq $1" //FO CSV //NH 2>/dev/null |
    sed -n 's/^"\([^"]*\)".*/\1/p' | head -1
}

KILLED=0; REFUSED=0; DROPPED=0; LEFT=0; KILLEDPIDS=""
for f in $FILES; do
  base="$(basename "$f")"
  body="$(tr -d '\r\n' < "$f" 2>/dev/null)"
  # name == content is the shim's own invariant (2.872 ARM1). A file that breaks it
  # was not written by the shim we trust, so it is dropped unread, never killed.
  if ! printf '%s' "$base" | grep -q '^[0-9][0-9]*$' || [ "$base" != "$body" ]; then
    echo "REAPPIDS dropped malformed file '$base' (content='$body')"
    rm -f "$f"; DROPPED=$((DROPPED + 1)); continue
  fi
  img="$(image_of "$base")"
  if [ -z "$img" ]; then
    echo "REAPPIDS pid $base already gone (no such task)"
    rm -f "$f"; DROPPED=$((DROPPED + 1)); continue
  fi
  if [ "$img" != "$IMAGE" ]; then
    echo "REAPPIDS REFUSED pid $base: image '$img' is not '$IMAGE' (recycled pid); file dropped"
    rm -f "$f"; REFUSED=$((REFUSED + 1)); continue
  fi
  if taskkill //F //PID "$base" >/dev/null 2>&1; then
    echo "REAPPIDS killed pid $base ($img)"; KILLED=$((KILLED + 1)); KILLEDPIDS="$KILLEDPIDS $base"
  else
    echo "REAPPIDS taskkill failed on pid $base (already gone?)"
  fi
  rm -f "$f"
done

# Survivor check is per-pid tasklist, not a machine-wide query: it is only asked
# about pids we actually killed, so the common (killed=0) case costs nothing.
if [ -n "$KILLEDPIDS" ]; then
  sleep 1
  for pid in $KILLEDPIDS; do
    [ -n "$(image_of "$pid")" ] && { echo "REAPPIDS SURVIVOR pid $pid"; LEFT=$((LEFT + 1)); }
  done
fi

echo "REAPPIDS dir=$DIR killed=$KILLED refused=$REFUSED dropped=$DROPPED survived=$LEFT"
[ "$LEFT" = 0 ] || exit 4
exit 0
