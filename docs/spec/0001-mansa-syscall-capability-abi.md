# AFR-0001 — Mansa system-call and capability ABI

| | |
|---|---|
| **Status** | **Implemented (v0, two dialects)**; v1 unified ABI **Designed** |
| **Version** | 0.1 (describes code at commit `485b863`, 2026-11-08) |
| **Editor** | Chief architect |
| **Code** | `kernel/src/process.rs`, `kernel/src/exceptions.rs` (AArch64); `kernel/src/pc/proc.rs`, `pc/user.rs`, `pc/cpu.rs`, `pc/mmu.rs` (x86-64); `userland/rustapp`, `userland/niavm` |
| **Suites** | `fault`, `sleep-syscall`, `sgi-el0`, `pc-ring3`, `pc-ipc`, `pc-userland` |
| **Licence** | Apache-2.0 |

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119.

---

## 1. Motivation

Mansa is specified as a capability microkernel (ADR 0001, ADR 0004): user code holds no authority except
the capabilities it is handed, and reaches the kernel only through a small, fixed set of system calls.
That boundary is the single most important interface in the system — every other spec (Asili drivers,
Nia, AfriIDL) is layered on it — and until now it existed only as code. This document:

1. records **v0** exactly as built, on both architectures, including where the two differ;
2. states the invariants a v0 caller may rely on and the ones it may not;
3. defines **v1**, a single architecture-neutral ABI, as the target the code converges to.

## 2. Scope and status summary

| Area | Built (v0) | Designed (v1) |
|---|---|---|
| Kernel entry, registers, return | Yes, both architectures | Unchanged except dedicated error codes |
| Syscall numbers | Yes, **different on ARM and x86** | One table for both |
| Capabilities | 4 slots per process, IPC endpoint rights SEND/RECV, granted at spawn by the kernel | Handle table, rights mask, derive/transfer/revoke, object types beyond endpoints |
| IPC | One endpoint, one `u64` per message, synchronous | Channels carrying bytes + handles, call/reply, shared-memory objects |
| Address spaces | Fixed layout, flat binaries | Same layout + an ELF-subset loader |
| Errors | One value: `EPERM = u64::MAX` | Small negative error set (§7.2) |

Everything in v1 is **designed, not built**. A v1 implementation is conformant only when the test vectors of
§10.2 pass on both architectures.

## 3. Terminology

- **Process** — one user address space plus one thread of control (v0 has no multi-threaded processes).
- **Handle** — a small integer naming a slot in the calling process's capability table.
- **Capability** — the kernel-held record a handle names: `{valid, object, rights}`. User code never sees
  the record, only the handle; a handle is meaningless in another process.
- **Endpoint** — the kernel object IPC messages pass through.
- **Word** — 64 bits on both architectures.

## 4. Kernel entry and the register convention (v0, normative)

### 4.1 Entry instruction

| | AArch64 | x86-64 |
|---|---|---|
| User privilege | EL0 | CPL 3 |
| Entry | `svc #0` | `int 0x80` (IDT gate 0x80, type/attr `0xEE`, the only DPL-3 gate) |
| Kernel side | lower-EL synchronous vector (+0x400) → `svc_entry`; `ESR_EL1.EC == 0x15` identifies an SVC | `isr_common` → `TrapFrame`; vector 0x80 → `proc::syscall` (or `user::syscall` when no process table is active) |
| Fast path | — | `SYSCALL`/`SYSRET` are **not** used in v0 |

The `svc` immediate MUST be 0. Callers MUST NOT rely on any other immediate being ignored.

### 4.2 Registers

| Role | AArch64 | x86-64 |
|---|---|---|
| Syscall number | `x8` | `rax` |
| Argument 0…5 | `x0 x1 x2 x3 x4 x5` | `rdi rsi rdx rcx r8 r9` |
| Return value | `x0` | `rax` |
| Preserved by the kernel | every GPR except `x0`, plus SP_EL0, ELR, SPSR | every GPR except `rax` |

Note the x86-64 argument 3 is `rcx`, not `r10` as in the Linux `syscall` ABI; this is possible because
v0 uses `int 0x80`, which does not clobber `rcx`. **A v1 move to `SYSCALL` MUST move argument 3 to `r10`**
(SYSCALL overwrites `rcx` with the return RIP and `r11` with RFLAGS).

### 4.3 Return values and errors (v0)

There is exactly one error value:

```
EPERM = 0xFFFF_FFFF_FFFF_FFFF   (u64::MAX)
```

It is returned for: an unknown syscall number; a handle out of range, invalid, or lacking the right; (x86)
a user buffer outside the caller's mapped user range. A syscall whose valid result could be `u64::MAX`
(e.g. `RECV` of a message whose value is `u64::MAX`) is **ambiguous in v0**; senders SHOULD NOT send
`u64::MAX`. v1 fixes this (§7.2).

### 4.4 Faults are not syscalls

Any other synchronous exception from user mode (data/instruction abort, undefined instruction, #PF, #GP,
#UD, #DE, …) terminates the process: ARM marks it `Dead`; x86 calls `proc::fault` (or, single-task mode,
returns `KILLED = 1<<63 | vector` to the launcher). The kernel MUST continue running other processes.
Suites: `fault`, `pc-ring3`.

## 5. System-call tables (v0, as built)

### 5.1 AArch64 (`kernel/src/process.rs`)

| # | Name | Arguments | Returns | Capability check |
|---|---|---|---|---|
| 0 | `SYS_DEBUG` | x0 = value | 0; value printed to the kernel log | none |
| 1 | `SYS_SEND` | x0 = handle, x1 = message word | 0, or EPERM | handle MUST carry `RIGHT_SEND` |
| 2 | `SYS_RECV` | x0 = handle | the message word, or EPERM | handle MUST carry `RIGHT_RECV` |
| 3 | `SYS_EXIT` | — (exit code not read) | does not return | none |
| 4 | `SYS_YIELD` | — | 0 | none |
| 5 | `SYS_WIN_CREATE` | x0 = width, x1 = height | window id | **none (gap)** |
| 6 | `SYS_WIN_FILL` | wid, x, y, w, h, colour | 0 | **none (gap)** |
| 7 | `SYS_WIN_PRESENT` | — | 0 | **none (gap)** |
| 8 | `SYS_GETPID` | — | process index | none |
| 9 | `SYS_WIN_TEXT` | wid, x, y, char, colour | 0 | **none (gap)** |
| 10 | `SYS_WIN_GETKEY` | — | key code (busy-polls) | **none (gap)** |
| 11 | `SYS_PUTC` | char | 0 | none |
| 12 | `SYS_TIME` | — | seconds from the PL031 RTC | none |
| 13 | `SYS_AI` | prompt_ptr, len (≤128), out_ptr, out_cap (≤256) | reply length | **none (gap)**; pointers **not range-checked** (see §8.2) |
| 14 | `SYS_SPEAK` | text_ptr, len (≤160) | 0 | **none (gap)**; pointer **not range-checked** |
| 15 | `SYS_SLEEP` | milliseconds (capped 5000) | milliseconds actually slept | none |
| 16 | `SYS_SGI_ARM` | — | 0 | none — diagnostic probe, MUST be removed in v1 |
| 17 | `SYS_SGI_READ` | — | count | none — diagnostic probe, MUST be removed in v1 |

### 5.2 x86-64, multi-process (`kernel/src/pc/proc.rs`)

| # | Name | Arguments | Returns | Capability check |
|---|---|---|---|---|
| 0 | `SYS_EXIT` | rdi = exit code (logged) | does not return | none |
| 1 | `SYS_PRINT` | rdi = value | 0 | none |
| 3 | `SYS_SEND` | rdi = handle, rsi = message word | 0, or EPERM | `RIGHT_SEND` |
| 4 | `SYS_RECV` | rdi = handle | message word, or EPERM | `RIGHT_RECV` |
| 5 | `SYS_PREEMPTED` | — | timer slices this process has lost | none |
| 8 | `SYS_GETPID` | — | process index | none |
| 11 | `SYS_PUTC` | char | 0 | none |
| 12 | `SYS_TIME` | — | seconds from the CMOS RTC | none |
| 13 | `SYS_AI` | as ARM | as ARM | none; pointers checked by `user_range()` |
| 14 | `SYS_SPEAK` | as ARM | 0 (text printed, not spoken) | none; pointer checked |
| 15 | `SYS_SLEEP` | as ARM | as ARM | none |
| 20–24 | `WIN_CREATE`, `WIN_FILL`, `WIN_PRESENT`, `WIN_TEXT`, `WIN_GETKEY` | as ARM 5, 6, 7, 9, 10 | as ARM | **none (gap)** |

### 5.3 x86-64, single-task mode (`kernel/src/pc/user.rs`)

Used by `pc-ring3` before the process table exists: `0 SYS_EXIT` (honoured only from CPL 3),
`1 SYS_PRINT`, `2 SYS_WHOAMI` (returns CPL, i.e. 3). A faulting task returns `KILLED | vector`
(`KILLED = 1 << 63`) to the launcher.

### 5.4 Divergence (why v1 exists)

| Call | ARM # | x86 # |
|---|---|---|
| EXIT | 3 | 0 |
| SEND | 1 | 3 |
| RECV | 2 | 4 |
| print/debug | 0 `DEBUG` | 1 `PRINT` |
| YIELD | 4 | — |
| PREEMPTED | — | 5 |
| window calls | 5, 6, 7, 9, 10 | 20–24 |

The same user binary therefore cannot run on both kernels even when its machine code is otherwise
portable (`userland/niavm` selects numbers by `cfg(target_arch)`). Nia `.wasm` apps are unaffected
because they never issue syscalls (AFR-0003).

## 6. Capabilities and IPC (v0, as built)

### 6.1 Capability record

```rust
// AArch64 (process.rs)            // x86-64 (pc/proc.rs)
struct Cap {                       struct Cap {
    valid:  bool,                      valid:    bool,
    obj:    usize, // endpoint index     endpoint: usize,
    rights: u8,                        rights:   u8,
}                                  }
const N_CAPS: usize = 4;           const N_CAPS: usize = 4;
const RIGHT_SEND: u8 = 1 << 0;     const RIGHT_SEND: u8 = 1;
const RIGHT_RECV: u8 = 1 << 1;     const RIGHT_RECV: u8 = 2;
```

Rights byte layout (v0):

```
 bit  7 6 5 4 3 2 1 0
      0 0 0 0 0 0 R S      S = SEND, R = RECV, bits 2..7 reserved, MUST be 0
```

- **Handle** = index into the per-process `caps[N_CAPS]` array; valid handles are 0..3.
- **Grant**: only the kernel grants, at process creation (ARM `grant(idx, handle, obj, rights)`; x86
  `spawn(code, caps: &[(endpoint, rights)])`, where list position becomes the handle). There is no
  syscall to create, copy, transfer, inspect or revoke a capability.
- **Check**: ARM requires `(rights & need) == need`; x86 requires `(rights & need) != 0`. With single-bit
  needs the two are equivalent; v1 MUST use the ARM form (all requested bits).
- **Revoke**: none. A capability ends when its process slot is reused.

### 6.2 Endpoint and message

v0 has exactly **one endpoint** per kernel and a message is exactly **one 64-bit word**.

ARM — synchronous rendezvous (`Endpoint { sender: Option<usize>, sender_msg: u64, receiver: Option<usize> }`):

```
            SEND(h, m)                                   RECV(h)
 ┌──────────────┐ receiver waiting? ──yes──► deliver m to receiver, receiver → Ready, return 0
 │ Running      │ ──no──► sender → BlockedSend(m) ──(RECV arrives)──► Ready, SEND returns 0
 └──────────────┘
 ┌──────────────┐ sender waiting? ──yes──► take m, sender → Ready, RECV returns m
 │ Running      │ ──no──► receiver → BlockedRecv ──(SEND arrives)──► Ready, RECV returns m
 └──────────────┘
```

At most one blocked sender and one blocked receiver can exist; a second blocked sender is a v0
limitation (it is never exercised by the suites).

x86-64 — one-slot buffered (`Endpoint { full: bool, value: u64 }`): SEND first hands the value to *any*
process in `BlockedRecv` (writing its saved `rax`); otherwise it fills the slot; if the slot is full the
sender blocks with the value pending. RECV empties the slot (refilling it from one blocked sender) or
blocks. When no process is runnable and some are blocked, the kernel reports
`blocked forever on IPC -- deadlock`.

**Process states** (both architectures):

```
          spawn                         SEND/RECV must wait
  (free) ───────► Ready ◄──────────────► Running ───────────────► BlockedSend / BlockedRecv
                    ▲   schedule / yield /                              │
                    │   timer preemption                                │ peer arrives
                    └───────────────────────────────────────────────────┘
  Running ── SYS_EXIT ──► Exited          Running ── fault ──► Dead
```

### 6.3 Address-space layout (v0)

| | AArch64 | x86-64 |
|---|---|---|
| Page/granule | 4 KiB granule, 39-bit VA, user regions as 2 MiB blocks | 4 KiB pages, 4-level |
| Code base / entry | `0x8000_0000`, EL0 R-X | `USER_BASE = 0x0000_0100_0000_0000`, up to 16 pages, R-only, padded with `0xCC` |
| Data | in the stack block | `USER_BASE + 0x2_0000`, one RW page |
| Stack | block at `0x8020_0000`, top `0x8040_0000`, RW, XN | 16 pages ending at `USER_BASE + 0x4_0000` |
| Optional MMIO window | `0x8060_0000` → PA `0x0900_0000` (Device memory) when spawned with `mmio` | — |
| Max user processes | 5 + idle | table size in `pc/proc.rs` |
| Kernel stack per process | 16 KiB from the kernel heap | from the shared heap |
| Executable format | **flat binary**, entered at offset 0 | **flat binary**, entered at offset 0 |

The linker scripts place `_start` at `0x8000_0000` (`userland/*/linker.ld`) and `0x100_0000_0000`
(`linker-x86_64.ld`). There is no ELF loader in v0.

## 7. Mansa ABI v1 (Designed — not built)

### 7.1 Entry

- AArch64: unchanged (`svc #0`, `x8`, `x0..x5`, return `x0`).
- x86-64: `SYSCALL`/`SYSRET` with number in `rax`, arguments `rdi rsi rdx r10 r8 r9`, return `rax`;
  `int 0x80` MAY be kept as a slow path with the v0 register order for one release, then removed.

### 7.2 Errors

Return values are signed. `0 ≤ r < 2^63` is success; `r ∈ [-4095, -1]` is an error. Defined errors:

| Value | Name | Meaning |
|---|---|---|
| -1 | `E_PERM` | handle valid but lacks the right |
| -2 | `E_BADHANDLE` | handle out of range or empty slot |
| -3 | `E_NOSYS` | unknown syscall number |
| -4 | `E_FAULT` | a user pointer is unmapped, not user, or wraps |
| -5 | `E_INVAL` | an argument is out of range |
| -6 | `E_NOMEM` | kernel cannot allocate the object |
| -7 | `E_WOULDBLOCK` | non-blocking variant would have blocked |
| -8 | `E_PEERCLOSED` | the other end of a channel is gone |
| -9 | `E_TOOBIG` | message exceeds the channel limits |
| -10 | `E_CANCELED` | the wait was ended by a close request on the caller's window (v0.5, §7.6) |
| -11 | `E_NOROUTE` | no route on the local network: no address from the router, or the next hop did not answer (v0.5, §7.6; added with S4, for the architect's confirmation) |
| -12 | `E_NOSOURCE` | no healthy strong entropy source: the call fails closed and its buffer is left untouched (v0.5, §7.6, `random_fill`) |
| -32 to -63 | file errors | reserved for AFR-0011 §11.6: file error code c is returned as `-(31 + c)` (Designed, step V6, §7.9) |

Message payloads are delivered through user buffers, never through the return register, so a payload
can no longer collide with an error.

### 7.3 Unified call table

| # | Name | Arguments | Result |
|---|---|---|---|
| 0 | `exit` | code | — |
| 1 | `yield` | — | 0 |
| 2 | `debug_write` | ptr, len | bytes written (debug builds only) |
| 3 | `clock_get` | clock id (0 = monotonic ns, 1 = UTC s) | value |
| 4 | `sleep` | ns | ns slept |
| 8 | `handle_close` | h | 0 |
| 9 | `handle_dup` | h, rights (subset) | new handle |
| 10 | `handle_info` | h, out_ptr (16 B: type u32, rights u32, koid u64) | 0 |
| 16 | `channel_create` | out_ptr (2 × u32 handles) | 0 |
| 17 | `channel_write` | h, bytes_ptr, bytes_len, handles_ptr, handles_n | 0 |
| 18 | `channel_read` | h, bytes_ptr, bytes_cap, handles_ptr, handles_cap → (out sizes in a 2 × u32 at bytes_ptr-header, see §7.5) | 0 |
| 19 | `channel_call` | h, wr_args_ptr, rd_args_ptr, timeout_ns | 0 |
| 24 | `wait_one` | h, signals, timeout_ns | observed signals |
| 32 | `vmo_create` | size | handle |
| 33 | `vmo_map` | vmo h, offset, len, prot | user address |
| 34 | `vmo_unmap` | addr, len | 0 |
| 40 | `irq_wait` | irq h | 0 (Asili, AFR-0002) |
| 41 | `mmio_map` | mmio h | user address (Asili) |
| 42 | `dma_alloc` | size, out_ptr (device address u64) | vmo handle (Asili) |
| 48 | `process_spawn` | image vmo h, handles_ptr, handles_n | process handle |

Window, AI and speech calls leave the syscall table in v1: they become channel protocols served by
user-space services (Tonga, Asili AI, Asili voice) and declared in AfriIDL (AFR-0004). An app reaches them
only if it holds a channel handle to that service — that is what makes the permission real.

### 7.4 Rights (v1, 32-bit)

```
bit  0 READ       1 WRITE      2 EXECUTE    3 MAP
bit  4 DUPLICATE  5 TRANSFER   6 SIGNAL     7 WAIT
bit  8 SEND (≡ v0 RIGHT_SEND)   9 RECV (≡ v0 RIGHT_RECV)
bit 10 CREATE (make a name in a folder, AFR-0011 §11.2)
bit 11 LIST   (read a folder's names, AFR-0011 §11.2)
bit 12..31 reserved, MUST be 0 on input, MUST be ignored on output by v1 callers
```

`handle_dup` MUST NOT widen rights. A handle sent in a message is **moved** (it leaves the sender's table)
and MUST carry `TRANSFER`. Closing the last handle to an object destroys it; a channel peer then observes
`E_PEERCLOSED`.

**Revocation** is by closing: a service that wants revocable delegation hands out a handle to an
intermediate channel it controls and closes that channel to revoke (the Zircon model). No global revoke
syscall is defined in v1.

### 7.5 Channel message limits

- At most **65 536 bytes** and **64 handles** per message.
- `channel_read` with a too-small buffer returns `E_TOOBIG` and leaves the message queued; the required
  sizes are written to the first 8 bytes of `bytes_ptr` (u32 bytes, u32 handles, little-endian).
- A channel queue holds at most 64 messages; a write to a full queue returns `E_WOULDBLOCK`.
- Message bodies are AfriIDL-encoded (AFR-0004 §5).

### 7.6 v0.5: the Dira subset (Designed, being built, 14 Nov 2026)

The calls Dira needs to leave the kernel (Dira M3, DIRA_M3_PLAN.md; the architect's verdict, ARCHITECT_REVIEWS 14 Nov
"Dira M3"), taken from v1 ahead of the rest of it, so that v1 replaces the kernel side and nothing in a program built
on them changes. PC first (ring 3, `pc/proc.rs`); ARM with its own plan.

**Entry and errors.** The v0 entry (§4.1, §4.2: `int 0x80`, `rax`, `rdi rsi rdx rcx r8 r9`). The calls below return
the v1 errors of §7.2 (signed; `-4095..=-1`), never v0's `EPERM`. The v0 calls keep v0's single error.

**Handles.** v0.5 widens the v0 capability record (§6.1) to a handle table of 8 entries per process, each entry an
object kind (`ENDPOINT`, `VMO`, `CELL`, `NET`, `CONN`, `RANDOM`, `TIME`) and v1 rights (§7.4). Handle numbers stay table indexes.
Only the kernel grants a handle at spawn, as in v0, apart from those a call below returns (a `VMO`, a `CONN`). v1
adds `handle_close`, `handle_dup`, transfer and channels on the same table. **v0.5 has no `handle_close`:** number 8
is `SYS_GETPID` on the PC in v0 (§5.2), and §9 never reuses a number within a version, so in v0.5 a `VMO` handle is
closed by its `vmo_unmap` and a `CONN` by `tcp_close`. (Correction while building S1, 14 Nov; v1's table gives 8 to
`handle_close` in the new version.) A `VMO` created but never mapped keeps its handle and its reservation against the
caps until the process exits, since nothing else can close it in v0.5; the table of 8 and the 32 MiB cap bound that,
and pc-vmo checks the reservation is back after such a process ends.

**What each program is given at spawn.** Only the kernel grants, at spawn; no call asks for a handle. A program not in
this table is spawned as today's userland, with neither CELL nor NET.

| Program | Spawned by | Handles | Rights | NET class bits |
|---|---|---|---|---|
| Dira (`userland/dira`) | the kernel's `dira` shell command only | CELL, NET, RANDOM, TIME | CELL: READ, WRITE, WAIT; NET: SEND, RECV, WAIT; RANDOM: READ; TIME: READ | 0, 1, 2 (public, private networks, link-local); bit 3 (127/8) only when the kernel is started with the test rig's flag |
| the `cell` test program (pc/proc.rs), **test-only** | the kernel's `celltest3` command, which refuses unless the test rig set fw_cfg `opt/afrisovera/test-grants` (a shipped image grants CELL to nothing but Dira) | CELL, titled "celltest3 (test)" | READ, WRITE, WAIT | none |
| rustapp, niavm and every other user program today | their shell commands, as in v0 | their v0 endpoints only | v0 SEND/RECV | none (no NET handle) |

**What `dira` gives Dira at spawn (S4), besides its handles.** All of it kernel-made, like a grant; nothing the
process writes is read back:
- a start block at the start of its data page: the mode (bit 0 text, bit 1 the test rig), which handle is the CELL
  and which the NET, the address as typed (at most 512 bytes; a longer one is refused at the shell, never cut), the
  TSC frequency (Dira reads `rdtsc` for its timings), and, written by the kernel when it ends one of Dira's waits with
  `E_CANCELED`, the TSC when it saw the stop and which stop it was (a close request, or Ctrl+C on the console);
- for `dira file NAME`, the store file's bytes (at most 16 KiB) mapped read-only and never executable at
  `USER_BASE + 0x2_4000`;
- up to 128 KiB of code (a spawn with grants; one without keeps 64 KiB). The bytes are userland/dira built by
  scripts/build-dira.sh; the kernel holds their SHA-256, the source's SHA-256 and the toolchain
  (kernel/src/pc/dira_bin.rs), records the hash in the boot's integrity manifest and starts no other bytes; suite
  dira-bin rebuilds them and fails if the committed ones differ.

A process can create a VMO (`vmo_create`) and a connection (`tcp_dial`, from a NET handle it holds); it can never
create CELL or NET.

**Memory (S1, built by the browser engineer, reviewed by the performance engineer):**

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 32 | `vmo_create` | size (bytes, a multiple of 4 KiB, at least 4 KiB) | `VMO` handle (READ, WRITE, MAP) | `E_INVAL` size; `E_NOMEM` past the caps below or no frames |
| 33 | `vmo_map` | vmo h, offset (0), len (the whole object), prot (READ\|WRITE) | user address | `E_BADHANDLE`; `E_PERM` without MAP, or prot with EXECUTE, or a CPU without NX; `E_INVAL` offset, len or prot; `E_NOMEM` no room in the map area or no frames |
| 34 | `vmo_unmap` | addr, len (as mapped) | 0, and the object's handle is closed | `E_INVAL` not a whole mapping |

**A v0.5 object's pages exist while it is mapped** (correction while building S1, 14 Nov). With one whole mapping per
object, `vmo_create` reserves the size against the caps and returns the handle; `vmo_map` takes the frames from the
frame pool, zeroes and maps them; `vmo_unmap` unmaps them, gives them back and closes the handle. So the frames an
object holds are exactly the pages of its mapping, recorded in the process's own page tables and nowhere else (no
frame list in a static), and `mmu::free_space` returns them at exit as it returns code and stack. v1, with several
mappings of one object, keeps the frames in the object from `vmo_create`; a program that creates, maps once and
unmaps (as Dira does) sees no difference.

Requirements (the architect's conditions on S1):
- every page is zeroed before it is mapped, and is never executable (NX set; a test jumps into one and must trap);
- one mapping of the whole object per `vmo_map` in v0.5 (offset 0, the whole length), in the **map area**: from
  `USER_BASE + 0x4000_0000` (`0x0000_0100_4000_0000`) up to, not including, `USER_BASE + 0x4400_0000` (64 MiB).
  That is 1 GiB above the code, data and stack of §6.3 (which end at `USER_BASE + 0x4_0000`). Mappings are placed
  first fit with one unmapped 4 KiB guard page after each. `vmo_map` never maps over code, data, stack or another
  mapping; when no gap fits, it returns `E_NOMEM`;
- the kernel's user-pointer check (`user_range`) accepts a buffer inside one of the caller's own mappings, as it
  accepts one between `USER_BASE` and the stack top today; a buffer that crosses a guard page is `E_FAULT`;
- per process at most 32 MiB in all objects, and at most 128 MiB for all processes together; past either cap
  `vmo_create` returns `E_NOMEM` and nothing is half made;
- the frames go back to the frame pool at `vmo_unmap`, when the process ends, and on every failed spawn; a `vmo_map`
  that runs out of frames gives back the ones it took and returns `E_NOMEM`, with nothing mapped;
- `ps` shows each process's object memory.

**Known limit in this release (PC):** the PC's frame pool is a 64 MiB block in the kernel image (`MAX_FRAMES` in
pc/shims.rs, `mansa_pc_frame_pool`), shared with the kernel's own page tables and heap growth. So the frames run out
before the 128 MiB total cap, and often before a process's 32 MiB: `vmo_map` then returns `E_NOMEM` with nothing
half made, as specified. The caps stay as written. Taking the pool from the free RAM the UEFI map reports (so its
size costs only the bitmap) is the performance engineer's own item, with its design to the architect first. Until
then, Dira starts at about 1 MiB a tab.

**The cell window (S2), the service calls of v0.5.** In v1 the cell window is a channel protocol to the desk
(Tonga) service; until then these five calls carry the same arguments and results as Nia's (afriui.md §25,
`cell_abi.rs`) and take the `CELL` handle the process was given at spawn. In v1 each becomes a message with the same
fields, its number is retired (§9: `E_NOSYS` forever), and the user library Dira calls (`afri_rt::cell`) changes
transport, not its function names.

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 64 | `cell_open` | cell h, min_cols, min_rows | 0 (the window is the handle's) | `E_PERM` no CELL right; `E_INVAL` negative; `E_WOULDBLOCK` another owner holds it; `E_PEERCLOSED` the desk is not showing |
| 65 | `cell_put` | cell h, col, row, cells_ptr, len (bytes, even) | cells written | `E_FAULT` the buffer is not the caller's; `E_INVAL`; `E_PEERCLOSED` closed |
| 66 | `cell_cursor` | cell h, col, row, shape (0..=3) | 0 | `E_INVAL`; `E_PEERCLOSED` |
| 67 | `cell_size` | cell h | cols << 14 \| rows | `E_PEERCLOSED` |
| 68 | `cell_event` | cell h, timeout_ms (negative: no limit) | an event as `cell_abi::event` packs it; 0 for none in time | `E_PEERCLOSED` (then every call says so) |
| 69 | `cell_put_text` | cell h, col, row, buf_ptr, buf_len (at most 1,024 bytes) | cells used | `E_FAULT` the buffer is not the caller's; `E_INVAL` a malformed buffer (below); `E_PEERCLOSED` closed |

`cell_put_text` writes one row of text from a column, for any script the desk can draw (added 21 Nov 2026 with the
browser's text past Latin-1; the architect's verdict). The buffer is `nruns` (u16, 1 to 64), then `nruns` runs of
(`len` u16, `role` u16), then the text as UTF-8; the runs' lengths cover the text exactly, in bytes, each run in one
colour role as `cell_put`'s. A run boundary inside a code point's bytes is `E_INVAL`; a run boundary inside a grapheme
cluster gives the whole cluster the role of its first code point. It comes in through the uaccess door, whole or not at all. The desk decodes the text
(invalid UTF-8 becomes U+FFFD), segments it into grapheme clusters, shapes each through its face chain, and puts one
cluster per cell, two for a wide one (East Asian wide and the like); a cluster no face can draw shows the box. It
stops at the row's end and returns the cells used. Right-to-left scripts (N'Ko, Arabic, Hebrew) are drawn in logical
order, left to right, until the desk's RTL work (C2c) lands; then this call follows it. `cell_put` (65) is unchanged
for apps that draw bytes.

The pointer in `cell_put` is checked against the caller's own mappings (as `mem_range` does for Nia). The title, its
marks and "Close anyway" are the desk's; the title of a process spawned by the `dira` command is "Dira (built-in)",
set by the kernel from the spawn, never by the process.

**The network (S3), service calls of v0.5.** In v1 a network service owns these over a channel; same rule as above.
A `NET` handle carries the classes it may reach: bit 0 public, bit 1 private networks, bit 2 link-local, bit 3 this
machine (127/8). No handle reaches 0/8, 224/3 or 255.255.255.255, whatever its bits.

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 72 | `tcp_dial` | net h, IPv4 (u32, big-endian), port, timeout_ms | `CONN` handle | `E_PERM` the class is not on the handle, or never allowed; `E_WOULDBLOCK` the process has a connection open (one at a time); `E_PEERCLOSED` refused or no answer in time; `E_NOROUTE` no route on the local network |
| 73 | `tcp_send` | conn h, ptr, len (at most 512) | bytes sent | `E_FAULT`; `E_INVAL`; `E_PEERCLOSED` |
| 74 | `tcp_recv` | conn h, ptr, cap, timeout_ms | bytes received; 0 = the peer closed | `E_FAULT`; `E_WOULDBLOCK` nothing in time; `E_TOOBIG` past 4 MiB on this connection (it is then closed) |
| 75 | `tcp_close` | conn h, how (bit 0: reset at once, for a load the person stopped; 0: FIN, then a reset after at most 200 ms) | 0 | `E_BADHANDLE` |
| 76 | `dns_a` | net h, name_ptr, name_len, timeout_ms | the IPv4 address (u32, big-endian, as a non-negative value) | `E_FAULT`; `E_INVAL` not a name; `E_PEERCLOSED` no checked answer (pc/dnscheck.rs); `E_NOROUTE` no route to the name server |

**Randomness and the clock (v0.5, added 21 Nov 2026 for Dira's TLS).** Two object kinds, `RANDOM` and `TIME`, granted
only at spawn like `CELL` and `NET`; a process can never create them. Each takes the v1 right READ and no other. In v1
they become services over channels like 64 to 76; the numbers are retired then, never reused (§9).

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 77 | `random_fill` | random h, ptr, len (1 to 256) | 0, the buffer filled | `E_BADHANDLE`; `E_PERM` not a RANDOM handle with READ; `E_INVAL` len 0 or past 256; `E_FAULT` the buffer is not the caller's writable memory; `E_NOSOURCE` no healthy strong source, the buffer untouched |
| 78 | `time_now` | time h, ptr (16 bytes) | 0, the buffer holds: seconds since 1970-01-01 UTC (u64), nanoseconds (u32, may be 0), flags (u32) | `E_BADHANDLE`; `E_PERM` not a TIME handle with READ; `E_FAULT` |

- `random_fill` is the kernel DRBG's secret output (`rng::fill_secret` on both kernels: the known-answer test passed and
  a fresh, health-tested reseed for this request). It fails closed: never a weaker fill. 256 bytes is below the DRBG's
  own per-request cap, so one call is one DRBG request.
- `time_now` reads the real-time clock (the PC's CMOS clock; ARM's PL031), UTC, no time zone. Flags bit 0: the clock
  was never set or reads before 2026-01-01, so it is not to be trusted for dates such as a certificate's. The
  monotonic clock is call 79, below.
- Both copy through the uaccess door, whole or not at all.

**The monotonic clock (v0.5, added 22 Nov 2026; the architect's verdict on CLOCK_PLAN, 21 Nov 2026).** No handle and
no right: elapsed time since boot tells a process nothing it could not measure by counting its own work. In v1 it
becomes the clock service's `monotonic`, and 79 is retired then, never reused (§9).

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 79 | `clock_monotonic` | none | nanoseconds since the kernel's timer was set up at boot (u64, as a non-negative value) | none |

- One rule for every process: the value is rounded down to a whole **microsecond** (the coarse-timer rule).
- **Never backwards:** the kernel keeps the largest value it has given out and returns at least that (`fetch_max`),
  across cores and across the rounding. A value can repeat.
- The counter: the PC's TSC at the rate measured once at boot (`timer::hz`); ARM's CNTPCT_EL0 at CNTFRQ_EL0, read by
  the kernel (what EL0 may read itself, through CNTKCTL_EL1, is unchanged). ns = (counter - t0) * 10^9 / hz, in 128
  bits. The PC's boot prints a `[timer]` line naming the rate's source and whether the TSC is invariant (CPUID
  0x80000007 EDX bit 8); where it is not, 79 still never goes backwards, but its rate may drift with power states.
  When no source answers and the rate is the assumed 1 GHz, the boot (and so the field report) prints "time source
  rate assumed, not measured".

**Test vectors for 79** (a ring-3 program `rt79`, suites `pc-rt79` and `arm-rt79`, no test-rig flag needed): 10,000
calls in a row, each at least the one before and each a whole number of microseconds; four more after 50 ms sleeps,
none backwards; two calls around a 100 ms sleep differ by 90 ms to 2 s; the frames all back. A failing control (a
job-only build that drops `fetch_max` and the rounding and takes 1 us off every second value) must fail the
backwards and the whole-microsecond checks. The Linux bridge's `clock1` program (pc-compat0) checks the same clocks
through `clock_gettime` against Linux.

**The stop rule (the kernel's job now that Dira is a process).** A close request on the process's window (the close
box, Alt+F4, Ctrl+\\ on the serial line), or the window closing, ends a blocked `tcp_dial`, `tcp_recv` or `dns_a`
within 100 ms. The call returns **`E_CANCELED` (-10)**, a new error added to §7.2's list for v0.5 and v1, and the
connection is reset at once, as at exit. The close request stays queued, so the next `cell_event` returns it
(kind 3), and the program answers it as Nia apps do. A blocked `cell_event` itself returns the close request as an
event, and after "Close anyway" kind 4 (`EV_CLOSED`) at once.

Every `tcp_dial` prints one `[net]` line with the process, the destination and the class. The kernel keeps TCP, the
route (M1b `route_open`), the DNS checks and the stop rules: when the process ends or its window closes, its
connection is reset at once. Nothing gives a process raw frames or a listening port. The rule that a public page may
not redirect into the home network stays in Dira (url.rs), which alone knows which page asked.

**What stays for v1:** `SYSCALL`/`SYSRET`, channels and handle transfer, `process_spawn` from a memory image, and the
window, AI, voice, cell and network services over channels (when they move, calls 64 to 76 are retired).

**Test vectors for 69, 77 and 78** (a ring-3 program `rt3` under the test rig's flag, suites `pc-rt3` and `arm-rt3`):
two 32-byte `random_fill`s differ and neither is all zero; len 0 and 257 are `E_INVAL`; a kernel address, a
non-canonical one and the code page are `E_FAULT`; without a RANDOM handle, and with a NET handle in its place,
`E_PERM`; a boot with no strong source gives `E_NOSOURCE` with the buffer (filled with 0xAA first) untouched.
`time_now` within 2 s of the host's UTC; flags bit 0 set when the clock is started at 2020-01-01; bad pointers
`E_FAULT`. `cell_put_text` with a row of Swahili, Yoruba (including a cluster with two combining marks, such as
U+1EB9 U+0300), Amharic and one unassigned code point: the cells used, the desk's verify line and a screenshot;
invalid UTF-8 shown as U+FFFD; runs that do not cover the text exactly `E_INVAL`; past 1,024 bytes `E_INVAL`; a run
boundary in the middle of a two-byte code point (after the first byte of U+1EB9) `E_INVAL`; a run boundary between a base
letter and its combining mark (U+1EB9 in one role, U+0300 in another) draws that cell in the first run's role.

**Test vectors (with each step):** S1: create, map, write and read back; refused EXECUTE; the jump into a mapped page
traps; the caps (`E_NOMEM`, nothing leaked: the frame pool's free count is the same before and after); exit frees;
map until the area is full (`E_NOMEM`), then check every mapping lies inside the area, none overlaps another or the
code, data and stack, and each is followed by its guard page; a buffer across a guard page is `E_FAULT`.
S2: a process without CELL is refused; a bad pointer is `E_FAULT`; the dira suite's window, against the process.
S3: the classes (refused 0/8, 224/3, 127/8 without bit 3, private without bit 1), one connection, the 4 MiB cap,
reset on exit, the `[net]` line; a close request while `tcp_recv` waits on a server that never answers: `E_CANCELED`
within 100 ms and the connection reset (in the capture); the same for `tcp_dial` and `dns_a`; pc-dira-net and dira
against the process.

### 7.7 v0.5: the Daraja subset (Designed, 14 Nov 2026; amended for stage L1.1, 21 Nov 2026)

The calls and rules a compatibility bridge (Daraja, docs/06) needs to run one foreign program, taken from v1 or added
beside it, so that a bridge built on them does not change when v1 lands. The plan is COMPAT_STAGE0_PLAN.md; the
architect's verdict is ARCHITECT_REVIEWS 14 Nov "compatibility stage 0". PC first (`pc/proc.rs`); ARM with its own plan.
Entry, errors and the handle table are those of §7.6. Written from: the System V AMD64 ABI (process start, auxiliary
vector), the AMD64 APM vol. 2 (SYSCALL, MSRs, IST), the ELF specification, and the Linux manual pages (`syscall(2)`,
`arch_prctl(2)`) for facts. No Linux kernel source was read.

**Personality.** Every process has a personality, fixed at spawn:

| Value | Name | Status |
|---|---|---|
| 0 | `NATIVE` | every process before this section; the Mansa call table |
| 1 | `LINUX_X86_64` | v0.5 |
| 2 | `WINDOWS_X86_64` | reserved; the kernel refuses to spawn it until its own plan |
| 3 | `DARWIN_X86_64` | reserved; as 2 |

The kernel never reads a foreign program's file (ELF, PE, Mach-O). The bridge loads it with the calls below.

**Entry.** `SYSCALL` is enabled (EFER.SCE, STAR, LSTAR, and SFMASK clearing IF, TF, DF and AC). One stub saves the
caller's registers into the same trap frame as `int 0x80`, then:
- **`NATIVE`:** a `syscall` instruction is handled as an invalid opcode (§4.4), exactly as before this section.
  (v1's native `SYSCALL` entry, §7.1, is a later step and dispatches on the same field.)
- **foreign:** the call is never looked up in the Mansa table. It is forwarded to the process's bridge (below).
- The return is always by `iretq`. `SYSRET` MUST NOT be used until a measured reason and its own review.
- NMI, #DB and #MC MUST have their own IST stacks before `SYSCALL` is enabled, because `SYSCALL` does not change the
  stack pointer and the stub runs briefly on a stack the user chose.
  The NMI handler MUST stay short (report, then end the process or halt): a #DB or #MC taken inside it, and that
  exception's `iretq`, re-open NMIs while IST 2 is still in use, so a second NMI would reuse the same stack. An NMI
  handler that does more needs its own plan for nested NMIs first.
- The stub uses one static scratch slot and is valid only while ring 3 runs on the boot processor. It MUST assert that
  in its own path. If ring 3 ever runs on other processors, the stub moves to per-CPU data reached with `swapgs`, and
  that change comes to the architect.
- `int 0x80` from a foreign process MUST NOT reach the Mansa table: the process ends and its bridge gets a FAULT
  message. A foreign process holds **no handles**, so no native call could act for it anyway.
- **FS and GS base** are saved and restored by the kernel on a switch to or from a foreign process. CR4.FSGSBASE stays
  off, so ring 3 cannot change them itself; they change only through `fproc_set_tls`.

**Spawn.** Only the kernel grants, at spawn (§7.6). In a test build (feature `compat0`), the shell command
`linux <name> [args]` makes an empty foreign process (a new address space with no user pages, not yet runnable,
preemptible) and its bridge, a native program, with exactly these handles:

| Handle | Kind | Rights | What it names |
|---|---|---|---|
| 0 | `FPROC` | READ, WRITE, MAP, EXECUTE, SIGNAL | that one foreign process; this handle exists only in its bridge |
| 1 | `CHANNEL` (kernel-peered) | READ, WRITE, WAIT | the foreign process's call channel; the kernel is the other end |
| 2 | `VMO` | READ, MAP | the bundle: a header (below), then a cpio "newc" archive of the program and its files |

The clock (the architect's verdict on CLOCK_PLAN, 21 Nov 2026): the bridge reads call 79 with no handle. Its
realtime is a base read once at its start plus the monotonic time since, so it never steps back within a run. Once
`time_now` (78) is in the kernel, the bridge also gets handle 3, `TIME` with READ, for that base; until then the base
is v0's `SYS_TIME` (seconds). The foreign process holds no handles either way.

The bundle header is per spawn, 48 bytes (the architect's layout, 15 Nov 2026): bytes 0..8 magic `DARAJA02`; 8..16
the archive's length in bytes, little-endian, written by the kernel from the bytes it copied, never from the archive;
16..32 `AT_RANDOM`, filled from `drbg::fill_secret` for that spawn and never reused (fail closed: no secret bytes, no
bridge); 32..40 the mapped page count, written by the kernel from the frames it mapped; 40..48 reserved 0. The bridge
copies the header once onto its own stack and refuses the bundle, by name, unless the magic is `DARAJA02`, bytes 40..48
are 0, and 48 + the length is at most the page count times 4096; its cpio reader is bounded by that length on every
read, and fuzz-host's `daraja_bundle` target checks it on any bytes. No `NET` or `CELL` handle is granted.

**Calls.**

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 17 | `channel_write` | ch h, bytes_ptr, bytes_len, 0, 0 | 0 | `E_PERM` no WRITE; `E_FAULT`; `E_INVAL` not a valid reply (below), or handles given; `E_PEERCLOSED` |
| 18 | `channel_read` | ch h, bytes_ptr, bytes_cap, 0, 0 | 0 | `E_PERM` no READ; `E_FAULT`; `E_TOOBIG` as §7.5; `E_WOULDBLOCK` empty; `E_PEERCLOSED` empty and the foreign process gone |
| 24 | `wait_one` | h, signals (bit 0 READABLE, bit 2 PEER_CLOSED), timeout_ns (negative: no limit) | observed signals; 0 when the time ran out | `E_BADHANDLE`; `E_PERM` no WAIT |
| 49 | `fproc_map` | fproc h, foreign addr, len, prot (bit 0 READ, 1 WRITE; READ or READ\|WRITE) | 0 | `E_PERM` no MAP right, or this CPU has no NX; `E_INVAL` EXECUTE asked, prot not R or RW, not page-aligned, outside the foreign area, or overlapping a mapping; `E_NOMEM` past the caps, no frames, or no free range entry (nothing half made) |
| 50 | `fproc_unmap` | fproc h, foreign addr, len | 0 | `E_INVAL` not page-aligned, or a page in the run not mapped; `E_NOMEM` no free range entry for the split (nothing changed) |
| 51 | `fproc_read` | fproc h, foreign addr, my ptr, len | bytes copied | `E_FAULT` any page on either side not mapped (all checked before the first byte moves) |
| 52 | `fproc_write` | fproc h, foreign addr, my ptr, len | bytes copied | `E_FAULT` as 51, or any foreign page in the range not WRITE (so never one that is EXECUTE, or sealed) |
| 53 | `fproc_start` | fproc h, rip, rsp | 0 | `E_INVAL` already started, rip not in an EXECUTE mapping, or rsp not in a WRITE mapping |
| 54 | `fproc_set_tls` | fproc h, which (0 FS, 1 GS), value | 0 | `E_INVAL` which, or value not canonical or not in the foreign area |
| 55 | `fproc_kill` | fproc h, exit code | 0 | `E_BADHANDLE` |
| 56 | `fproc_protect` | fproc h, foreign addr, len, prot (0 none, READ, or READ\|EXECUTE) | 0 | `E_PERM` EXECUTE without the right; after `fproc_start`, any right a page in the run does not have now (RW to RX included, and anything from none); `E_INVAL` WRITE asked, not page-aligned, a page in the run not mapped, or before the start EXECUTE on a page that is not currently RW; `E_NOMEM` no free range entry for the split (nothing changed) |

Channel numbers and their semantics are v1's (§7.3, §7.5). In v0.5 they act only on the kernel-peered call channel,
and any other handle kind is `E_BADHANDLE`. When v1 channels land, nothing a bridge calls changes.

**Number 24 on the PC (the architect's ruling, 14 Nov 2026).** On the PC under v0.5, number 24 is `wait_one` for a
bridge process and `WIN_GETKEY` (§5.2) for every other process. A process is a bridge from its spawn and stays one.
Bridges have no v0 window calls (20 to 23 are `E_NOSYS`). At v1 the window calls leave and 24 is `wait_one` for all.
Test vectors: a bridge's 24 after its foreign process has ended gives PEER_CLOSED; a plain process's 24 is still
`WIN_GETKEY` (pc-ring3 and pc-userland unchanged); a bridge's 20 gives `E_NOSYS`.

**Memory.**
- The **foreign area** is PML4 slot 2, `USER_BASE` (`0x0000_0100_0000_0000`) up to `USER_BASE + 2^40`. A foreign
  process has none of §6.3's native layout.
- **Foreign memory is not a VMO** (amended 14 Nov 2026 to fit §7.6 as built, where a VMO holds frames only while
  it is mapped). `fproc_map` takes fresh zeroed frames and maps them into the foreign process only, R or RW, never
  EXECUTE. The bridge fills them with `fproc_write`, then `fproc_protect` drops WRITE. The bridge holds no handle for
  foreign memory; the bundle (handle 2) is a VMO mapped only in the bridge, never in the foreign process.
- **Counting:** the frames are counted on the **foreign process's own** §7.6 count (`vmo_pages`, and the total), so
  the foreign process ending gives the frames and the reservation back together, as for any process. The per-process
  cap (32 MiB) applies to the bridge and its foreign process together, checked at both doors: the bridge's
  `vmo_map` counts its foreign process's pages too, and `fproc_map` counts the bridge's. They go back to the frame pool when the
  foreign process ends, and pc-daraja checks that the frame pool's used count and the total reservation are back at
  their starting values with the foreign process dead and the bridge still alive.
- **No NX, no foreign memory:** on a CPU without NX, `fproc_map` is `E_PERM`, as `vmo_map` is (§7.6).
- **Write or execute, never both, and the seal is one way, per page.** A page is RW (no-execute), R (no-execute), RX,
  or none. Before the start, `fproc_protect` may turn RW into R or RX. Nothing ever turns R or RX back into WRITE:
  `fproc_map` never maps over an existing page, and no call adds WRITE. `fproc_write` refuses any page that is not WRITE.
- **EXECUTE only before the start.** `fproc_protect` may add EXECUTE only before `fproc_start`. Stage 0 and L1 need no
  code generated at run time; a program that needs a JIT is its own plan, because "write, then execute" is what W^X
  exists to stop.
- **After the start, rights are only removed** (amended 21 Nov 2026 for L1.1, the architect's verdict on
  COMPAT_L1_PLAN section 9). The new rights must be a subset of every covered page's rights now: RW to R, RW or R or RX
  to none, RX to R. Adding any right is `E_PERM`, RW to RX included (it adds EXECUTE). A bridge answers a program's
  `mprotect` with these rules, and its own refusal (EACCES) for anything they forbid.
- **None frees the frames.** Rights 0 gives the pages' frames back at once (they leave the process's count), and the
  pages keep rights 0 in their range, so a later unmap or protect of them still answers as for mapped pages. A page
  with rights 0 never gains a right again (`E_PERM`); the way back is unmap, then a fresh `fproc_map` (zeroed frames).
  This differs from Linux, which keeps the contents under PROT_NONE; a bridge says so by name rather than give zeroed
  pages where a program expects its old data. Keeping frames under rights 0 is its own plan if a program that matters
  needs it.
- **Ranges.** The kernel keeps up to 128 ranges per foreign process (a range: a run of pages with one set of rights).
  `fproc_map` that starts exactly where a range with the same rights ends grows that range instead of taking a new
  entry. `fproc_unmap` and `fproc_protect` act on any page-aligned run inside mapped pages: the kernel splits ranges at
  the run's two ends, checking for room first, so a full table is `E_NOMEM` with nothing changed. A bridge's
  `MAP_FIXED` is `fproc_unmap` then a fresh `fproc_map`, never a map over pages.
- **Stale TLB entries.** When `fproc_protect` or `fproc_unmap` changes a page of a foreign address space, including
  every split, no TLB entry may keep the old permission. On one core the bridge is running during its call, so the
  foreign space is not the loaded one, and the next CR3 load into it drops its non-global entries (user pages are never
  global). The kernel still runs `invlpg` on each changed page when the foreign root is the loaded one, so this stays
  true if that ever changes (another core, or a call made with the foreign space loaded).
- **The limit:** the low 1 TiB (slot 0, the kernel's 1:1 map) cannot hold foreign pages. Programs linked at fixed low
  addresses (non-PIE, for example at `0x400000`) cannot run under v0.5. Position-independent programs, and ours
  linked high, can. Lifting this is its own plan to the architect before stage L1 claims "programs as downloaded".

**The call message** (kernel to bridge, on the call channel), 96 bytes, little-endian:

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 2 | version | 1 |
| 2 | 2 | kind | 1 CALL; 2 FAULT (the process is already stopped); 3 EXITED (ended by the kernel, e.g. `fproc_kill` or a refusal) |
| 4 | 2 | personality | as above |
| 6 | 2 | thread | 0 (one thread in v0.5) |
| 8 | 8 | seq | starts at 1, one more per message for this process |
| 16 | 8 | nr | CALL: `rax`; FAULT: the vector; EXITED: the exit code |
| 24 | 48 | args[6] | CALL: `rdi rsi rdx r10 r8 r9`; FAULT: error code, then the faulting address (CR2) for #PF; otherwise 0 |
| 72 | 8 | rip | of the call or fault |
| 80 | 8 | rsp | of the call or fault |
| 88 | 8 | reserved | 0 |

**The reply** (bridge to kernel, `channel_write`), exactly 24 bytes: u16 version 1, u16 kind (1 RESULT: `value` goes
in `rax` and the process runs on; 2 EXIT: the process ends with `value` as its code), u32 reserved 0, u64 seq (that of
the CALL waiting), u64 value. A reply of another size, kind or version, with a reserved field not 0, with the wrong
seq, or with no CALL waiting, is `E_INVAL` and changes nothing. The kernel consumes a valid reply at once; replies never
queue. FAULT and EXITED messages take no reply.

**Rules.**
- One CALL in flight per foreign process. While it waits, the process is blocked and is not scheduled.
- A foreign process's fault is always a FAULT message to its bridge, never a kernel panic and at most one console line.
- **The bridge ending ends its foreign process** (the call channel's peer is gone); the foreign process ending gives
  the bridge a last EXITED or FAULT message, then `E_PEERCLOSED`. Both are freed as any process is (kernel stack,
  address space, VMOs), and the frame pool's free count returns to its starting value.
- Each foreign process's start and end is one `[daraja]` console line: the process, its bridge, the personality, the
  exit code or the fault.
- **Per-call lines only when asked** (amended 21 Nov 2026 for L1.1). The kernel's per-call `[daraja]` line, and the
  bridge's, appear only for a pair spawned with the trace flag. In the test build the shell's `linux <name>` sets it
  when the bundle holds the file `daraja/trace` (the shell reads the archive's names; the kernel never reads a
  foreign program's file). It is off by default, because per-call lines at 115200 baud dominate a real program's run
  time. pc-daraja and pc-compat0 keep one traced run, where the call order is compared.

**Test vectors (suite `pc-daraja`, with each step).** A foreign process that runs `int 0x80` ends, with the refusal
line and a FAULT to its bridge. `fproc_map` with EXECUTE is `E_INVAL`; `fproc_protect` adding EXECUTE to a page that is
not RW is `E_INVAL`, asking for WRITE is `E_INVAL`, and adding EXECUTE after `fproc_start` is `E_PERM`. `fproc_write`
into an R or RX page is `E_FAULT`; after the seal, `fproc_map` over an R or RX page is `E_INVAL` (the other half of
"nothing returns to WRITE"); `fproc_read` across the end of a mapping is `E_FAULT` and copies nothing. With the
foreign process dead and the bridge alive, the frame pool's used count and the total reservation are back at their
starting values. A reply with the wrong seq is `E_INVAL`, the process still waits, and the right reply then works. Killing the
bridge while its process waits ends both and returns every frame. A native program's `syscall` ends as an invalid
opcode (`pc-ring3` unchanged). A foreign `syscall` with TF set does not single-step the kernel. The IST entries for
NMI, #DB and #MC are set at boot. The end-to-end proof: `hello0` and `fault0`, built from source with musl 1.2.5, give
the same stdout, stderr and exit code on Linux and through the bridge, and the forwarded call numbers equal `strace`'s
list.

**Test vectors for the L1.1 amendment (pc-daraja, one case per point).**
- A second `fproc_map` that starts where an RW range ends grows it: the range count stays the same.
- Unmapping the middle page of a three-page range leaves two ranges, and both still read through `fproc_read`.
- Protecting part of a range to R, then the whole range to R, works; a write to the R page by the started program
  faults (a FAULT message, vector 14), which shows no stale TLB entry kept WRITE.
- After the start: R to RW, and RW to RX, are both `E_PERM`, and nothing changes.
- Protecting a run to none gives its frames back: the pair's page count drops by the run's pages; protecting it back
  to R is `E_PERM`; unmapping it works.
- With the range table full, a split by `fproc_unmap` or `fproc_protect` is `E_NOMEM`, and the ranges and pages are
  exactly as before.
- End to end: `mprot0` (rights only removed) gives the same output as Linux; `mprot1` (rights added after the start)
  is refused with EACCES on all three tries through the bridge, and allowed on Linux, each side graded on its own.

## 8. Security considerations

### 8.1 What v0 already guarantees

- User code runs at EL0/CPL 3; kernel memory is mapped EL1-only (ARM) / supervisor-only (x86).
- ARM user code pages are not writable and the stack is execute-never.
- A fault in user mode kills only that process (suites `fault`, `pc-ring3`).
- SEND/RECV check the handle and right on every call (suites `pc-ipc`; ARM Phase 0 demo).

### 8.2 Known gaps in v0 (each tracked in (a private engineering record))

1. **ARM `SYS_AI` and `SYS_SPEAK` dereference user pointers without a range check.** A process can pass a
   kernel virtual address and have the kernel read (AI, SPEAK) or write (AI reply, up to 256 bytes) it.
   This is a privilege-escalation primitive. Required fix: the x86 `user_range()` check — the whole range
   in `[USER_ENTRY, USER_STACK_TOP)`, no overflow, every page user-mapped in the caller's tables — before
   any access. **Severity: high.**
2. **x86 user pages and NX: closed.** `mmu::map_user_in` marks every writable ring-3 page NX (bit 63) once
   EFER.NXE is set at boot, so stack, data and v0.5 memory objects are not executable; pc-vmo jumps into an object
   and the CPU faults. On a CPU without NX (none made since about 2004) the boot line says so, and those pages are
   writable but not guaranteed non-executable, and `vmo_map` refuses every v0.5 object there (`E_PERM`, never `E_NOMEM`: freeing memory cannot help), so §7.6's
   "never executable" holds wherever an object is mapped. (Corrected 14 Nov with S1; it said user pages were never NX.)
3. Window, AI, speech, time and putc calls are **ambient** — any process may call them. In v1 they are
   capability-gated by moving behind channels (§7.3).
4. x86 SEND ignores the endpoint the handle names (one global endpoint, and a blocked receiver is matched
   regardless of endpoint). Harmless with one endpoint, wrong as soon as there are two.
5. A message value of `u64::MAX` is indistinguishable from EPERM.
6. Diagnostic SGI syscalls 16/17 are callable by any ARM process.

### 8.3 Rules for v1 implementers

- Every user pointer MUST be validated (range, overflow, mapping, permission) before the first access, and
  accessed with fault-safe copy routines; TOCTOU on shared buffers MUST be assumed.
- A syscall MUST NOT leak kernel addresses or uninitialised kernel memory in results.
- Rights MUST be checked as "all requested bits present".
- Handle values SHOULD be randomised per process so that a stale handle is unlikely to name a new object.

## 9. Versioning

- The ABI version is a single integer. v0 is the code today; v1 is §7.
- From v1 on, a process image declares its ABI version in its header; the kernel MUST refuse an image that
  declares a version it does not implement, with `E_NOSYS` from `process_spawn`.
- Syscall numbers are never reused for a different meaning. A removed call returns `E_NOSYS` forever.
- New calls take new numbers; new arguments to an existing call require a new number.
- Within one ABI version, reserved rights bits and reserved fields MUST be zero on input.

## 10. Test vectors

### 10.1 v0 (each is exercised by the named suite or can be added to it)

| # | Arch | Setup | Call | Expected |
|---|---|---|---|---|
| V0-1 | both | caps[0] = {endpoint 0, SEND} | SEND(h=0, 42) with a receiver blocked on caps RECV | returns 0; receiver's RECV returns 42 (`pc-ipc`) |
| V0-2 | both | caps[0] = {endpoint 0, RECV} only | SEND(h=0, 1) | EPERM (`0xFFFFFFFFFFFFFFFF`) |
| V0-3 | both | — | SEND(h=4, 1) (h ≥ N_CAPS) | EPERM |
| V0-4 | both | caps[1] empty | RECV(h=1) | EPERM |
| V0-5 | ARM | — | x8 = 99 | EPERM |
| V0-6 | x86 | — | rax = 99 | EPERM |
| V0-7 | x86 | — | SYS_AI(prompt_ptr = 0xFFFF_8000_0000_0000, 4, …) | EPERM, kernel survives |
| V0-8 | both | — | user writes to its code page | process killed, others continue (`fault`, `pc-ring3`) |
| V0-9 | x86 single-task | — | SYS_WHOAMI | 3 |
| V0-10 | both | — | SLEEP(100) | returns ≥ 100 and ≤ 5000 (`sleep-syscall`) |
| V0-11 | ARM | — | SYS_AI(prompt_ptr = kernel VA, …) | **today: kernel reads it (gap §8.2-1). Required: EPERM.** |

### 10.2 v1 conformance (to be written with the implementation)

| # | Call | Expected |
|---|---|---|
| V1-1 | `handle_dup(h, rights ∪ {bit not held})` | `E_PERM` |
| V1-2 | `channel_write` of a handle without `TRANSFER` | `E_PERM`, handle still in sender's table |
| V1-3 | `channel_write` of a handle with `TRANSFER` | sender's slot empty; receiver gets a new handle naming the same koid |
| V1-4 | `channel_read` with bytes_cap = 0 on a 10-byte message | `E_TOOBIG`; first 8 bytes of buffer = `0A 00 00 00 00 00 00 00`; message still queued |
| V1-5 | close one end; `channel_read` the other | `E_PEERCLOSED` once the queue is empty |
| V1-6 | any call with a pointer `0xFFFF_FFFF_FFFF_FFF0`, len 32 | `E_FAULT` (overflow) |
| V1-7 | `handle_info` on a channel handle | type = 2 (channel), rights as granted |

## 11. References

- ADR 0001, ADR 0004; docs/01-kernel-mansa.md; docs/10-security-model.md.
- seL4 Reference Manual (capabilities, IPC); Fuchsia Zircon syscall and handle documentation (the v1
  channel/VMO/rights model is deliberately close to Zircon's so its literature applies).
- AMD64 APM vol. 2 §6.1 (SYSCALL/SYSRET register use); Arm ARM D1.10 (exception entry, ESR_EL1.EC).
- System V Application Binary Interface, AMD64 Architecture Processor Supplement (process start, auxiliary
  vector); the ELF specification; Linux manual pages `syscall(2)`, `arch_prctl(2)` (facts only, §7.7).
