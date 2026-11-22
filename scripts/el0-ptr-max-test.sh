#!/usr/bin/env bash
# el0-ptr-max: el0-ptr on a CPU with PAN (QEMU -cpu max), in the quick checks' CORE (architect, 22 Nov 2026), so the
# kernel's ordinary EL0 paths (the AI and SPEAK copies, the RTC driver, the window calls) meet PAN on every push. It
# also requires "pan: on" and no "PAN stopped" line. ARM only (arm-only.txt).
cd "$(dirname "$0")/.."
EL0PTR_CPU=max exec bash scripts/el0-ptr-test.sh
