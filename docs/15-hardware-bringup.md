# 15 — Real-hardware bring-up (Phase 3 groundwork)

AfriSovera runs today on QEMU `-M virt`. This document is the honest map of what
changes when we target real silicon — **Raspberry Pi 4** (Broadcom BCM2711) and
**PINE64 / PinePhone** (Allwinner A64) — and how the codebase is being shaped to get
there. It is groundwork, not a claim that we boot on hardware yet.

## What is already decoupled

`kernel/src/board.rs` centralizes the peripheral MMIO bases the drivers used to
hard-code. `uart`, `gic`, `fwcfg` and `virtio` now read their bases from `board::`,
and the `board` shell command prints the active map. Adding a new board starts as a
new `Map` value here — not a hunt across the tree.

All four boards in this document now have a map: `board-virt` (default),
`board-raspi` (BCM2837), `board-rpi4` (BCM2711, milestone 2.722) and `board-pine64`
(Allwinner A64, milestone 2.723) — the last two being the third and fourth columns
of the table below. Exactly one is compiled in as the active board, but *any* of
them can be inspected from any build — `board list`, `board rpi4`, `board pine64` —
so the addresses below are checkable from a running kernel rather than only on
paper. `scripts/board-rpi4-test.sh` and `scripts/board-pine64-test.sh` parse their
own column of that table and assert the kernel prints exactly those addresses, so
the maps and this document cannot drift apart silently.

Only `board-virt` and `board-raspi` **boot**. `board-rpi4` is a map, not a port: it
still needs the rest of items 1 and 4 of "Suggested order" below (item 1's
flat-binary link target is done as of 3.672 — `kernel/linker-rpi4.ld`, entry
`0x8_0000`, compile-checked only; as of 3.673 the `armstub` question has a
documentation-sourced answer — no custom `armstub64.bin` needed, stock RPi4
firmware already enters `kernel8.img` at EL2 — but is still boot-unverified;
mailbox framebuffer remains, though as of 3.675 the driver itself is no longer
hardwired to BCM2837's address — see below). As of 3.674, item 3 (GIC + generic timer) is also
documentation-closed rather than needing new code: `board-rpi4` compiles through
the *same* boot path as `board-virt` (there is no `#[cfg(feature = "board-rpi4")]`
branch in `main.rs`, only a `board-raspi` / `not(board-raspi)` split), and that
path already calls the board-agnostic `gic::init()` (bases resolved from
`board::resolved()`, and `board.rs` has carried real GIC-400 addresses for
BCM2711 since 2.722) and `timer::init()` (reads `CNTFRQ_EL0` at runtime rather
than a compiled-in frequency). Item 2 is written but has never met a firmware
that passes a live DTB. The BCM2711 does have a real GICv2 (GIC-400), unlike
the Pi 3 whose GIC bases are honestly 0.

**Item 4 groundwork (3.675): the mailbox framebuffer driver is now board-parametric,
not BCM2837-only.** `kernel/src/pi_fb.rs` used to hard-code
`MBOX_BASE: usize = 0x3F00_B880` — correct for BCM2837 (`board-raspi`) but silently
wrong the moment anything tries to reuse the driver for BCM2711 (`board-rpi4`,
mailbox at `0xFE00_B880` — same `0x00_B880` offset, rebased onto the low-peripheral
window that moved from `0x3F00_0000` to `0xFE00_0000`). `board::Map` now carries a
`mailbox` field (`0` on `board-virt`/`board-pine64`, which have no VideoCore
mailbox), and `pi_fb.rs` reads `board::MAILBOX_BASE` instead of repeating the
constant, so the two addresses can never drift out of sync again. This is
compile-checked groundwork only — `pi_fb` still only compiles under
`--features board-raspi` (`main.rs:36`), so nothing changes for `board-rpi4`,
which still has no boot path (item 1/2's remaining gaps). Live-verified on the
board this driver already targets: `scripts/pi-test.sh` run standalone booted
`-M raspi3b`, and the guest's own serial output still shows
`[pi-fb] framebuffer @ 0x3c100000, pitch 2560, 640x480x32` and
`[mansa] Pi framebuffer splash drawn (640x480)` — byte-identical to the
address milestone 2.45 recorded, proving the refactor changed nothing observable
on the one board it can currently run on.

`board-pine64` is further still from a port, and the map itself says so. Each `Map`
now records the UART *part*, not just its base, so `board pine64` prints
`UART (8250dw)` with the caveat that the PL011 driver does not apply. Its GICv2 CPU
interface sits at `0x01C8_2000` and its GPIO is the Allwinner PIO pin controller at
`0x01C2_0800`; both are recorded in `board.rs` alongside the two addresses this table
pins down.

Item 1 below — the second UART *driver* — is **done** as of milestone 2.724:
`kernel/src/uart_ns16550.rs` drives a DesignWare 8250/16550, and
`board::UART_IS_8250` (derived from `uart_kind`, so the map stays the single source
of truth) picks it as the console at compile time. `board <name>` prints a
`console driver` row naming `pl011` or `ns16550` per board. So the A64's
`0x01C2_8000` now has a code path that would form a character correctly; what it
still lacks is items 2–5 (entry path, framebuffer, DTB, EL3 handoff), which is why
PINE64 stays map-only in the table above.

## Peripheral map differences

| Peripheral | QEMU virt | Raspberry Pi 4 (BCM2711) | PINE64 (Allwinner A64) |
|---|---|---|---|
| Primary UART | PL011 @ `0x0900_0000` | PL011 @ `0xFE20_1000` | **DesignWare 8250** @ `0x01C2_8000` |
| Interrupt ctrl | GICv2 @ `0x0800_0000` | GIC-400 (GICv2) @ `0xFF84_1000` | GICv2 @ `0x01C8_1000` |
| GIC CPU iface | `0x0801_0000` | `0xFF84_2000` | `0x01C8_2000` |
| Timer | ARM generic timer | ARM generic timer | ARM generic timer |
| RTC | PL031 @ `0x0901_0000` | none (I²C RTC or none) | none (PMIC RTC via I²C) |
| Framebuffer | fw_cfg + ramfb | VideoCore **mailbox** | Display Engine 2 + HDMI |
| Firmware cfg | fw_cfg @ `0x0902_0000` | none | none |
| Main DRAM | `0x4000_0000`, 256 MiB (what `-m` gives it) | `0x0000_0000`, 1 GiB (SKUs to 8 GiB) | `0x4000_0000`, 2 GiB (3 GiB SKU) |
| Root `compatible` | `linux,dummy-virt` | `brcm,bcm2711`, `raspberrypi,4-model-b` | `allwinner,sun50i-a64`, `pine64` |

The DRAM row is the only one whose *size* matters, and the only one where **0 is a
real base** — both Pis start memory at 0, with the peripheral window sitting above it.
It is also the one row the machine is allowed to disagree with us about: a peripheral
base is fixed by the silicon, but DRAM size is set by `-m` under QEMU and by which SKU
of a board is in front of you, so the map states a *nominal* figure (the smallest SKU,
since under-claiming loses frames while over-claiming hands the allocator addresses
that are not memory) and a live tree is expected to correct it. Milestone 2.730 carries
the range into `board::resolved()` on that footing: a different size is reported as a
size, never as a map error.

The last row is not an address — it is the **name each machine gives itself** in the
root node of its device tree, and it is what makes every other row checkable. A map
diffed against a tree from a different machine produces confident rows about nothing,
so milestone 2.728 has each map state the names it answers to and `board machine`
report them against the tree's: same machine, other machine, or a tree that names
none. (The Pi 3 answers to `brcm,bcm2837` / `raspberrypi,3-model-b`.)

The interrupt controller occupies **two** windows, listed as two rows above because a
GICv2 driver needs both: the distributor (shared by all cores) and the CPU interface
(banked per core). In a device tree they are the first and second ranges of one node's
`reg`, which is why milestone 2.727 makes them move together or not at all — a
distributor from the machine paired with a CPU interface from the compile-time map
would be an interrupt controller assembled out of two different machines.

## The parts the base-address abstraction does **not** solve

Changing an address is the easy 20%. The honest hard parts:

1. **A different UART *driver*, not just a base.** *(done — milestone 2.724.)* The
   A64's UART is a DesignWare 8250/16550-class part with a different register layout
   than PL011 — so PINE64 needs a new driver behind the same `uart` interface,
   selected per board. The board map handles RPi4 (also PL011) for free; A64 does
   not. `uart_ns16550.rs` is that driver: 16550 registers on a 4-byte stride
   (`reg-shift = <2>`), FIFOs enabled at `console_init`, the baud divisor left to
   firmware, and a **bounded** TX spin so an unclocked UART loses output instead of
   hanging the kernel at its first character. It has never driven real silicon (no
   QEMU machine for `sun50i`), so `uart8250` checks its register discipline against a
   RAM-backed fake register block on every test run — the part a paper port gets
   wrong. The rest waits for hardware.
2. **Boot path / entry.** QEMU loads the ELF directly at `0x4000_0000` via `-kernel`.
   Real boards don't:
   - **RPi4**: the VideoCore GPU runs first, reads `config.txt`, and loads
     `kernel8.img` (a flat binary) to `0x8_0000`. We need a flat-binary build + an
     `armstub` that drops to EL2/EL1 with the DTB pointer in `x0`.
     **Update (3.672): the flat-binary half of this exists now.**
     `kernel/linker-rpi4.ld` links the `board-rpi4` ELF at `0x8_0000` (selected by
     `kernel/build.rs` from `CARGO_FEATURE_BOARD_RPI4`), and
     `scripts/build-rpi4-image.sh` builds it, asserts the ELF's entry point is
     exactly `0x8_0000`, and `objcopy -O binary`s it to `target/mansa-rpi4.img` —
     a compile-level check only (entry point + non-empty binary), never a boot
     claim. The shared `_start` in `kernel/src/main.rs` already drops EL2→EL1
     and captures `x0` for every board (it predates this milestone): it writes
     `spsr_el2`/`elr_el2` directly and `eret`s, code that is only correct if the
     CPU is *already at EL2* when `_start` runs — it does not (and cannot,
     without extra EL3-only setup) also handle being entered at EL3.
     **Update (3.673): the `armstub` question is answered by documentation
     research, not by a boot — no custom `armstub64.bin` is required.** Per
     Raspberry Pi's own firmware boot docs and Trusted Firmware-A's RPi4 port
     docs: `bootcode.bin`/the GPU firmware only loads a file at address `0x0`
     and enters it at EL3 when `config.txt` names an `armstub8.bin` (or the
     `armstub=` key points at one). Absent that file — the case for a plain
     `kernel8.img` with no `armstub8.bin` on the boot partition, which is what
     `scripts/build-rpi4-image.sh` produces — the firmware instead runs its own
     default stub at `0x0`, which already drops to EL2 and jumps straight to
     the loaded kernel image with the DTB pointer in `x0`. That is exactly the
     entry state this kernel's `_start` already assumes (EL2, DTB in `x0`), so
     stock firmware's default path satisfies it with no custom stub.
     **Update (3.712): the unmodified real-hardware `0x8_0000` binary now
     boots to full console init in QEMU's raspi4b model — via a firmware
     BYPASS, not a fix to the address mismatch below.** `-M raspi4b,help`
     confirms the machine exposes no load-address override flag (3.704's
     gap #1, closed in the negative), but QEMU's generic loader device
     (`-device loader,file=target/mansa-rpi4.img,addr=0x8_0000,force-raw=on,
     cpu-num=0` in place of `-kernel`) sets the CPU's reset PC directly to
     `0x8_0000`, skipping `raspi4b`'s boot-stub emulation entirely (confirmed
     by trace: execution starts at `_start`'s own header branch, no
     `0x300`/`0x0` firmware detour). Console banner, MMU init, frame
     allocator, heap, GICv2 and generic timer all print and the machine idles
     stably for 8s with no crash — first time this image has reached console
     output in QEMU. The cost: skipping firmware means the CPU never drops
     from its architectural EL3 reset state, and `_start`'s EL-check
     (`kernel/src/main.rs:97-101`, `CurrentEL != 2` ⇒ "already EL1, continue")
     silently conflates EL3 with EL1 — measured serial output shows `boot
     core online at EL3` from a live `mrs CurrentEL` read, meaning every
     subsequent EL1-scoped register write this run made was actually
     happening at EL3. It did not fault (EL3 can touch lower-EL registers)
     but this is NOT a verified EL1 boot. Full trace, controls, and the EL3
     bug detail: `docs/finding-3.712-qemu-raspi4b-device-loader-load-address-override.md`.
     **Update (3.737): the EL-check bug is fixed.** `_start`
     (`kernel/src/main.rs:97-111`) now branches explicitly — `CurrentEL==2`
     drops to EL1h as before, `CurrentEL==1` continues directly, and anything
     else (EL3, EL0; no supported board's firmware hands off there) parks in
     the same `wfe` loop secondary cores use, instead of falling through
     un-dropped. Re-running finding-3.712's exact loader command against the
     patched binary now produces NO console output (rc=124, silent) where it
     previously printed the full boot banner at EL3; an `-d in_asm` trace
     confirms the CPU takes `cmp x0,#2`→not-taken, `cmp x0,#1`→not-taken,
     `b park_secondary`→`wfe` loop. `-M virt` (all 690 registered suites) and
     `-M raspi3b` (`pi-test.sh`, board-raspi feature) both still boot and pass
     unchanged — neither has ever reached this code at any EL but 1 or 2.
     **Update (3.704): QEMU's `raspi4b` machine model DOES exist on this
     toolchain (QEMU 11.0.50) and was tried — the "no QEMU rpi4 UART path"
     framing below was stale.** Booting the existing `target/mansa-rpi4.img`
     against `-M raspi4b` does not error; an instruction trace shows it first
     runs QEMU's own default boot stub (empirically confirming the EL2/
     DTB-in-x0 entry state this document cited from firmware docs above), then
     starts executing OUR kernel — but at physical address `0x200000`, not the
     `0x8_0000` `kernel/linker-rpi4.ld` links for. That address is correct for
     real RPi4 firmware's own `kernel_address` default (unchanged, still only
     cited not silicon-tested); it is simply not what QEMU's raspi4b `-kernel`
     loader uses. The binary's absolute (non-PIC) addressing then faults on the
     resulting `+0x18_0000` offset, in an unrecoverable exception loop, before
     console init ever runs — no boot, no UART output, but a real, measured,
     arithmetically-confirmed failure mode instead of an untried one. Full
     trace and arithmetic in
     `docs/finding-3.704-qemu-raspi4b-loads-kernel-at-0x200000.md`. GIC-400
     init and the mailbox framebuffer (items 3-4 below) remain entirely
     unexercised via THIS `-kernel` path — the crash happens before either
     could run. (3.712, above: both DO run, but only via the firmware-bypass
     `-device loader` technique and only at the unverified EL3 privilege
     level — not evidence items 3-4 work under a real, firmware-driven
     boot.)
   - **PINE64**: boots through ARM Trusted Firmware (BL31 at EL3) then U-Boot,
     which loads the kernel into DRAM and enters at EL2/EL1. We need a U-Boot image
     wrapper and to honor the handed-in DTB.
3. **No fw_cfg / ramfb.** Our first-pixels path is QEMU-only. RPi4 framebuffer comes
   from the **VideoCore mailbox property interface**; A64 needs a Display Engine 2 +
   TCON + HDMI/DSI driver. Both are real driver projects.
4. **Device discovery.** On hardware we should parse the **flattened device tree**
   (DTB) the firmware passes in `x0`, rather than assuming a fixed map — the board
   module is the interim step toward a DTB-driven map. Two milestones did this in
   halves. **2.725** made the tree *judge* the map: `board check` diffs the
   compiled-in map against the tree `fdt.rs` parsed (UART base *and part*, GIC
   distributor, RTC) and prints a verdict per peripheral, so a wrong constant is
   visible output rather than a dead console. **2.726** made it *drive*:
   `board::resolved()` folds a live tree into the map, and peripheral consumers
   (VFS mtime stamps, the RNG seed, the Asili device-manager entries) read the
   resolved base — every value carrying where it came from, since an address
   without a provenance is what this was supposed to stop shipping. Four rules
   bound the override: only a **live** blob may override (never the bundled
   fixture); a peripheral the walker did not find never zeroes a working address
   (silence is not absence); a base may not move to a node naming a **different
   UART part** (a right address with the wrong driver is a dead console); and the
   **console itself stays compile-time**, because it is brought up before the tree
   is parsed and a bad blob must still have somewhere to report itself. On QEMU's
   `-kernel` path firmware passes no DTB pointer, so both the check and the
   resolution run against the bundled reference `virt.dtb` and say so — a
   self-check against a fixture must not read as one against the live machine, and
   for the same reason that fixture is not allowed to override anything. The
   override branch is therefore exercised by `board resolved test`, against
   synthetic trees. **2.727** closed the last consumer still on constants:
   `gic.rs` now reads both windows from `board::resolved()` at use time, which is
   the peripheral that most needed it — a UART at the wrong base is a dead console
   you notice in a second, an interrupt controller at the wrong base simply never
   interrupts and the machine stops making progress with nothing printed to say
   why. Reading at use time buys correctness at the price of an *ordering*
   requirement (`resolve()` before every access, on every core), so the accessors
   count their own calls and separately count any made while `is_resolved()` is
   still false; `gicbase` prints the tally, and a regression that hoists
   `gic::init()` above resolution shows as a non-zero "before resolve" count
   rather than as silence. The GIC also forced a fifth rule: **distributor and CPU
   interface move together or not at all** — a tree naming exactly one of the two
   overrides neither, since a controller assembled from one machine's distributor
   and another's CPU interface is worse than either source used honestly.
   **2.728** stepped back from the peripherals to the tree as a whole: every rule
   above bounds one *peripheral*, and none of them asks whether the blob describes
   this machine at all. Each `Map` now states the root `compatible` names it
   answers to (last row of the table above), `fdt.rs` reads the root node's own
   `compatible` list, and `board machine` reports the verdict — same machine,
   other machine (named, when the kernel has a map for it), or a tree naming none,
   which is not an accusation. The matcher runs on synthetic roots under
   `board machine test`, because the wrong-machine verdict cannot occur on a QEMU
   virt boot and would otherwise ship unexecuted. It is a *report*, not yet a
   rule: `resolve_from()` still keys only on live-vs-fixture, so the same kernel
   that says "other machine" would resolve off that machine's tree regardless.
   Making cross-machine the sixth rule is **2.729**.
   **2.729** supplied that consequence. Rule 6 — *a tree from another machine
   overrides nothing* — is asked once, before any row is considered, because a
   foreign blob passes the row-level rules easily: the parts match (two boards
   can both use a PL011), both GIC windows are named, every address is plausible.
   They are real addresses, just not real *here*. On `Machine::Other` the map is
   returned untouched and the refusal is recorded (`Resolved::wrong_machine`), so
   `board resolved` says so instead of printing a table that looks ordinary.
   `Machine::Unstated` is deliberately **not** a refusal: a tree naming no machine
   contradicts nothing, and treating silence as an accusation would disable
   resolution on every blob whose root omits `compatible`. Rule 1 still runs
   first, so a foreign *fixture* is refused for being a fixture — the operator is
   owed the true reason, not whichever refusal fires last. All four cases are
   proved against synthetic roots (`board resolved test`, 14/14), the only way to
   execute them on a machine that never receives a live DTB.
   **2.730** extended resolution past the peripherals to the largest resource on
   the board. `fdt.rs` had parsed the tree's `/memory` node for several milestones,
   but `ram_base`/`ram_size` were printed at boot and consumed nowhere; each `Map`
   now declares its own DRAM range (table above) and `board::resolved()` carries it
   under rules 1–6 exactly as it carries a peripheral — a fixture may not size our
   memory, and neither may another machine's tree. Two things had to be said
   differently on this row. First, **0 is a real base** for DRAM, so what tells the
   resolver "the walker found no `/memory` node" is a *size* of 0, not a base of 0;
   and a size of 0 is silence, not a claim of zero memory. Second, the size is the
   one figure a healthy machine is expected to disagree with the map about (`-m
   384M`, a 2 GiB SKU), so `board check` prints it but does **not** count it as a
   disagreement — calling that an error would train the operator to ignore the word
   on the rows where it means something. That leniency then needed a bound, which
   is rule 7: **impossible memory is refused, not believed.** RAM is the only row
   whose consumer is the frame allocator, so a wrong MMIO base costs one device
   while a wrong RAM range hands out addresses that are not memory. A range that
   wraps the address space, one smaller than the resident kernel already occupies,
   one larger than `paging.rs`'s 39-bit identity map can address, or a base/size
   that is not a whole number of 4 KiB frames leaves DRAM on the map *and* records
   why (`Resolved::ram_refused`), because a refusal owes the operator a reason
   where silence does not. The heap stays compile-time for the same reason the
   console does — it is placed before the tree is parsed — so the resolved range is
   what the Phase 3 frame allocator will take, and `board resolved` says that out
   loud rather than implying a consumer that does not exist yet. Nine new cases,
   23/23.
   **2.731** made that consumer real. Everything 2.726–2.730 built was, until this
   point, decorative in the one place it mattered: `main.rs` still sized
   `frame::init()` from a raw `fdt::ram_end_or()` read, so the largest consumer on
   the board walked around all seven rules — a fixture tree, a foreign machine's
   tree, an impossible `/memory` claim, each of them refused by the resolver and
   then read straight from the tree anyway. The allocator now takes
   `board::resolved().ram_end()`, and `fdt::ram_end_or` was **deleted** rather than
   left standing, because it was the last route by which any future consumer could
   size itself from the tree without passing the rules. The raw figures stay in
   `fdt::info()`, which reports what the tree *said* — a different question from
   what we trust. The second half is that feeding the allocator a resolved range
   exposed two truncations it had always applied in silence: the identity map's
   ceiling and its own static bitmap both shrink whatever range they are given, so
   a `-m 384M` boot reported 256 MiB and read exactly like a machine that has
   256 MiB. A bound the operator cannot see is indistinguishable from memory that
   is not there. `frame::plan` is now a pure function naming which limit bound the
   range and how many frames it cost, reported on the boot line and by `frame`; the
   ceiling is read from `paging::NORMAL_MAP_END`, declared by the file that *builds*
   the map rather than copied into the file that must respect it. Ten cases run the
   bounds against synthetic ranges (`frame test`), since this machine boots with one
   range and it is a range both bounds accept — and writing them found the honest
   shape of the map bound: offering 2 GiB behind a 1 GiB window is bounded by the
   *bitmap*, not the map, because 65 536 frames is only 256 MiB, so the map binds
   first only when DRAM runs past the window with under 256 MiB of window left.
   That is the residue **2.732** takes up: the bitmap cap is a constant older than
   the board maps, and it is smaller than every declared DRAM in the table except
   virt's.
   **2.732** took it up, and the shape of the fault is worth stating plainly,
   because it is the shape most compile-time constants in a kernel have: the
   constant was not wrong on the machine we boot. `BITMAP_WORDS = 1024` is 65 536
   frames is 256 MiB, and `board-virt` declares 256 MiB, so the two agreed exactly —
   which is precisely why it survived being read many times. It agreed by
   coincidence, having been written before `board.rs` existed; on `board-rpi4`
   (1 GiB declared) or `board-pine64` (2 GiB) it would have handed most of the
   board's own nominal DRAM back before a single live tree was read, with the map
   and the allocator disagreeing at compile time and nothing in the build to say so.
   `MAX_FRAMES` is now `min(board::ACTIVE.ram_size, NORMAL_MAP_END - ram_base)`:
   what this board says it has, clamped to what the MMU can translate as Normal
   memory, since bits for frames past the identity map are BSS spent on addresses we
   can never hand out. On virt the clamp is slack and the capacity is bit-for-bit
   what the constant gave; on pine64 the clamp binds and stops us reserving a
   gibibyte of untranslatable addresses. `BOUND_BITMAP` no longer quotes a figure —
   it cannot, being per-board — and states instead the only thing it now means: *the
   tree offered more DRAM than this board declares.* Because the figure left the
   message, `frame` prints **both** ceilings on every boot whether or not either
   binds; 2.731's point was that an invisible bound is indistinguishable from absent
   memory, and a bound only readable *after* it fires is barely better. The
   self-test's synthetic cases were rewritten in multiples of the bitmap's capacity
   rather than in megabytes: "512 MiB overflows the bitmap" was a fact about
   `board-virt` that would have silently stopped being one — and stopped testing
   anything — on a build with a larger bitmap. Two cases were added against the
   **real** board map and the **real** ceiling, both of which fail if the constant
   returns. 12/12.
   The residue 2.732's clamp exposed by binding was: on pine64 the allocator
   managed 1 GiB of a declared 2 GiB, not because the bitmap was small but
   because `paging.rs` mapped exactly one hard-coded 1 GiB Normal block on
   every board — the same species of board-blind constant, one layer down,
   and the tightest limit left. **2.733 closed it (done, commit `6647147`):**
   `paging::window(ram_base, ram_size)` now builds the identity map's Normal
   run from `board::ACTIVE`'s own declared range — as many 1 GiB blocks as
   the range spans, still Device everywhere else, still 2 MiB granularity on
   the Pi where peripherals sit inside the first GiB — so `NORMAL_MAP_END` is
   per-board and 2.732's clamp goes slack on every board in the table instead
   of binding on pine64 alone. Proved with a self-test over every known board
   map (`window` re-derived per board, checked for full coverage), not just
   the one this kernel boots.
   **2.734-2.739** then carried the same "stop being board- or workload-blind
   about memory" arc past the identity map and into the kernel heap itself —
   naming the arena's bound, its largest customer (thread/process stacks),
   the panic path for the small allocators, and finally a real `dealloc`
   (free list, coalescing) so the allocator stops decaying under uptime. All
   ✅ Done per `docs/07-phase-0-plan.md`; not re-narrated here since none of
   it is board-address work — see that table for the detail.
5. **RAM size, caches, secure world.** Real DRAM sizing (from the DTB), correct
   cache/MMU attributes on real caches (QEMU is forgiving), and entering from EL3
   firmware all need attention that emulation lets us skip.

## Progress: raw Image boot + live DTB (milestone 2.29)

Step 1 below is **done for QEMU**. The kernel now carries a 64-byte **arm64 Linux
Image header** and is linked at **0x4020_0000** (QEMU virt reserves the first 2 MiB
of DRAM for the DTB and loads a raw `Image` there). Built with
`scripts/build-image.sh` and booted via `scripts/run-image.sh`, QEMU's bootloader
passes a **live device tree** in x0, which `fdt.rs` parses — verified by booting
`-m 512M` and reading 512 MiB back from the live blob. The same Image header and
load convention are what U-Boot and real boards expect, so this is genuine progress
toward hardware, not just a QEMU trick. (The default `-kernel <elf>` path still
works and falls back to the bundled reference DTB, since ELF entry gets x0=0.)

## Progress: second board — QEMU raspi3b (milestone 2.31)

AfriSovera now **boots on a second machine**. A build-time board selection
(`--features board-raspi`, default `board-virt`) picks a Raspberry Pi (BCM2837)
peripheral map — PL011 UART @ `0x3F20_1000` — a board-specific linker script, and a
minimal Pi `kernel_main` (MMU off, UART-only banner, then park). Built + booted via
`scripts/run-raspi.sh` on `qemu-system-aarch64 -M raspi3b`, the kernel prints its
banner over the Pi's PL011.

Two lessons from the bring-up:
- **Load address:** with an arm64 Image header (text_offset 0), QEMU loads the Image
  at RAM base + 2 MiB on the Pi as well — so the Pi build links at `0x20_0000`, not
  the `0x8_0000` the raw-firmware convention suggests.
- **FP/SIMD:** `cpu::enable_fp_simd()` must run before any NEON the compiler emits for
  `memcpy`/`memset`/formatting — otherwise the first such instruction is an FP-access
  trap (ESR EC 0x7). This applies on every board.

**Update (milestone 2.44): the Pi now runs with the MMU on + a live heap.** The
`board-raspi` build enables the MMU using a BCM2837-correct map. The Pi's first GiB
is RAM (Normal cacheable) *except* the peripheral window `0x3F00_0000..0x3FFF_FFFF`
(Device) — the inverse of QEMU virt — so `paging.rs` uses a board-gated **level-2,
2 MiB-granule** table for the Pi (blocks 0..504 Normal, 504..512 Device; `L1[0]`
points at it). With RAM cacheable, atomics work and the kernel heap allocates
(`heap OK: sum of squares 1..8 = 204`). Remaining Pi work: the **BCM2837 legacy
interrupt controller** (@ `0x3F00_B200`, not a GIC), the ARM/system timer, and the
**VideoCore mailbox framebuffer** — none of which QEMU virt's drivers cover.

**Update (milestone 2.45): the Pi now has a framebuffer.** `kernel/src/pi_fb.rs`
drives the **VideoCore mailbox** property interface (mailbox `0x3F00_B880`, channel 8)
to allocate a 640×480×32 framebuffer and reads back its base + pitch, then draws the
AfriSovera splash. Verified on `-M raspi3b` by screenshot (`framebuffer @ 0x3c100000`).
Note: QEMU's Pi framebuffer is **BGR**, so the pixel writer swaps R/B. Board-gated;
the virt `ramfb` path is untouched.

**Update (milestone 2.51): the Pi has a timebase.** The `board-raspi` build reads the
BCM2837 **system timer** (free-running 1 MHz counter at `0x3F00_3004`) and confirms it
advances — a real hardware clock. What's left for interrupt-driven scheduling on the
Pi: (1) map the **ARM local peripherals** at `0x4000_0000` (per-core timer + IRQ
routing) — currently outside the Pi's 0–1 GiB identity map, so L1 entry 1 must be
added as Device; (2) drive the **BCM2837 legacy interrupt controller** at
`0x3F00_B200` (enable/ack registers — *not* a GICv2, so `gic.rs` doesn't apply); or
alternatively use the **ARM generic timer** (architected, same as virt) with its
interrupt routed via the local controller. Then wire a periodic tick into the
scheduler. This is the next Pi milestone.

**Update (2.58): the Pi now takes real timer interrupts.** The Pi3 has no GIC —
the ARM generic timer routes through the BCM2836/2837 **local interrupt
controller at 0x4000_0000** (per-core). The kernel maps that region Device (a new
`L1[1]` block; it was previously unmapped), sets `VBAR_EL1`, arms the generic
timer (`CNTP_TVAL/CTL`), and routes CNTPNSIRQ to the Core0 IRQ line by writing the
Core0 timer-int-control register at `0x4000_0040` (bit 1). A board-specific IRQ
handler reads the local controller's Core0 IRQ-source register at `0x4000_0060`
(no GIC IAR) and re-arms CNTP, which clears the level-sensitive line (no EOI).
`scripts/pi-test.sh` boots raspi3b and asserts `[pi] timer tick 1..N`. QEMU's
raspi3b models this local-timer routing; on real silicon the same registers apply.

**Update (2.59): the Pi timer IRQ now drives the scheduler.** The Pi `kernel_main`
registers its boot context as the idle thread and spawns two kernel threads, and
the Pi IRQ handler calls `sched::tick()` after re-arming CNTP, so each interrupt
round-robins idle → A → B. It reuses the same `sched.rs` machinery (forged frames,
`cpu_switch`, `thread_trampoline`, context-switch-inside-the-IRQ) as virt; the Pi
is single-core so the global `SCHED` keeps its one-accessor invariant. `pi-test.sh`
asserts `[pi] thread A/B tick`.

**The Pi timer story is complete**: MMU + heap + framebuffer + polled timebase +
interrupt-driven ticks + preemptive scheduling. One codebase now boots, renders
graphics, reads a hardware clock, and runs a preemptive multitasking kernel on two
machines with entirely different interrupt controllers (GICv2 vs BCM local intc) —
the board abstraction proven deep. Still Pi-partial vs virt: the Pi runs EL1
kernel threads, not yet EL0 user processes with preemption (a later Pi step).

## Suggested order when Phase 3 starts

1. Flat-binary build target + `armstub`/U-Boot wrapper; get **UART "hello"** on a
   real board (RPi4 first — same PL011). Flat-binary link target: done (3.672).
   RPi4 `armstub`: research says none needed, stock firmware's default EL2
   entry already matches (3.673) — still boot-unverified. PINE64's U-Boot
   wrapper is untouched.
2. ~~Parse the DTB into `board::` at runtime (replace the compile-time map)~~ —
   both halves are written: comparison (2.725, `board check`) and override
   (2.726, `board::resolved()`), and the interrupt controller reads the resolved
   map too (2.727), and the tree is now checked for *whose* machine it describes
   before any of that is trusted — reported by 2.728 (`board machine`) and
   enforced as rule 6 by 2.729, where a tree naming another machine overrides
   nothing at all while a tree naming none still may. 2.730 took the map past the
   peripherals to **DRAM** itself, under a seventh rule the peripheral rows never
   needed: an impossible `/memory` claim is refused rather than believed, since
   this is the one row whose consumer is the frame allocator. That consumer stopped
   being hypothetical in 2.731: `frame::init()` now sizes itself from
   `board::resolved()` and the raw tree read was deleted, so no path is left from
   the blob to the allocator that skips the rules — and the two limits that shrink
   the range on the way in now say which one bound it. 2.732 then took the second of
   those limits off its constant: the bitmap is sized from `board::ACTIVE`, so it
   covers what this board declares (clamped to what the map can translate) and its
   bound reports the one case it was meant for. Two deliberate exclusions
   remain, which more code does not fix: the **console** and the **heap** stay
   compile-time until real silicon, because both are set up before the tree is
   parsed and a bad blob must still have somewhere to report itself.
   Like step 5, the code exists; what it needs is a machine that actually
   hands us a live DTB.
3. GIC + generic timer on hardware (mostly addresses + frequency). Research
   says both halves are already covered by existing board-agnostic code —
   GIC-400 implements the GICv2 architecture register-for-register (ARM
   CoreLink GIC-400 TRM: same `GICD_`/`GICC_` register set `gic.rs` already
   uses via `board::resolved()`), and the BCM2711's `CNTFRQ_EL0` is programmed
   by the firmware's armstub before the kernel runs, which `timer.rs` already
   reads at runtime instead of assuming a fixed frequency (3.674) — still
   boot-unverified on real hardware. **Update (3.740): the "no ... QEMU rpi4
   machine model in this tree" half of that claim is stale** — 3.704 found
   QEMU 11.0.50's `raspi4b` machine model does exist, and 3.712 used it (via
   `-device loader`, bypassing firmware) to run this exact GIC-400/timer init
   code path once, successfully, printing both. That run was not a verified
   EL1 boot (measured at EL3, per 3.712/3.737's finding), and 3.737's EL-check
   fix now parks `_start` before GIC/timer init is ever reached on that same
   command — so this code path has gone from "ran once, unverified" to
   "provably not reached at all" via the loader route, and remains untried via
   any firmware-driven boot. Item 3 is still open; only the "no machine model
   exists" framing was wrong.
4. Mailbox framebuffer (RPi4) for on-screen output. `board::Map` now carries the
   mailbox base per board (BCM2711 `0xFE00_B880`, same offset as BCM2837 rebased
   onto the BCM2711 peripheral window) and `pi_fb.rs` reads it via
   `board::MAILBOX_BASE` instead of a hard-coded BCM2837 constant (3.675) —
   compile-checked groundwork only, still gated to `--features board-raspi`
   until `board-rpi4` has a boot path at all (items 1/2 above).
5. ~~A64 UART driver~~ (written, 2.724 — needs silicon, not more code) + DE2
   framebuffer for PINE64.
6. SD/eMMC + USB — the long tail.

Until then, `board.rs` keeps the kernel honest about *where* its peripherals are,
and this document keeps us honest about how much is left.

## SMP and the MMU-off secondary (honest limits)

On QEMU virt the kernel brings a **second CPU core online via PSCI** (milestone
2.52), has it **enable the MMU with the primary's page tables** (2.54), and hands
it a **shared spinlock-guarded job queue** it drains in parallel (2.53). All are
verified in `smp-test.sh`.

Milestones 2.52-2.53 first ran the secondary with the **MMU off**, which was a
QEMU-only shortcut:

- **Exclusive monitor.** The job-queue spinlock uses `compare_exchange`
  (LDXR/STXR). Load/store-exclusive to *Device* memory — which is how an MMU-off
  core sees DRAM — is CONSTRAINED UNPREDICTABLE on real ARMv8; the monitor may
  never succeed. QEMU's TCG tracks exclusives by address regardless, so it worked
  there.
- **Cache coherency.** The primary runs MMU-*on* and sees the shared queue as
  Normal cacheable; an MMU-off secondary sees it as Device (uncached). On silicon
  those mismatched attributes are incoherent — one core can miss the other's
  writes. QEMU models coherent memory, so it worked there too.

**Milestone 2.54 resolves this on virt.** Before the secondary touches any shared
memory it calls `paging::enable_mmu_secondary()`, which programs its
`MAIR_EL1`/`TCR_EL1`/`TTBR0_EL1` to match the primary (from a shared `TCR_VALUE`
const — one source of truth) and sets `SCTLR_EL1.{M,C,I}`. The primary's identity
map covers all DRAM (VA==PA), so the core keeps running across the switch; page
tables are inner-shareable cacheable, so they are coherent for the second core.
After this the exclusive monitor and cacheable atomics are **well-defined in
mechanism**, not QEMU-lenient. The boot log shows `[core 1] online` →
`[core 1] MMU on (secondary tables active)` → the queue draining.

What still differs on a **real board**: the primary's page tables must actually
map that board's RAM and peripherals (the DTB-driven map, above), and secondary
release/spin-table vs PSCI entry differs per platform — but the *mechanism*
(secondary adopts the primary's translation, then does coherent shared-memory
work) is now proven, not stubbed.

**Milestone 2.55 adds a real per-core scheduler.** With the MMU on, core 1 sets
its own `VBAR_EL1`, brings up its own GIC CPU interface (`gic::init_secondary()`
— the distributor is shared and already enabled; PPIs like the timer's INTID 30
are banked per-core), arms its own generic timer (`CNTP_*` are per-core system
registers), and round-robins two kernel threads on static stacks under its own
timer IRQ. The IRQ handler dispatches by `MPIDR` — core 0 drives `SCHED`, core 1
drives its isolated `SCHED1` — so the two schedulers never share mutable state
and need no cross-core lock. Console output *does* need one: `uart::_print` now
takes a real cross-core spinlock (the previous "lock" only masked local IRQs), so
the two cores' messages serialize instead of interleaving.

**Milestone 2.56 adds cross-core signalling.** `gic::send_sgi(target_core, intid)`
writes `GICD_SGIR` (a CPU-target bitmask plus a software-generated-interrupt id,
0-15) so the primary can interrupt core 1 on demand. Core 1's IRQ handler now
treats `INTID < 16` as an SGI, counts it, and reports it. The primary spaces its
sends because same-id/same-source SGIs coalesce while one is pending, so all
three arrive as distinct signals. This is the primitive behind cross-core IPC,
remote scheduler kicks, and TLB shootdown — none of which are built yet, but the
delivery path is proven.

**Milestone 2.57 adds a cross-core IPC message channel** on top of the SGI. The
primary writes a payload into a shared lock-guarded mailbox, fires SGI id 1, and
core 1's handler reads it, computes a result, and publishes it back with an ack
sequence the primary waits on — a full round trip (100→200, 200→400, 300→600).
The ack-wait serializes sends, so no SGI coalescing.

This closes the SMP arc (2.52-2.57): a second core comes online, adopts the
primary's translation, does shared-memory work under a spinlock, runs its own
preemptive scheduler, and now exchanges both signals and data with the primary.

What is still open on SMP (future, not blocking): threads are statically pinned to
a core (no migration / load-balancing across a unified run-queue), and the IPC
channel carries a single word with a derived reply — not the general
capability-routed multi-endpoint IPC the EL0 processes use. The mechanism, though,
is proven end to end. Subsequent milestones pivot to breadth (Raspberry Pi
interrupt-driven ticks first) to keep the OS well-rounded rather than deepening
SMP further right now.

## SGI delivery reaches EL0, and it broke the Pi build for ~190 ticks (2.991-3.182)

This section was missing from this doc even though the code landed and was
fully repaired months ago (as of milestone 3.182); recorded here at 3.670 so
the blueprint stops disagreeing with `git log` about what Phase 3 HW prep has
actually been through.

**What shipped.** Milestones 2.56/2.57 above proved cross-core SGI delivery
and IPC to core 1's *kernel* thread. 2.991-2.994 extended the same mechanism
to an EL0 *user-space* process: `crate::c1_on_sgi(intid)` (declared in
`main.rs`) is called from two IRQ entry points — the EL1 path
(`exceptions.rs`'s `rust_irq_handler`) and the new EL0 path
(`process.rs`'s `rust_irq_from_el0`) — so a process running at EL0 now sees
the same inter-core signal a kernel thread does. `scripts/sgi-el0-probe-2991.sh`
falsified first (exit 10, EL1 delivered / EL0 not) then flipped green (exit 0)
once the EL0 call site landed, and suite 685 `sgi-el0` registered it.

**How it broke `board-raspi`.** `c1_on_sgi`'s definition is gated
`#[cfg(not(feature = "board-raspi"))]` — the Pi3 (`board-raspi`) has no GIC at
all (BCM2837 legacy local interrupt controller, see milestone 2.58 above), so
the GICv2-shaped SGI plumbing this function assumes does not apply there. The
**EL1** call site sits inside `rust_irq_handler`, which carries the identical
cfg gate, so definition and caller were configured out *together* — invisible
for 400+ milestones. The new **EL0** call site (`process.rs`) carried no cfg
at all, so under `--features board-raspi` the call compiled in against a
function the same build had compiled out: `error[E0425]: cannot find function
`c1_on_sgi`` (`kernel/src/process.rs:696`). Exactly one registered suite
(`scripts/pi-test.sh`) builds `board-raspi`, so no other suite — including
`sgi-el0` itself, which builds the default `board-virt` features — could see
the break. It first showed up as `pi FAIL` inside walk 2995 (108 s: a failed
`cargo build` plus the suite's 60 s marker-wait timing out against a guest
that never booted, not a load-sensitivity ceiling as first misdiagnosed at
finding-3.120 and corrected at `docs/finding-3.121-sgi-arm-breaks-raspi-build.md`).

**The fix (landed, commit `6ab91a8`, milestone 3.181).** One statement-position
`#[cfg(not(feature = "board-raspi"))]` on the `process.rs` call site — not
un-gating the declaration (that would compile GICv2 mailbox handling into a
GIC-less build) and not a raspi-only stub body (dead code no suite exercises)
— restores the exact pre-2.993 raspi behaviour (acknowledge, EOI, drop) while
leaving the EL0 delivery path intact everywhere it applies. Falsified first,
not asserted: the unpatched raspi build reproduced `E0425` (rc=101), then the
same unedited commands after the patch gave raspi build rc=0, default build
rc=0, and `scripts/pi-test.sh` rc=0 (`all 6 markers seen after 2455 ms`,
`PI TEST PASSED (raspi3b: MMU + heap + framebuffer + preemptive scheduler
under timer IRQ)`); `sgi-el0` re-run standalone still rc=0, confirming the
EL0 arm itself was untouched. **3.182** additionally hardened
`scripts/pi-test.sh` with a build-failure guard (a failed `cargo build` now
ends the suite instead of falling through to objcopy/grade whatever binary
happened to already sit in `target/` — a hazard measured live during walk
2995: a build death produced an IMG, QEMU ran it, and the failure was
reported under the kernel's name rather than the build's).

**Current status: GREEN, no open gap in this chain.** Both patches
(`docs/repair-3124-c1-on-sgi.patch`, `docs/repair-3125-pi-buildguard.patch`)
are landed, not merely proposed. Full derivation, falsification evidence, and
the sequencing discipline used to land a kernel fix safely *behind* a live
685-suite regression walk are in `docs/finding-2.997-pi-board-raspi-cfg.md`,
`docs/design-2.998-raspi-sgi-cfg-fix.md`, and
`docs/finding-3.121-sgi-arm-breaks-raspi-build.md`.

**The standing lesson this episode adds to the "what the base-address
abstraction does not solve" list above:** exactly one registered suite
compiles `--features board-raspi`, so a change to any `#[cfg(feature =
"board-raspi")]`-adjacent call site is a build configuration no other green
suite can see land or break. Before landing a call to a `board-raspi`-gated
(or -ungated) item, build **both** feature configurations a registered suite
actually compiles (`default` and `--no-default-features --features
board-raspi`) — not just the one the change was written against.
