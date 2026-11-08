#!/usr/bin/env bash
# Phase 2.881 - the SELECTION half of a marker-keyed reaper.
#
# WHY THIS EXISTS: run-all-tests.sh:361 kills by IMAGE NAME (taskkill //F //IM),
# which reaps every qemu on the box. 2.879/2.880 established that a per-walk
# marker can be minted into a guest's argv (-name afrish-walk-<id>). But
# `taskkill` CANNOT filter on a command line - its only filters are IMAGENAME,
# WINDOWTITLE, PID, SESSION, STATUS, USERNAME, CPUTIME, MEMUSAGE, MODULES,
# SERVICES (measured: taskkill /? on this host). So the marker has to be turned
# into a PID LIST first, via CIM. That is this script, and ONLY that: it prints
# PIDs and NEVER kills anything.
#
# Usage:  qemu-select-2881.sh <marker>
# Exit:   0  = at least one qemu carries the marker (PIDs printed as "MATCH <pid>")
#        10  = zero matched (fails CLOSED - a caller that kills nothing is safe)
#         2  = REFUSED: the marker is missing or not of the shape
#              afrish-walk-<token>, so it could never be trusted to discriminate.
set -u

MARKER="${1:-}"

if [ -z "$MARKER" ] || ! printf '%s' "$MARKER" | grep -Eq '^afrish-walk-[A-Za-z0-9._-]+$'; then
  echo "REFUSED: marker '$MARKER' is empty or not of the form afrish-walk-<token>."
  echo "  A loose marker would select guests it does not own - that is the //IM"
  echo "  blast radius this script exists to remove. Selecting NOTHING."
  exit 2
fi

PS1_TMP="$(mktemp -t qsel2881.XXXXXX).ps1"
trap 'rm -f "$PS1_TMP"' EXIT
cat > "$PS1_TMP" <<'PSEOF'
param([string]$Marker)
$procs = @(Get-CimInstance Win32_Process -Filter "Name LIKE 'qemu-system-aarch64%'")
Write-Output ("TOTAL " + $procs.Count)
foreach ($p in $procs) {
  $cl = $p.CommandLine
  if ($null -ne $cl -and $cl.Contains($Marker)) {
    Write-Output ("MATCH " + $p.ProcessId)
  } else {
    Write-Output ("SKIP " + $p.ProcessId)
  }
}
PSEOF

OUT="$(powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(cygpath -w "$PS1_TMP" 2>/dev/null || echo "$PS1_TMP")" -Marker "$MARKER" 2>/dev/null | tr -d '\r')"

echo "$OUT"
NMATCH="$(printf '%s\n' "$OUT" | grep -c '^MATCH ')"
NSKIP="$(printf '%s\n' "$OUT" | grep -c '^SKIP ')"
echo "marker=$MARKER matched=$NMATCH unmarked-qemu-left-alone=$NSKIP"
[ "$NMATCH" -gt 0 ] && exit 0
echo "NO-MATCH: nothing carries this marker; a reaper keyed on it is a no-op here."
exit 10
