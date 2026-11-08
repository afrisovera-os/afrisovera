# 09 — Asili AI: AI as an OS Service (the AI-era pillar)

> The last decade's OSes were built before on-device AI mattered. AfriSovera is
> being built *now*, so AI is a **first-class OS capability**, not an app you
> download. This doc gives the honest architecture — what's real on a $40 phone,
> what needs the cloud, and why doing it at the OS layer (capability-gated,
> on-device, private) is a genuine advantage no incumbent can easily copy.

## The thesis in one paragraph

**Asili AI is an OS-level inference service.** Apps don't ship their own models or
call a foreign cloud; they request an **AI capability** (translate, transcribe,
speak, assist, embed) the same way they request payments or the network. The model
runs in a **sandboxed system service**, **on-device and private by default**, and
the app only ever sees the result. This makes AI *safe* (capability-gated), *cheap*
(one shared runtime, not N copies), *private* (data doesn't leave the phone), and
*sovereign* (no dependency on a foreign AI mothership).

## Why at the OS layer (not per-app)

| Per-app AI (today) | Asili AI (OS service) |
|---|---|
| Every app bundles its own model → wasted RAM/storage on a 1–2 GB phone | One shared, memory-mapped model runtime |
| Every app phones home → privacy loss, data leaves the country | On-device by default; cloud only with explicit per-request consent |
| Permissions are all-or-nothing | A specific, revocable **AI capability** per app |
| No system-wide assistant/voice/translation | OS-wide voice, translation, TTS/STT, assist |

Capability security ([`01`](01-kernel-mansa.md)) is exactly what makes this trustworthy: an app with a
"translate" capability **cannot** read your files or reach the network to exfiltrate
them — the kernel physically prevents it.

## Architecture

```
   app (Nia/WASM, Web, or bridged)
      │  requests an AI capability (e.g. "transcribe", "assist")
      ▼
  ┌──────────────────────────────────────────────┐
  │  Asili AI service (userspace, capability-gated)│
  │   • model registry (quantized, signed)         │
  │   • inference runtime (CPU / NPU / GPU backend) │
  │   • policy: on-device first, cloud on consent   │
  └───────────────┬──────────────────┬─────────────┘
                  │ on-device         │ optional, consented
                  ▼                   ▼
         NPU/GPU/CPU via Asili   Sovereign cloud endpoint
         driver (Vulkan compute) (self-hostable by a partner)
```

- **Inference runtime**: executes quantized models on whatever accelerator the
  device has — CPU (always), GPU via Vulkan compute (Tonga shares the stack), or a
  vendor **NPU** exposed through an Asili driver with a stable ABI.
- **Model registry**: models are **signed, versioned artifacts** (like apps in
  AfriStore), so what runs is verifiable. Small models ship with the OS; larger
  ones install on capable devices.
- **Policy engine**: on-device first; falls back to a **self-hostable sovereign
  cloud** (run by a telco/state/university, not us) **only with explicit,
  per-request user consent** — never silently.

## Right-sizing to real hardware (the honest part)

We will **not** pretend a $40 / 1–2 GB phone runs a frontier LLM. What is real:

| Capability | Feasible on-device (cheap phone) | Needs better HW / cloud |
|---|---|---|
| **Speech-to-text / Text-to-speech** | ✅ small models — huge for low-literacy users | higher accuracy → larger models |
| **Translation** (local languages first) | ✅ compact NMT models | rare language pairs |
| **On-device assist / autocomplete / classification** | ✅ sub-1B quantized models | long-context reasoning |
| **Embeddings / semantic search** | ✅ small encoders | — |
| **Large general chat / codegen** | ⚠️ tier-gated on capable devices | ✅ consented sovereign cloud |

The design center: **STT + TTS + translation on-device** is already transformative
for our target users, and it's achievable today. Everything heavier is tiered by
device capability or offloaded with consent — the same honest tiering as the app
compatibility story ([`06`](06-universal-app-compatibility.md)).

## Built from open models — we don't train from scratch

Asili AI is assembled from the open-source AI ecosystem, not trained from zero
(the same "stand on giants" rule as the rest of the stack — [`04`](04-the-toolchain.md)):

- **Models:** take permissively-licensed open models — compact speech-to-text
  (Whisper-tiny class), small open TTS, compact open translation (NMT), and a
  sub-1B open LLM — and **quantize** them (int4/int8) into signed model artifacts.
- **Runtime:** a lightweight inference engine (ggml/llama.cpp-class, MIT-licensed,
  or ONNX Runtime) compiled for the device; our contribution is the **capability
  integration, sandboxing, model registry, and hardware tiering**, not reinventing
  the math.
- **Hardware-adaptive:** at load time the runtime probes the device (CPU features,
  Vulkan-capable GPU, vendor NPU via an Asili driver) and selects the **backend +
  model size tier** that fits — tiny quantized models on a $40 phone, larger ones
  on capable hardware. Same API, performance scaled to the silicon.

This is buildable now: open models + open runtime + our OS-level packaging and
capability layer.

## What developers get

- **AI SDK**: one API (`ai.transcribe`, `ai.translate`, `ai.assist`, `ai.embed`)
  across every AfriSovera device — write once, runs on whatever accelerator exists.
- **AI-native shell**: natural language → capability-scoped commands, **shown and
  explained before they run** (never executed blindly).
- **On-device coding assistant** and **AI-assisted app scaffolding** — the phone as
  a self-sufficient dev machine (the emerging-market reality from [`03`](03-app-runtime-and-sdk.md)).

## What users get

- OS-wide **voice interface**, **translation**, **read-aloud** — accessibility and
  low-literacy support as a platform feature, not a per-app afterthought.
- **Privacy by construction**: inference is on-device; nothing is uploaded without a
  clear, per-request consent prompt.

## Why this is a moat, not a me-too

Incumbents bolt AI onto a closed OS and route your data to their cloud. AfriSovera's
AI is **on-device, capability-sandboxed, private, sovereign, and open** — the exact
properties a surveillance-funded platform *cannot* offer without undermining its
business model. For the Global South specifically, **private on-device translation
and voice** is a killer feature, not a demo.

## Built so far (milestone 2.46)

Asili today is a **deterministic, rule-based on-device responder** — private, no
cloud, reached only through the AI capability (`SYS_AI` for EL0 apps, the `chat`/`ai`
WASM apps, and the `ask` shell command). It genuinely does: **real arithmetic**
(`6 x 7`), **percentages** (`20% of 150`), **unit/base conversions** (C↔F, km↔miles,
decimal→hex), the **on-device clock**, and a **knowledge base** about AfriSovera
(privacy, security, voice, networking, signing, open-source, cross-platform,
sovereignty). This is the honest v0: the *architecture* (AI as a sandboxed,
capability-gated OS service whose input never leaves the device) is real; the
*intelligence* is still rules, not a neural net. The quantized on-device model below
replaces the rules without changing the capability boundary.

## Honest limits & phased plan

- Phase 2–3: STT/TTS/translation on-device (small signed models) + the AI capability
  and SDK; AI-native shell for developers.
- Phase 4: sovereign, self-hostable cloud offload (consented) via AfriCore; NPU
  driver ABIs; on-device coding assist.
- Kill-criterion honesty: if on-device models can't hit useful quality on target
  hardware, we ship the capability + tiering and lean on consented cloud — we do not
  ship a fake "AI phone" that secretly uploads everything. Sovereignty is the point.
