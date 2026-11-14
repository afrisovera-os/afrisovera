# Performance benchmark (suites `perf` and `pc-perf`)

Started 13 Nov 2026 by the performance and server engineer. Owner's mandate: keep AfriSovera fast on a 4 GB RAM
machine, and make it good for servers and programming.

## What it measures

`scripts/perf-test.sh` (ARM phone and board kernel) and `scripts/pc-perf-test.sh` (PC kernel, 4 GB of RAM, the
owner's laptop) boot the kernel in QEMU, type a fixed list of shell commands and read the numbers back
(`scripts/perf-bench.py`):

| Metric | What it is |
|---|---|
| `boot.kernel_ms` | the kernel's own clock when the shell prompt appears |
| `mem.idle_total_kib` | RAM in use at the idle prompt: the kernel image plus the physical frames taken |
| `mem.idle_heap_kib`, `mem.idle_frames_kib`, `mem.heap_arena_kib` | the parts of that |
| `mem.apps3_peak_heap_kib` | how far the kernel heap's high-water rises while three apps each draw a window |
| `mem.apps3_left_kib` | what those three apps leave behind once they exit (a leak shows here) |
| `image.*` | the kernel's own RAM: code, read-only data, data and zeroed data, in KiB |
| `cpu.sha256_us`, `cpu.sha256_mibs` | `bench`: SHA-256 over 2 MiB |
| `draw.*` | the drawing timers of `fbcon time`, `fbcon blit` and `fbcon wm` |
| `timer.sleep50_over_ms` | how late `sleep 50` wakes |
| `net.sink_*` | bytes a peer pushes into `net tcp sink` in 6 seconds |
| `boot.host_s` | seconds the runner took to reach the prompt (the runner's speed, not graded) |

## Why the numbers repeat on any machine

QEMU runs with `-icount shift=0`: the guest clock moves one nanosecond per guest instruction, as on a 1 GHz
in-order core. A time the kernel measures itself therefore counts instructions, so a slow the cloud runner or GitHub
runner makes the run longer, not the numbers worse. The network rows depend on the host clock (the peer is a
host program) and are graded loosely.

## Budgets and baseline

- `budgets.tsv`: the hard limits (for example the PC idles in 256 MiB or less) and, per metric, how much worse
  than the baseline a run may be before it is red.
- `baseline.tsv`: the last kept numbers. A run that is slower or bigger than its tolerance fails, so a slowdown is
  caught the day it lands.
- `history.tsv`: every kept run, oldest first.

## Running it

In the cloud only (never a full run on the PC): dispatch `perf,pc-perf` on your CI mirror (`docs/release.md`,
"The CI mirrors"), or put them in `ONLY` in `cloud/job.sh` for the cloud runner. Each run prints its rows as `HIST` lines
and, on GitHub, in the run summary.

## Moving the baseline

After a change that is meant to move a number (a speed-up, or an agreed cost), copy that run's rows into
`baseline.tsv` and append them to `history.tsv` in the same commit, and say why in `SESSION_PLAN.md`. A number
that got worse by accident is fixed, not written into the baseline.

## Today's numbers and known gaps (13 Nov 2026)

- PC kernel boot 1.70 s in the debug build the USB stick ships (5.59 s on master: the SATA driver waited a second per
  empty port), 0.55 s in the release build; ARM 0.30 s.
- Idle RAM: PC 89 MiB, ARM 32 MiB, almost all of it fixed reservations inside the kernel image (the PC's 64 MiB
  frame pool and 9 MiB drawing canvas, the ARM kernel's 24 MiB for app code and stacks).
- The release build is 8 to 11.6 times faster at SHA-256 and 4.2 to 4.5 times faster at drawing a desktop frame
  than the debug build (PC full frame at 1280x800: 39.3 ms debug, 9.4 ms release). `perf-release` and
  `pc-perf-release` measure it.
- No disk throughput number yet: there is no read or write timing command (asked of the driver engineer).
- On GitHub, a run that includes a perf suite keeps `build/perf` as the artifact `perf` for 30 days, passing or not.
