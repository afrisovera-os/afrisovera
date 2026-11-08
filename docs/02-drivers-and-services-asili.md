# 02 — Asili: Drivers & System Services

Above the Mansa kernel, **everything is a userspace process**: drivers, the filesystem, the network stack, the display server, telephony, and the AfriCore services. They communicate only via capability-secured IPC. **Asili** (Swahili: *foundation/origin*) is the framework that hosts drivers and the stable ABI that keeps them alive across OS updates.

## The driver problem, solved structurally

The reason no one ships new phone OSes is the driver gap. Asili attacks it on two fronts.

### 1. A stable driver ABI (the long-term win)
- Drivers are **normal sandboxed processes** that hold capabilities only for *their* hardware (a specific MMIO range, IRQ line, DMA region). A compromised driver cannot touch the rest of the system.
- The **driver ↔ system contract is a versioned, stable ABI** (the Fuchsia lesson). A driver compiled once keeps working across OS updates — no recompiling the world for every release.
- Drivers are **hot-restartable**: crash → supervisor restarts the driver process → hardware re-initializes, device stays up.
- This makes drivers *cheap to write and maintain*, so a community and hardware partners can actually sustain a driver ecosystem — the thing Symbian/webOS never made affordable.

### 2. An Android-driver bridge (the bring-up shortcut)
Writing native Asili drivers for every sensor on a Qualcomm SoC from scratch would take years before the phone even powers a screen. So for **hardware bring-up**, Asili can host **existing Android kernel drivers / HAL blobs** behind the stable ABI — conceptually like `libhybris`, which let Sailfish OS and Ubuntu Touch reuse Android drivers on real phones. This buys working cellular, GPU, and sensors *now*; native drivers replace the shims over time, device by device.

```
        app / service
             │  (typed IPC, AfriIDL)
             ▼
   ┌───────────────────────┐
   │  Asili driver process │  ← sandboxed, holds only its HW capabilities
   │  ┌─────────────────┐  │
   │  │ native Asili drv │  │   ← the long-term, clean implementation
   │  │      — or —      │  │
   │  │ Android-blob shim│  │   ← bring-up: reuse vendor driver
   │  └─────────────────┘  │
   └───────────┬───────────┘
               │ capabilities (MMIO / IRQ / DMA) granted by Mansa
               ▼
            hardware
```

## The core system services (all userspace)

| Service | Responsibility |
|---|---|
| **Device Manager** | Enumerates hardware, binds drivers, supervises/restarts them |
| **Display Server (Tonga)** | Vulkan-based compositor; owns the framebuffer; composites Sankofa UI surfaces; zero-copy via shared VMOs |
| **VFS / Storage** | Filesystem service(s); a modern CoW filesystem for integrity + cheap snapshots (atomic OS updates) |
| **Netstack** | Userspace TCP/IP + Wi-Fi/cellular data paths; per-app network *capability* gating (no cap → no network, enforced by the kernel) |
| **Telephony / Multi-SIM** | Modem control, calls, SMS, USSD, intelligent multi-SIM routing |
| **Power Manager** | Profiles, ration mode, thermal, battery health |
| **Input** | Touch, buttons, keyboard, gestures |

## AfriCore — the sovereign services layer

These ride on the same userspace + IPC model and define AfriSovera's identity. (Same sovereign concepts as the rest of the AfriSovera vision, but here implemented natively, not as Android replacements.)

- **AfriID** — capability-aware identity & auth; standards-based (OIDC/OAuth2); **self-hostable** by a telco/state/university; nothing forces a single foreign mothership.
- **AfriPay** — **OS-level mobile money.** A Payments service + a trusted system payment sheet abstracting M-Pesa, MTN/Airtel MoMo, USSD rails, and banks. Apps request a payment via a capability; they never touch credentials directly.
- **AfriPush** — open, self-hostable push (UnifiedPush-style), so notifications don't tether the OS to a foreign cloud.
- **AfriShare** — OS-level, signature-verified **peer-to-peer** transfer of apps, files, and even OS updates over Wi-Fi Direct/BT — vital where data is scarce.
- **AfriStore** — sovereign distribution: WASM apps + PWAs + (bridged) Android apps; local-currency payouts via AfriPay; offline install via AfriShare; signature-verified.

## Update & integrity model
- CoW filesystem + **A/B-style atomic updates**: an update is staged and switched atomically; a failed boot rolls back. No half-flashed bricks.
- Verified boot chain with AfriSovera-signed images; capability model means even a compromised service is boxed in.
