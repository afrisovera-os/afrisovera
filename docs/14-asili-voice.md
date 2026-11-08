# 14 — Asili Voice: On-Device Text-to-Speech & Notifications

> The goal: AfriSovera **speaks** — a real, human-sounding voice for notifications
> ("your battery is low, please charge," "you have new mail"), not a beep. This doc
> is the honest architecture: how we get there with an **open, on-device** voice
> that is ours to ship — not Microsoft's engine, not the cloud — and what is real
> today vs. what still has to be built.

## The honest headline

- We do **not** train a voice model from scratch (that needs data + compute we
  don't have) and we do **not** use or "train from" Microsoft's Read Aloud (it's a
  closed engine, and not trainable material).
- We **do** take an **existing open, permissively-licensed neural TTS** and run it
  **on-device** — the same "stand on open models" rule as the rest of Asili AI
  ([`09`](09-asili-ai.md)). Best fit: **Piper** (MIT, tiny, fast, runs on a Raspberry Pi),
  with **eSpeak-NG** as a lightweight fallback and Coqui/VITS for higher quality on
  capable hardware.
- Voice is a **capability** (`CAP_VOICE`): an app calls `speak(text)` only if it
  holds the voice capability — so nothing speaks (or records) without permission.

## Architecture

```
  app / system service ── speak("battery low") ──▶ [capability gate: CAP_VOICE]
                                                          │
                                                          ▼
                                            Asili Voice service (on-device)
                                             • open TTS model (Piper) -> PCM
                                             • hardware-tiered voice quality
                                                          │
                                                          ▼
                                            Audio driver (Asili) -> speaker
```

- **`speak(text)` host function** — capability-gated, reads text from the app's
  memory. **Done** (see `nia.rs`) — it prints `[asili voice] (would speak aloud):
  "<text>"` AND, since 3.264, synthesizes that text through `crate::voice::say`.
  (The serial line is a legacy of the placeholder era; it is kept because all 687
  green suites grade it, and it is NOT evidence about the audio — 3.254.)
- **On-device speech synthesis** — **two layers, and only the upper one is missing.**
  `kernel/src/voice.rs` (`pub fn say(text: &[u8])`, Phase 2.24) is a **real
  text-dependent formant synthesizer** — glottal pulse train + biquad formant
  resonators, sonorant/fricative/plosive classes, coarticulation glides, integer
  DSP, no model — streaming through `virtio::sound_play_pcm()`. **Done.** An open
  neural model (Piper) producing natural words over the SAME path is still **to do**.
  **Corrected at tick 3.260 (measured from call sites): `voice::say` has two live
  callers — the EL0 speak syscall (`process.rs:897`) and the `say` shell command
  (`shell.rs:1299`) — but `HOST_SPEAK`, the WASM `speak(text)` this document is
  about, calls `virtio::sound_play_tone()` instead, which takes NO argument. So a
  Nia app's `speak()` is TEXT-INDEPENDENT by construction** (finding 3.260).
  **REPAIRED at tick 3.264 (kernel commit `3ecd00c`): `HOST_SPEAK` calls
  `crate::voice::say(&tb[..tn])`, and the same unedited two-text probe flipped
  10 → 0. The paragraph above is kept as the record of what was true before it.**
  **UNBOUNDED at tick 3.370 (design-3.367 (B), commit `5d52457`): `say()` used to
  fill a fixed whole-utterance buffer (`MAXLEN = 48000` samples ≈ 1.0884 s) and
  BREAK out of its character loop when the buffer filled — silently until 3.366.
  Measured consequence (finding 3.365): a 44-char notification was spoken as its
  first 10 characters, and every "complete" capture in this chain was exactly
  `MAXLEN * 4` = 192000 bytes because the ceiling, not the text, set the length.
  `say()` now STREAMS: `UTTER` is a per-character staging buffer (`MAXLEN` 8192,
  `voice.rs:33`) handed to `virtio::sound_stream_begin`/`_write`/`_end`
  (`virtio.rs:649-703`), which carries a partial period across writes so the caller
  never sees `PERIOD_BYTES`. The ceiling is gone BY CONSTRUCTION — there is no
  `n >= MAXLEN` break — and static `.bss` drops by **79616 B**. MEASURED: the same
  unedited `scripts/longspeak-probe-3369.sh` flipped exit 10
  `UTTERANCE-CEILING-HELD` → exit 0 `UTTERANCE-COMPLETE`, `pcm=2347052` for a
  124-char sentence (12.2× the old ceiling) with no `TRUNCATED` line.
  `sound_play_pcm` is now that same streaming API called once — ONE submit path.
  NOT bought: underrun (synthesis is now concurrent with playback) did not fire on
  one idle-host TCG boot, which is not evidence it cannot; RTF 0.104 is one sample.**
- **Audio driver** — **Done (v0).** `kernel/src/virtio.rs` drives the `virtio-sound`
  device: bus enumeration, feature negotiation, split virtqueues, and the PCM
  control sequence (`SET_PARAMS`/`PREPARE`/`START`) + tx streaming. A real ~441 Hz
  tone was verified by capturing 372 ms of non-silent PCM through QEMU's `wav`
  audiodev (see docs/07 milestone 2.21). **Corrected at tick 3.254 (measured, not
  inferred): that tone is emitted by the `audio` SHELL COMMAND, not by `speak()`.**
  `crate::virtio::sound_play_tone()` has exactly ONE caller in the whole tree —
  `kernel/src/shell.rs:1091` — and `HOST_SPEAK` (`kernel/src/nia.rs:2426-2439`)
  only `kprintln!`s `[asili voice] (would speak aloud): "<text>"`. **`speak()`
  makes no sound at all today**; the driver and the host function are both done but
  they have never been wired to each other. **That sentence was true when written
  and is FALSE NOW: the wire-up landed at 3.256 and was re-pointed at the
  text-dependent synthesizer at 3.264 — kept above as the record, not as the state.**
  Boot with `-device virtio-sound-device,audiodev=…` (see `scripts/audio-shot.sh`).
- **Privacy** — TTS runs on-device; text never leaves the device (same guarantee as
  the AI capability). STT (speech-to-text) is the mirror capability, also on-device.

## Notifications (the "human voice reminders")

System events become spoken notifications through the same path:

| Event | Spoken |
|---|---|
| Battery low | "Your battery is low — please charge your device." |
| New mail | "You have new mail." |
| Update ready | "A system update is ready to install." |
| Timer/alarm | the user's label, read aloud |

A **Notification service** (AfriPush-adjacent) turns events into `speak()` calls,
respecting a Do-Not-Disturb policy and the user's chosen voice/language. Because
it's on-device, it works **offline** and privately — vital for the target markets.

## Current state / what's left

- ✅ `speak(text)` capability-gated host function + a demo (`notify.wasm` speaks a
  battery warning). The **API and capability model are real.**
- ✅ Audio driver (virtio-sound) — **real sound output.** `speak()` streams PCM to
  the device; verified 440 Hz / 372 ms captured to WAV (docs/07 milestone 2.21).
- ✅ On-device **formant synthesizer** (`kernel/src/voice.rs`, no model, integer DSP)
  — real text-dependent speech, live on the EL0 syscall and the `say` shell command.
- ✅ **The WASM `speak()` host function is wired to it** (tick 3.264, kernel commit
  `3ecd00c`): `HOST_SPEAK` now calls `crate::voice::say(&tb[..tn])` — the exact call
  `process.rs:897` already makes — instead of the argument-less `sound_play_tone()`
  that made finding 3.260's text-independence true by construction. MEASURED, not
  inferred: `scripts/speaktext-probe-3263.sh`, unedited across the patch, flipped
  exit 10 `SPEAK-TEXT-INDEPENDENT` → exit 0 `SPEAK-TEXT-DEPENDENT`, grading the PCM
  PAYLOADS of two boots whose texts are the only difference. **NOT claimed: the
  `[asili voice]` serial line says nothing about the audio (it is the line that
  fooled this document at 3.254 and is identical on both sides), and this buys
  FORMANT speech, not Piper — the box below stays open.**
- ✅ **The utterance is UNBOUNDED** (tick 3.370, commit `5d52457`, design-3.367 (B)).
  `say()` streams period-by-period instead of filling a fixed 48000-sample buffer, so
  both `speak()` (WASM) and the `say` shell command speak the WHOLE text at any
  length; static `.bss` fell by 79616 B in the same change. MEASURED by the unedited
  `scripts/longspeak-probe-3369.sh` flipping 10 → 0 (`pcm=2347052`, 12.2× the old
  192000-byte ceiling, no `TRUNCATED` line). Before it, a 44-char notification was
  audibly its first 10 characters (finding 3.365) and said so only since 3.366.
- ⬜ On-device TTS **model** (Piper) producing natural words over the same path.
- ⬜ Notification service mapping system events → speech; voice/language selection;
  STT (the input mirror).

**Honest bottom line:** the *plumbing, permission model, and audio output* for voice
are in place today — `speak()` makes real sound, text-dependent, at any length.
What's left for a real human-sounding voice is the **TTS model** (Piper) that turns
words into PCM instead of the current *formant synthesizer* (not a tone: the
placeholder-tone wording was corrected at 3.260/3.370), plus the inference runtime. We build it from open models, on-device,
sovereign — never Microsoft's engine, never the cloud.
