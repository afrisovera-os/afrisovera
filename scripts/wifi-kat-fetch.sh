#!/usr/bin/env bash
# W1, Wi-Fi crypto: download the published test-vector files, check each against the size and sha256 pinned below,
# and run scripts/wifi-kat-parse.py on them to write:
#   wlan-crypto/fixtures/kat.txt      the host test's vectors (wlan-crypto/tests/kat.rs)
#   wlan-crypto/fixtures/SOURCE.txt   where each file came from, its size and sha256
#   kernel/src/wlan/crypto_kat.rs     the same vectors as Rust constants, for `wifi selftest`
# The parser reads the bytes straight from the downloaded files; nothing is typed by hand (architect, 14 Nov 2026).
# A moved or changed file fails here on its pinned hash, instead of quietly giving other vectors.
# hostapd's file is BSD-licensed; only the IEEE 802.11 test-vector data in it is used (no code).
#
#   scripts/wifi-kat-fetch.sh            write the three files
#   scripts/wifi-kat-fetch.sh --check    download and parse again into a temporary folder, and fail unless the
#                                        result is byte-identical to the committed files (suite wifi-crypto)
# Needs network, so it runs in the cloud job or CI, not on the shared PC.
set -euo pipefail
cd "$(dirname "$0")/.."
MODE="${1:-write}"
DL="$(mktemp -d)"
if [ "$MODE" = "--check" ]; then
  DEST="$(mktemp -d)"
else
  DEST="."
fi
mkdir -p "$DEST/wlan-crypto/fixtures" "$DEST/kernel/src/wlan"
HOSTAP=6d3d778451a9329085eab5777386db0b78d56987   # hostap.git HEAD on 14 Nov 2026
# name  url  bytes  sha256
FILES="
rfc2202.txt https://www.rfc-editor.org/rfc/rfc2202.txt 11942 c19effeca47e801be304460da3b2fb05f595e2309abceb050e40e024868fe483
rfc6070.txt https://www.rfc-editor.org/rfc/rfc6070.txt 7334 e8cb0810032ad90f0fbd2001e7a42a079b6551064225e12b2a1ad4a7cb9105ea
rfc3394.txt https://www.rfc-editor.org/rfc/rfc3394.txt 73072 faa400c69c22e5f4a911222dc19c17091931a4b8c0658adc7011163e63b253ff
rfc4493.txt https://www.rfc-editor.org/rfc/rfc4493.txt 38751 252850ac038a44e931fdce6cb02b444a18ed1da0c9e212bfdd1da0ed3366fc1c
rfc3610.txt https://www.rfc-editor.org/rfc/rfc3610.txt 64509 d6c857b1dea18d8259ee0ccc5468859990200e8580d0ad55dc75963d0c46588d
crypto_module_tests.c https://w1.fi/cgit/hostap/plain/src/crypto/crypto_module_tests.c?id=HOSTAP 82924 40b0183c8812863fcb89a50e013a4b3ece7fd84608bd68bd18dca8335747d2d4
"
SRC="$DEST/wlan-crypto/fixtures/SOURCE.txt"
{
  echo "# The published files the Wi-Fi crypto vectors were parsed from, by scripts/wifi-kat-fetch.sh."
  echo "# name url bytes sha256"
} > "$SRC"
while read -r name url size sha; do
  [ -n "$name" ] || continue
  url="${url/HOSTAP/$HOSTAP}"
  curl -fsSL --retry 3 --max-time 120 "$url" -o "$DL/$name"
  got_size=$(wc -c < "$DL/$name" | tr -d ' ')
  got_sha=$(sha256sum "$DL/$name" | cut -d' ' -f1)
  echo "$name  $got_size bytes  sha256 $got_sha  $url"
  if [ "$got_size" != "$size" ] || [ "$got_sha" != "$sha" ]; then
    echo "ERROR: $name is not the pinned file (want $size bytes, sha256 $sha)" >&2
    exit 2
  fi
  echo "$name $url $got_size $got_sha" >> "$SRC"
done <<< "$FILES"
python3 -I scripts/wifi-kat-parse.py "$DL" --fixture "$DEST/wlan-crypto/fixtures/kat.txt" \
        --rust "$DEST/kernel/src/wlan/crypto_kat.rs" --sources "$SRC"
rm -rf "$DL"
if [ "$MODE" = "--check" ]; then
  rc=0
  for f in wlan-crypto/fixtures/kat.txt wlan-crypto/fixtures/SOURCE.txt kernel/src/wlan/crypto_kat.rs; do
    if cmp -s "$f" "$DEST/$f"; then echo "same as a fresh parse: $f"; else echo "DIFFERS from a fresh parse: $f"; rc=1; fi
  done
  rm -rf "$DEST"
  exit $rc
fi
echo "wifi-kat-fetch: wrote wlan-crypto/fixtures/kat.txt, wlan-crypto/fixtures/SOURCE.txt and kernel/src/wlan/crypto_kat.rs"
