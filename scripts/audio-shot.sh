#!/usr/bin/env bash
# Boot Mansa with a virtio-sound device attached and drive the shell over TCP,
# so the virtio bus scan + audio negotiation can be verified. The `wav` audiodev
# captures any played PCM to a file (used by the streaming milestone 2.21); for
# now the device just needs to be present so the driver can negotiate with it.
# Needs qemu + python3.
set -uo pipefail

OUT="${1:-audio.png}"
shift || true
WAV="${WAV:-audio-out.wav}"
QMP_PORT=4455
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

CMDS=("$@")
if [ ${#CMDS[@]} -eq 0 ]; then
    CMDS=(devices audio)
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false \
    -device ramfb -display none \
    -audiodev "wav,id=snd0,path=$WAV" \
    -device virtio-sound-device,audiodev=snd0 \
    -serial tcp:127.0.0.1:4460,server,nowait \
    -qmp tcp:127.0.0.1:4455,server,nowait -kernel "$BIN" &
QPID=$!

sleep 5
python3 scripts/shell-demo.py 4460 4455 "$OUT" "${CMDS[@]}"
RC=$?

# 3.266: WAIT FOR THE PAYLOAD TO STOP GROWING before killing QEMU.
# The driver returns as soon as the guest's PROMPT comes back, but the wav
# audiodev is still draining the utterance to disk -- so the old unconditional
# kill here cut the capture at whatever byte the scheduler happened to be on.
# MEASURED at 3.265 on the rewired (voice::say) path: three boots of the SAME
# command gave 8236 / 192044 / 69676 bytes, and the probe's in-run CONTROL
# refused a verdict (RIG-NOISY) because of it. This is a QUIESCENCE wait, not
# a constant sleep (the 2.783 lesson): it returns as soon as the file has been
# unchanged for AUDIO_QUIET seconds, so a boot that plays nothing costs the
# quiet window and nothing more. The ceiling is a backstop, never the plan.
AUDIO_QUIET="${AUDIO_QUIET:-2.0}"
AUDIO_WAIT_MAX="${AUDIO_WAIT_MAX:-30}"
wav_size() { [ -f "$WAV" ] && wc -c < "$WAV" 2>/dev/null || echo 0; }
if [ "$RC" -eq 0 ]; then
    quiet_polls=$(python3 -c "print(max(1,int(round($AUDIO_QUIET/0.25))))")
    max_polls=$(python3 -c "print(max(1,int(round($AUDIO_WAIT_MAX/0.25))))")
    last=$(wav_size); stable=0; polls=0
    while [ "$polls" -lt "$max_polls" ]; do
        sleep 0.25
        polls=$((polls + 1))
        now=$(wav_size)
        if [ "$now" = "$last" ]; then
            stable=$((stable + 1))
            [ "$stable" -ge "$quiet_polls" ] && break
        else
            stable=0
            last="$now"
        fi
    done
    if [ "$stable" -ge "$quiet_polls" ]; then
        echo "[audio-shot] wav settled at $last bytes after $((polls * 25 / 100)).$((polls * 25 % 100))s"
    else
        echo "[audio-shot] WARN wav still growing at $AUDIO_WAIT_MAX s ceiling ($last bytes) -- capture may be truncated"
    fi

    # 3.266, MEASURED not assumed: SETTLED IS NOT COMPLETE. With the waiter
    # above alone, three boots settled at 65536 / 114688 / 188416 bytes -- every
    # one a multiple of 4096, i.e. the wav backend's STDIO BUFFER, not the
    # utterance -- and all three files carried riffsz=0 datasz=0, the
    # placeholder header qemu writes at open and only rewrites when the backend
    # is CLOSED. `kill` never lets that happen, so the size a poller sees is
    # whichever 4 KiB boundary the flush last crossed. Shut qemu down through
    # QMP instead: a clean exit drains the backend's buffer, and THAT is what
    # made three boots come back 192044 / 192044 / 192044 with a byte-identical
    # control.
    #
    # HONEST GAP, MEASURED not assumed: this loop was WRITTEN to wait for the
    # header to be rewritten, on the theory that datasz>0 is a direct
    # measurement of "the capture is whole". IT NEVER FIRES -- all three
    # complete 192044-byte captures still carry riffsz=0 datasz=0, so this
    # qemu's wav backend does not seek back over its placeholder header even on
    # a clean quit. What the loop actually buys is a BOUNDED DRAIN WAIT after
    # the quit (it runs to its 10 s ceiling every time), which is load-bearing;
    # the completeness signal it was named for does not exist here. Tightening
    # it to a post-quit quiescence check is owed, and must be re-measured
    # against three boots, not reasoned about.
    python3 - "$QMP_PORT" <<'PY' || echo "[audio-shot] WARN qmp quit failed; falling back to kill"
import json, socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=10)
s.settimeout(10)
f = s.makefile("rwb")
f.readline()                                   # greeting
for cmd in ('{"execute":"qmp_capabilities"}', '{"execute":"quit"}'):
    f.write((cmd + "\n").encode()); f.flush()
    f.readline()
PY

    hdr_polls=0
    while [ "$hdr_polls" -lt 40 ]; do
        if python3 -c "
import struct,sys
d=open(sys.argv[1],'rb').read(44)
sys.exit(0 if len(d)==44 and struct.unpack('<I',d[40:44])[0]>0 else 1)" "$WAV" 2>/dev/null; then
            break
        fi
        sleep 0.25
        hdr_polls=$((hdr_polls + 1))
    done
    echo "[audio-shot] wav final: $(wav_size) bytes, header $(python3 -c "
import struct,sys
d=open(sys.argv[1],'rb').read(44)
print('datasz=%d' % struct.unpack('<I',d[40:44])[0] if len(d)==44 else 'SHORT')" "$WAV" 2>/dev/null || echo ABSENT)"
fi

kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "wrote $OUT (rc=$RC)"
exit $RC
