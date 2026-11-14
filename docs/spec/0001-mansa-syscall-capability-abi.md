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
bit 10..31 reserved, MUST be 0 on input, MUST be ignored on output by v1 callers
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
object kind (`ENDPOINT`, `VMO`, `CELL`, `NET`, `CONN`) and v1 rights (§7.4). Handle numbers stay table indexes;
`handle_close` (8) is in the subset. Only the kernel grants a handle at spawn, as in v0, apart from those a call
below returns (a `VMO`, a `CONN`). v1 adds `handle_dup`, transfer and channels on the same table.

**What each program is given at spawn.** Only the kernel grants, at spawn; no call asks for a handle. A program not in
this table is spawned as today's userland, with neither CELL nor NET.

| Program | Spawned by | Handles | Rights | NET class bits |
|---|---|---|---|---|
| Dira (`userland/dira`) | the kernel's `dira` shell command only | CELL, NET | CELL: READ, WRITE, WAIT; NET: SEND, RECV, WAIT | 0, 1, 2 (public, private networks, link-local); bit 3 (127/8) only when the kernel is started with the test rig's flag |
| rustapp, niavm and every other user program today | their shell commands, as in v0 | their v0 endpoints only | v0 SEND/RECV | none (no NET handle) |

A process can create a VMO (`vmo_create`) and a connection (`tcp_dial`, from a NET handle it holds); it can never
create CELL or NET.

**Memory (S1, built by the browser engineer, reviewed by the performance engineer):**

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 32 | `vmo_create` | size (bytes, a multiple of 4 KiB, at least 4 KiB) | `VMO` handle (READ, WRITE, MAP) | `E_INVAL` size; `E_NOMEM` past the caps below or no frames |
| 33 | `vmo_map` | vmo h, offset (0), len (the whole object), prot (READ\|WRITE) | user address | `E_BADHANDLE`; `E_PERM` without MAP, or prot with EXECUTE; `E_INVAL` offset, len or prot; `E_NOMEM` no room in the map area |
| 34 | `vmo_unmap` | addr, len (as mapped) | 0 | `E_INVAL` not a whole mapping |
| 8 | `handle_close` | h | 0 | `E_BADHANDLE` |

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
- the frames go back to the frame pool when the last handle closes or the process ends, and on every failed spawn;
- `ps` shows each process's object memory.

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

The pointer in `cell_put` is checked against the caller's own mappings (as `mem_range` does for Nia). The title, its
marks and "Close anyway" are the desk's; the title of a process spawned by the `dira` command is "Dira (built-in)",
set by the kernel from the spawn, never by the process.

**The network (S3), service calls of v0.5.** In v1 a network service owns these over a channel; same rule as above.
A `NET` handle carries the classes it may reach: bit 0 public, bit 1 private networks, bit 2 link-local, bit 3 this
machine (127/8). No handle reaches 0/8, 224/3 or 255.255.255.255, whatever its bits.

| # | Name | Arguments | Result | Errors |
|---|---|---|---|---|
| 72 | `tcp_dial` | net h, IPv4 (u32, big-endian), port, timeout_ms | `CONN` handle | `E_PERM` the class is not on the handle, or never allowed; `E_WOULDBLOCK` the process has a connection open (one at a time); `E_PEERCLOSED` refused or no answer in time |
| 73 | `tcp_send` | conn h, ptr, len (at most 512) | bytes sent | `E_FAULT`; `E_INVAL`; `E_PEERCLOSED` |
| 74 | `tcp_recv` | conn h, ptr, cap, timeout_ms | bytes received; 0 = the peer closed | `E_FAULT`; `E_WOULDBLOCK` nothing in time; `E_TOOBIG` past 4 MiB on this connection (it is then closed) |
| 75 | `tcp_close` | conn h | 0 | `E_BADHANDLE` |
| 76 | `dns_a` | net h, name_ptr, name_len, timeout_ms | the IPv4 address (u32, big-endian, as a non-negative value) | `E_FAULT`; `E_INVAL` not a name; `E_PEERCLOSED` no checked answer (pc/dnscheck.rs) |

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

**Test vectors (with each step):** S1: create, map, write and read back; refused EXECUTE; the jump into a mapped page
traps; the caps (`E_NOMEM`, nothing leaked: the frame pool's free count is the same before and after); exit frees;
map until the area is full (`E_NOMEM`), then check every mapping lies inside the area, none overlaps another or the
code, data and stack, and each is followed by its guard page; a buffer across a guard page is `E_FAULT`.
S2: a process without CELL is refused; a bad pointer is `E_FAULT`; the dira suite's window, against the process.
S3: the classes (refused 0/8, 224/3, 127/8 without bit 3, private without bit 1), one connection, the 4 MiB cap,
reset on exit, the `[net]` line; a close request while `tcp_recv` waits on a server that never answers: `E_CANCELED`
within 100 ms and the connection reset (in the capture); the same for `tcp_dial` and `dns_a`; pc-dira-net and dira
against the process.

## 8. Security considerations

### 8.1 What v0 already guarantees

- User code runs at EL0/CPL 3; kernel memory is mapped EL1-only (ARM) / supervisor-only (x86).
- ARM user code pages are not writable and the stack is execute-never.
- A fault in user mode kills only that process (suites `fault`, `pc-ring3`).
- SEND/RECV check the handle and right on every call (suites `pc-ipc`; ARM Phase 0 demo).

### 8.2 Known gaps in v0 (each tracked in docs/review/)

1. **ARM `SYS_AI` and `SYS_SPEAK` dereference user pointers without a range check.** A process can pass a
   kernel virtual address and have the kernel read (AI, SPEAK) or write (AI reply, up to 256 bytes) it.
   This is a privilege-escalation primitive. Required fix: the x86 `user_range()` check — the whole range
   in `[USER_ENTRY, USER_STACK_TOP)`, no overflow, every page user-mapped in the caller's tables — before
   any access. **Severity: high.**
2. **x86 user pages are never marked NX** (`map_user_in` sets PRESENT|USER|WRITE only): the stack and
   data pages are executable. Fix: set bit 63 when EFER.NXE is enabled. **Severity: medium.**
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
