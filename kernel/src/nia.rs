//! Nia — the WebAssembly application runtime.
//!
//! A real (if compact) WASM interpreter: locals, structured control flow
//! (block/loop/if/br/br_if/return), i32 arithmetic + comparisons, calls between
//! functions, and **linear memory** (data segments + load/store). Crucially, each
//! app runs with a **capability set**: an imported host function executes only if
//! the app holds the matching capability — otherwise the runtime traps. This is
//! the whole security thesis applied to apps: *a WASM app can reach a service
//! only through a host import it was granted.*
//!
//! (The interpreter currently runs in a supervisor context; hosting the
//! interpreter itself inside a userspace EL0 process is a further step. The
//! capability gating of host imports — the security-relevant part — is real.)

use core::cell::Cell;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};

// Types and defined functions per module (16 each until claude/nia-a1a3, OS_REQUESTS item 10 A1). Module keeps
// them in narrow element types (u8 type index, u32 code offset and length, u16 counts), each stored through a
// checked conversion, so the Module frame grows by about 640 bytes, not 2.9 KB.
const MAX_TYPES: usize = 32;
const MAX_FUNCS: usize = 128;
const MAX_IMPORTS: usize = 16; // imported functions per module (8 until claude/nia-imports)
const STACK_CAP: usize = 256;
const LOCALS_CAP: usize = 32;
const MAX_GLOBALS: usize = 16;
const CTRL_CAP: usize = 32;
const MEM_SIZE: usize = 65536; // 1 WASM page
// Nia guard (claude/nia-guard): how many WASM functions may be active at once
// (`run` counts as one). Each level costs `invoke` + `run_body` frames on the
// KERNEL stack, so recursion without a limit overran it. Host calls are not
// counted. Apps built with nia-apps need at most 8. `wasmfuzz` measures the
// frame per level and the stack left at this depth.
const MAX_CALL_DEPTH: usize = 16;

// Capabilities a Nia app may hold (which host imports it may call).
const CAP_PRINT: u32 = 1 << 0; // host "print"(i32)
const CAP_PUTC: u32 = 1 << 1; // host "putc"(i32)
const CAP_DRAW: u32 = 1 << 2; // host window/draw functions
const CAP_FILE: u32 = 1 << 3; // host filesystem functions
const CAP_TIME: u32 = 1 << 4; // host clock
const CAP_INPUT: u32 = 1 << 5; // host keyboard input
const CAP_AI: u32 = 1 << 6; // host on-device AI (Asili AI)
const CAP_VOICE: u32 = 1 << 7; // host voice / text-to-speech (Asili Voice)
const CAP_NET: u32 = 1 << 8; // host networking (virtio-net: ping / DNS)
const CAP_RANDOM: u32 = 1 << 9; // host random number generator
/// What an app outside the integrity manifest gets (appget's sideloaded apps, architect's change B,
/// claude/appget): no file, net, AI or voice. Manifest apps keep CAP_ALL.
pub const CAP_SIDELOAD: u32 = CAP_PRINT | CAP_PUTC | CAP_DRAW | CAP_TIME | CAP_INPUT | CAP_RANDOM;
const CAP_ALL: u32 = CAP_PRINT
    | CAP_PUTC
    | CAP_DRAW
    | CAP_FILE
    | CAP_TIME
    | CAP_INPUT
    | CAP_AI
    | CAP_VOICE
    | CAP_NET
    | CAP_RANDOM;

// Host function ids resolved from import names.
const HOST_PRINT: u8 = 0;
const HOST_PUTC: u8 = 1;
const HOST_WIN_CREATE: u8 = 2;
const HOST_WIN_FILL: u8 = 3;
const HOST_WIN_PRESENT: u8 = 4;
const HOST_FILE_WRITE: u8 = 5;
const HOST_FILE_READ: u8 = 6;
const HOST_TIME: u8 = 7;
const HOST_GETKEY: u8 = 8;
const HOST_WIN_TEXT: u8 = 9;
const HOST_AI: u8 = 10;
const HOST_SPEAK: u8 = 11;
const HOST_NET: u8 = 12;
const HOST_RANDOM: u8 = 13;
const HOST_PRINTQ: u8 = 14; // host "print64"(i64) -> prints a 64-bit integer
const HOST_POLLKEY: u8 = 15; // host "pollkey"() -> the next key or -1, never waits (claude/nia-poll)
const HOST_SLEEP: u8 = 16; // host "sleep_ms"(ms) -> the ms slept, at most SLEEP_MAX_MS per call
// The cell window calls (docs/design/afriui.md section 25; the codes and packing are in crate::desk::cell_abi).
const HOST_CELL_OPEN: u8 = 17; // cell_open(min_cols, min_rows) -> 1 or an error
const HOST_CELL_PUT: u8 = 18; // cell_put(win, col, row, ptr, len) -> cells written or an error
const HOST_CELL_CURSOR: u8 = 19; // cell_cursor(win, col, row, shape) -> 0 or an error
const HOST_CELL_SIZE: u8 = 20; // cell_size(win) -> cols << 14 | rows, or an error
const HOST_CELL_EVENT: u8 = 21; // cell_event(timeout_ms) -> an event, 0 (no event in time) or an error
// win_text_run(win, x, y, ptr, len, color): one row of text in one call (OS_REQUESTS item 10 A3).
const HOST_WIN_TEXT_RUN: u8 = 22;
/// The most characters win_text_run draws: one row of the widest window (CW / 8); a longer run is cut, not refused.
const TEXT_RUN_MAX: usize = 30;
const HOST_UNKNOWN: u8 = 255;

/// Owner id shown on windows a WASM app creates via the compositor.
const NIA_OWNER: usize = 9;

// --- Module A: compute sum(1..10)=55 via a loop (needs "print"). ------------
#[rustfmt::skip]
static WASM_SUM: &[u8] = &[
    0x00,0x61,0x73,0x6D, 0x01,0x00,0x00,0x00,
    0x01,0x0D,0x03, 0x60,0x01,0x7F,0x00, 0x60,0x01,0x7F,0x01,0x7F, 0x60,0x00,0x00,
    0x02,0x0D,0x01, 0x03,b'e',b'n',b'v', 0x05,b'p',b'r',b'i',b'n',b't', 0x00,0x00,
    0x03,0x03,0x02, 0x01, 0x02,
    0x07,0x07,0x01, 0x03,b'r',b'u',b'n', 0x00,0x02,
    0x0A,0x36,0x02,
      0x2B, 0x01,0x02,0x7F,
        0x41,0x01, 0x21,0x01, 0x41,0x00, 0x21,0x02,
        0x02,0x40, 0x03,0x40,
          0x20,0x01, 0x20,0x00, 0x4A, 0x0D,0x01,
          0x20,0x02, 0x20,0x01, 0x6A, 0x21,0x02,
          0x20,0x01, 0x41,0x01, 0x6A, 0x21,0x01,
          0x0C,0x00,
        0x0B, 0x0B, 0x20,0x02, 0x0B,
      0x08, 0x00, 0x41,0x0A, 0x10,0x01, 0x10,0x00, 0x0B,
];

// --- Module B: print a string from linear memory (needs "putc"). ------------
// (module (import "env" "putc" (func $putc (param i32))) (memory 1)
//  (data (i32.const 0) "HELLO FROM WASM\00")
//  (func (export "run") (local $p i32)(local $c i32)
//    (block (loop
//      (local.set $c (i32.load8_u (local.get $p)))
//      (br_if 1 (i32.eqz (local.get $c)))
//      (call $putc (local.get $c))
//      (local.set $p (i32.add (local.get $p) (i32.const 1)))
//      (br 0)))))
#[rustfmt::skip]
static WASM_STR: &[u8] = &[
    0x00,0x61,0x73,0x6D, 0x01,0x00,0x00,0x00,
    0x01,0x08,0x02, 0x60,0x01,0x7F,0x00, 0x60,0x00,0x00,
    0x02,0x0C,0x01, 0x03,b'e',b'n',b'v', 0x04,b'p',b'u',b't',b'c', 0x00,0x00,
    0x03,0x02,0x01, 0x01,
    0x05,0x03,0x01, 0x00,0x01,
    0x07,0x07,0x01, 0x03,b'r',b'u',b'n', 0x00,0x01,
    0x0A,0x29,0x01,
      0x27, 0x01,0x02,0x7F,
        0x41,0x00, 0x21,0x00,
        0x02,0x40, 0x03,0x40,
          0x20,0x00, 0x2D,0x00,0x00, 0x21,0x01,
          0x20,0x01, 0x45, 0x0D,0x01,
          0x20,0x01, 0x10,0x00,
          0x20,0x00, 0x41,0x01, 0x6A, 0x21,0x00,
          0x0C,0x00,
        0x0B, 0x0B, 0x0B,
    0x0B,0x16,0x01, 0x00, 0x41,0x00,0x0B, 0x10,
      b'H',b'E',b'L',b'L',b'O',b' ',b'F',b'R',b'O',b'M',b' ',b'W',b'A',b'S',b'M',0x00,
];

// --- Module C: a graphical app — draws a window via the host ABI (needs draw).
// Generated by scripts/gen-wasm.py (win_create/win_fill/win_present calls).
#[rustfmt::skip]
static WASM_DRAW: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x13, 0x03, 0x60,
    0x02, 0x7F, 0x7F, 0x01, 0x7F, 0x60, 0x06, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F,
    0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x33, 0x03, 0x03, 0x65, 0x6E, 0x76,
    0x0A, 0x77, 0x69, 0x6E, 0x5F, 0x63, 0x72, 0x65, 0x61, 0x74, 0x65, 0x00,
    0x00, 0x03, 0x65, 0x6E, 0x76, 0x08, 0x77, 0x69, 0x6E, 0x5F, 0x66, 0x69,
    0x6C, 0x6C, 0x00, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x0B, 0x77, 0x69, 0x6E,
    0x5F, 0x70, 0x72, 0x65, 0x73, 0x65, 0x6E, 0x74, 0x00, 0x02, 0x03, 0x02,
    0x01, 0x02, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x03, 0x0A,
    0x5C, 0x01, 0x5A, 0x01, 0x01, 0x7F, 0x41, 0xC8, 0x01, 0x41, 0xF8, 0x00,
    0x10, 0x00, 0x21, 0x00, 0x20, 0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0xC8,
    0x01, 0x41, 0xF8, 0x00, 0x41, 0xC4, 0xD4, 0xC0, 0x00, 0x10, 0x01, 0x20,
    0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0xC8, 0x01, 0x41, 0x16, 0x41, 0x80,
    0xC0, 0x82, 0x07, 0x10, 0x01, 0x20, 0x00, 0x41, 0x10, 0x41, 0x24, 0x41,
    0xC8, 0x00, 0x41, 0x3C, 0x41, 0xBF, 0x92, 0x9F, 0x01, 0x10, 0x01, 0x20,
    0x00, 0x41, 0xE8, 0x00, 0x41, 0x24, 0x41, 0xD0, 0x00, 0x41, 0x3C, 0x41,
    0xD6, 0xBE, 0xFD, 0x07, 0x10, 0x01, 0x10, 0x02, 0x0B,
];

// --- Module D: file I/O — write a file, read it back, print it (needs file). -
// Generated by scripts/gen-wasm.py.
#[rustfmt::skip]
static WASM_FILE: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x10, 0x03, 0x60,
    0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x02, 0x2D, 0x03, 0x03, 0x65, 0x6E, 0x76, 0x0A, 0x66, 0x69,
    0x6C, 0x65, 0x5F, 0x77, 0x72, 0x69, 0x74, 0x65, 0x00, 0x00, 0x03, 0x65,
    0x6E, 0x76, 0x09, 0x66, 0x69, 0x6C, 0x65, 0x5F, 0x72, 0x65, 0x61, 0x64,
    0x00, 0x00, 0x03, 0x65, 0x6E, 0x76, 0x04, 0x70, 0x75, 0x74, 0x63, 0x00,
    0x01, 0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07,
    0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x03, 0x0A, 0x49, 0x01, 0x47, 0x01,
    0x03, 0x7F, 0x41, 0x00, 0x41, 0x08, 0x41, 0x10, 0x41, 0x0D, 0x10, 0x00,
    0x1A, 0x41, 0x00, 0x41, 0x08, 0x41, 0xC0, 0x00, 0x41, 0xC0, 0x00, 0x10,
    0x01, 0x21, 0x00, 0x41, 0xC0, 0x00, 0x21, 0x01, 0x41, 0xC0, 0x00, 0x20,
    0x00, 0x6A, 0x21, 0x02, 0x02, 0x40, 0x03, 0x40, 0x20, 0x01, 0x20, 0x02,
    0x4E, 0x0D, 0x01, 0x20, 0x01, 0x2D, 0x00, 0x00, 0x10, 0x02, 0x20, 0x01,
    0x41, 0x01, 0x6A, 0x21, 0x01, 0x0C, 0x00, 0x0B, 0x0B, 0x0B, 0x0B, 0x20,
    0x02, 0x00, 0x41, 0x00, 0x0B, 0x08, 0x6E, 0x6F, 0x74, 0x65, 0x2E, 0x74,
    0x78, 0x74, 0x00, 0x41, 0x10, 0x0B, 0x0D, 0x53, 0x41, 0x56, 0x45, 0x44,
    0x20, 0x42, 0x59, 0x20, 0x57, 0x41, 0x53, 0x4D,
];

// --- Module E: read the clock and print it (needs time). --------------------
#[rustfmt::skip]
static WASM_CLOCK: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0C, 0x03, 0x60,
    0x00, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x18,
    0x02, 0x03, 0x65, 0x6E, 0x76, 0x04, 0x74, 0x69, 0x6D, 0x65, 0x00, 0x00,
    0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x01,
    0x03, 0x02, 0x01, 0x02, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00,
    0x02, 0x0A, 0x08, 0x01, 0x06, 0x00, 0x10, 0x00, 0x10, 0x01, 0x0B,
];

// --- Module F: interactive — type into a window until '~' (getkey+win_text). -
#[rustfmt::skip]
static WASM_TYPE: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x1F, 0x05, 0x60,
    0x02, 0x7F, 0x7F, 0x01, 0x7F, 0x60, 0x06, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F,
    0x7F, 0x00, 0x60, 0x05, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x00, 0x60, 0x00,
    0x01, 0x7F, 0x60, 0x00, 0x00, 0x02, 0x4F, 0x05, 0x03, 0x65, 0x6E, 0x76,
    0x0A, 0x77, 0x69, 0x6E, 0x5F, 0x63, 0x72, 0x65, 0x61, 0x74, 0x65, 0x00,
    0x00, 0x03, 0x65, 0x6E, 0x76, 0x08, 0x77, 0x69, 0x6E, 0x5F, 0x66, 0x69,
    0x6C, 0x6C, 0x00, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x08, 0x77, 0x69, 0x6E,
    0x5F, 0x74, 0x65, 0x78, 0x74, 0x00, 0x02, 0x03, 0x65, 0x6E, 0x76, 0x06,
    0x67, 0x65, 0x74, 0x6B, 0x65, 0x79, 0x00, 0x03, 0x03, 0x65, 0x6E, 0x76,
    0x0B, 0x77, 0x69, 0x6E, 0x5F, 0x70, 0x72, 0x65, 0x73, 0x65, 0x6E, 0x74,
    0x00, 0x04, 0x03, 0x02, 0x01, 0x04, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75,
    0x6E, 0x00, 0x05, 0x0A, 0x6B, 0x01, 0x69, 0x01, 0x04, 0x7F, 0x41, 0xD2,
    0x01, 0x41, 0xEE, 0x00, 0x10, 0x00, 0x21, 0x00, 0x20, 0x00, 0x41, 0x00,
    0x41, 0x00, 0x41, 0xD2, 0x01, 0x41, 0xEE, 0x00, 0x41, 0xA8, 0xB4, 0xD0,
    0x00, 0x10, 0x01, 0x20, 0x00, 0x41, 0x00, 0x41, 0x00, 0x41, 0xD2, 0x01,
    0x41, 0x12, 0x41, 0xEB, 0xF4, 0xEC, 0x00, 0x10, 0x01, 0x10, 0x04, 0x41,
    0x08, 0x21, 0x01, 0x41, 0x1C, 0x21, 0x02, 0x02, 0x40, 0x03, 0x40, 0x10,
    0x03, 0x21, 0x03, 0x20, 0x03, 0x41, 0xFE, 0x00, 0x46, 0x0D, 0x01, 0x20,
    0x00, 0x20, 0x01, 0x20, 0x02, 0x20, 0x03, 0x41, 0xF0, 0xCD, 0x83, 0x07,
    0x10, 0x02, 0x20, 0x01, 0x41, 0x08, 0x6A, 0x21, 0x01, 0x10, 0x04, 0x0C,
    0x00, 0x0B, 0x0B, 0x0B,
];

// --- Module G: ask the on-device AI a question (needs ai). ------------------
#[rustfmt::skip]
static WASM_AI: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x10, 0x03, 0x60,
    0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x02, 0x15, 0x02, 0x03, 0x65, 0x6E, 0x76, 0x02, 0x61, 0x69,
    0x00, 0x00, 0x03, 0x65, 0x6E, 0x76, 0x04, 0x70, 0x75, 0x74, 0x63, 0x00,
    0x01, 0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07,
    0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02, 0x0A, 0x3E, 0x01, 0x3C, 0x01,
    0x03, 0x7F, 0x41, 0x00, 0x41, 0x0B, 0x41, 0xC0, 0x00, 0x41, 0xC0, 0x01,
    0x10, 0x00, 0x21, 0x00, 0x41, 0xC0, 0x00, 0x21, 0x01, 0x41, 0xC0, 0x00,
    0x20, 0x00, 0x6A, 0x21, 0x02, 0x02, 0x40, 0x03, 0x40, 0x20, 0x01, 0x20,
    0x02, 0x4E, 0x0D, 0x01, 0x20, 0x01, 0x2D, 0x00, 0x00, 0x10, 0x01, 0x20,
    0x01, 0x41, 0x01, 0x6A, 0x21, 0x01, 0x0C, 0x00, 0x0B, 0x0B, 0x0B, 0x0B,
    0x11, 0x01, 0x00, 0x41, 0x00, 0x0B, 0x0B, 0x57, 0x48, 0x4F, 0x20, 0x41,
    0x52, 0x45, 0x20, 0x59, 0x4F, 0x55,
];

// --- Module H: MULTI-TURN interactive AI chat — ask questions, empty line quits.
#[rustfmt::skip]
static WASM_CHAT: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x14, 0x04, 0x60,
    0x00, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x04, 0x7F, 0x7F, 0x7F,
    0x7F, 0x01, 0x7F, 0x60, 0x00, 0x00, 0x02, 0x22, 0x03, 0x03, 0x65, 0x6E,
    0x76, 0x06, 0x67, 0x65, 0x74, 0x6B, 0x65, 0x79, 0x00, 0x00, 0x03, 0x65,
    0x6E, 0x76, 0x04, 0x70, 0x75, 0x74, 0x63, 0x00, 0x01, 0x03, 0x65, 0x6E,
    0x76, 0x02, 0x61, 0x69, 0x00, 0x02, 0x03, 0x02, 0x01, 0x03, 0x05, 0x03,
    0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x03,
    0x0A, 0x84, 0x01, 0x01, 0x81, 0x01, 0x01, 0x05, 0x7F, 0x02, 0x40, 0x03,
    0x40, 0x41, 0x00, 0x21, 0x00, 0x02, 0x40, 0x03, 0x40, 0x10, 0x00, 0x21,
    0x01, 0x20, 0x01, 0x41, 0x0D, 0x46, 0x0D, 0x01, 0x20, 0x01, 0x41, 0x0A,
    0x46, 0x0D, 0x01, 0x20, 0x00, 0x20, 0x01, 0x3A, 0x00, 0x00, 0x20, 0x01,
    0x10, 0x01, 0x20, 0x00, 0x41, 0x01, 0x6A, 0x21, 0x00, 0x0C, 0x00, 0x0B,
    0x0B, 0x20, 0x00, 0x45, 0x0D, 0x01, 0x41, 0x0A, 0x10, 0x01, 0x41, 0x00,
    0x20, 0x00, 0x41, 0x80, 0x01, 0x41, 0xC0, 0x01, 0x10, 0x02, 0x21, 0x02,
    0x41, 0x80, 0x01, 0x21, 0x03, 0x41, 0x80, 0x01, 0x20, 0x02, 0x6A, 0x21,
    0x04, 0x02, 0x40, 0x03, 0x40, 0x20, 0x03, 0x20, 0x04, 0x4E, 0x0D, 0x01,
    0x20, 0x03, 0x2D, 0x00, 0x00, 0x10, 0x01, 0x20, 0x03, 0x41, 0x01, 0x6A,
    0x21, 0x03, 0x0C, 0x00, 0x0B, 0x0B, 0x41, 0x0A, 0x10, 0x01, 0x0C, 0x00,
    0x0B, 0x0B, 0x0B,
];

// --- Module I: speak a notification via Asili Voice (needs voice cap). -------
#[rustfmt::skip]
static WASM_NOTIFY: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x09, 0x02, 0x60,
    0x02, 0x7F, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0D, 0x01, 0x03, 0x65,
    0x6E, 0x76, 0x05, 0x73, 0x70, 0x65, 0x61, 0x6B, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x01, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72,
    0x75, 0x6E, 0x00, 0x01, 0x0A, 0x0A, 0x01, 0x08, 0x00, 0x41, 0x00, 0x41,
    0x2C, 0x10, 0x00, 0x0B, 0x0B, 0x32, 0x01, 0x00, 0x41, 0x00, 0x0B, 0x2C,
    0x59, 0x4F, 0x55, 0x52, 0x20, 0x50, 0x43, 0x20, 0x49, 0x53, 0x20, 0x4C,
    0x4F, 0x57, 0x20, 0x4F, 0x4E, 0x20, 0x42, 0x41, 0x54, 0x54, 0x45, 0x52,
    0x59, 0x2C, 0x20, 0x50, 0x4C, 0x45, 0x41, 0x53, 0x45, 0x20, 0x43, 0x48,
    0x41, 0x52, 0x47, 0x45, 0x20, 0x49, 0x54, 0x2E,
];

// --- Module I2: the SAME app with a DIFFERENT spoken text (3.262). ----------
// Additive, so notify.wasm stays byte-identical and ONE build serves both boots.
// Generated by `python scripts/gen-wasm.py` from NOTIFY2_MSG -- do not hand-edit.
#[rustfmt::skip]
static WASM_NOTIFY2: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x09, 0x02, 0x60,
    0x02, 0x7F, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0D, 0x01, 0x03, 0x65,
    0x6E, 0x76, 0x05, 0x73, 0x70, 0x65, 0x61, 0x6B, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x01, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72,
    0x75, 0x6E, 0x00, 0x01, 0x0A, 0x0A, 0x01, 0x08, 0x00, 0x41, 0x00, 0x41,
    0x29, 0x10, 0x00, 0x0B, 0x0B, 0x2F, 0x01, 0x00, 0x41, 0x00, 0x0B, 0x29,
    0x54, 0x48, 0x45, 0x20, 0x4B, 0x45, 0x54, 0x54, 0x4C, 0x45, 0x20, 0x48,
    0x41, 0x53, 0x20, 0x42, 0x4F, 0x49, 0x4C, 0x45, 0x44, 0x2C, 0x20, 0x59,
    0x4F, 0x55, 0x52, 0x20, 0x54, 0x45, 0x41, 0x20, 0x49, 0x53, 0x20, 0x52,
    0x45, 0x41, 0x44, 0x59, 0x2E,
];

// --- Linear memory ----------------------------------------------------------
static mut WASM_MEM: [u8; MEM_SIZE] = [0; MEM_SIZE];

// --- putc: text shown as it is written (claude/nia-putc, OS_REQUESTS item 6) --------------------------
// Each byte goes through a small UTF-8 assembler and is printed at once, on lines that start with
// "    [wasm app] ". A complete valid character prints as it is, unless it is unsafe for a terminal: C0 and C1
// controls other than \n and \t, DEL, and the bidi controls U+202A..U+202E and U+2066..U+2069 print `?`. An
// invalid or cut sequence prints U+FFFD. Nothing is buffered or sliced, so no byte an app writes can panic the
// kernel (the old end-of-run print sliced a "?" fallback to the buffer length). `putc_break` closes an open line
// before any other kernel print during a run, and `finish_putc` runs where every run ends.
static mut PUTC_OPEN: bool = false; // a "[wasm app] " line has been started and not ended
static mut PUTC_PEND: [u8; 4] = [0; 4]; // the bytes of a UTF-8 character still being assembled
static mut PUTC_PLEN: usize = 0;
static mut PUTC_NEED: usize = 0;

fn putc_safe(c: char) -> bool {
    match c {
        '\n' | '\t' => true,
        ' '..='~' => true,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => false, // bidi controls ("trojan source")
        _ => c >= '\u{A0}', // C0, DEL and C1 (U+0080..U+009F) are below
    }
}

fn putc_emit(c: char) {
    let c = if putc_safe(c) { c } else { '?' };
    // SAFETY: single-threaded interpreter; plain flag.
    unsafe {
        if !PUTC_OPEN {
            crate::kprint!("    [wasm app] ");
            PUTC_OPEN = true;
        }
        if c == '\n' {
            crate::kprint!("\n");
            PUTC_OPEN = false;
        } else {
            crate::kprint!("{}", c);
        }
    }
}

/// Host `putc(byte)`: assemble UTF-8 and print each character as soon as it is complete.
fn putc_byte(b: u8) {
    // SAFETY: single-threaded interpreter; PUTC_PLEN stays below 4.
    unsafe {
        if PUTC_PLEN > 0 {
            if b & 0xC0 == 0x80 {
                (*addr_of_mut!(PUTC_PEND))[PUTC_PLEN] = b;
                PUTC_PLEN += 1;
                if PUTC_PLEN == PUTC_NEED {
                    let n = PUTC_PLEN;
                    PUTC_PLEN = 0;
                    // from_utf8 also refuses overlong forms and surrogates: those print U+FFFD.
                    match core::str::from_utf8(&(&*core::ptr::addr_of!(PUTC_PEND))[..n]).ok().and_then(|t| t.chars().next()) {
                        Some(c) => putc_emit(c),
                        None => putc_emit('\u{FFFD}'),
                    }
                }
                return;
            }
            PUTC_PLEN = 0; // a character cut by a new lead byte or ASCII: U+FFFD, then handle this byte
            putc_emit('\u{FFFD}');
        }
        match b {
            0x00..=0x7F => putc_emit(b as char),
            0xC2..=0xDF | 0xE0..=0xEF | 0xF0..=0xF4 => {
                PUTC_NEED = if b < 0xE0 { 2 } else if b < 0xF0 { 3 } else { 4 };
                (*addr_of_mut!(PUTC_PEND))[0] = b;
                PUTC_PLEN = 1;
            }
            _ => putc_emit('\u{FFFD}'), // a continuation byte alone, or a byte that never starts UTF-8
        }
    }
}

/// Close an open putc line (and flush a cut character as U+FFFD) before any other kernel print during a run.
fn putc_break() {
    // SAFETY: single-threaded interpreter; plain flags.
    unsafe {
        if PUTC_PLEN > 0 {
            PUTC_PLEN = 0;
            putc_emit('\u{FFFD}');
        }
        if PUTC_OPEN {
            crate::kprint!("\n");
            PUTC_OPEN = false;
        }
    }
}

/// Where every run ends (clean return, TRAP, Ctrl+C): the same as a break.
fn finish_putc() {
    putc_break();
}

fn putc_reset() {
    // SAFETY: single-threaded interpreter; plain stores.
    unsafe {
        PUTC_OPEN = false;
        PUTC_PLEN = 0;
    }
}

// N1 stop key (claude/nia-n1). An app that loops forever is stopped by Ctrl+C
// (byte 0x03). Every POLL_EVERY branches back to a loop label, `branch` reads
// one byte from the console (`uart::getc`): 0x03 stops the app with a TRAP, any
// other byte goes into this queue (dropped when it is full), and `getkey`
// drains the queue before it reads the console, so no key is lost. Ctrl+C read
// by `getkey` stops the app too, so apps never see byte 3. The queue is cleared
// when an app starts, so keys left over when an app ends are dropped and never
// reach the next app. Only the serial console feeds `uart::getc` today.
const STOP_KEY: u8 = 0x03;
const POLL_EVERY: u32 = 4096;
const KEYQ_CAP: usize = 16;
const STOPPED: &str = "stopped by Ctrl+C";
static mut KEYQ: [u8; KEYQ_CAP] = [0; KEYQ_CAP];
static mut KEYQ_HEAD: usize = 0;
static mut KEYQ_LEN: usize = 0;
static mut LOOP_BRANCHES: u32 = 0;
static mut KEYS_QUEUED: u32 = 0; // keys the loop poll queued since the app started (wasmfuzz reads it)

fn keyq_reset() {
    // SAFETY: single-threaded interpreter; plain stores to statics.
    unsafe {
        KEYQ_HEAD = 0;
        KEYQ_LEN = 0;
        LOOP_BRANCHES = 0;
        KEYS_QUEUED = 0;
    }
}

fn keyq_pop() -> Option<u8> {
    // SAFETY: single-threaded interpreter; indexes stay below KEYQ_CAP.
    unsafe {
        if KEYQ_LEN == 0 {
            return None;
        }
        let b = (*addr_of_mut!(KEYQ))[KEYQ_HEAD];
        KEYQ_HEAD = (KEYQ_HEAD + 1) % KEYQ_CAP;
        KEYQ_LEN -= 1;
        Some(b)
    }
}

/// Called on every branch back to a loop label: the forced-close check every time (one relaxed load), then one
/// desk tick and one console read per POLL_EVERY.
fn loop_poll() -> Result<(), &'static str> {
    cell_forced_check()?;
    // SAFETY: single-threaded interpreter; a plain counter.
    unsafe {
        LOOP_BRANCHES = LOOP_BRANCHES.wrapping_add(1);
        if LOOP_BRANCHES % POLL_EVERY != 0 {
            return Ok(());
        }
    }
    crate::desk::app_tick(); // only while the app owns a cell window, at most every 10 ms
    console_key()
}

/// "Close anyway" was chosen on the app's cell window: the app stops here (afriui.md section 25).
const FORCED: &str = "closed by the person (Close anyway)";

fn cell_forced_check() -> Result<(), &'static str> {
    if crate::desk::cell_forced() {
        return Err(FORCED);
    }
    Ok(())
}

/// One row of cells: `cell_put` caps its length here before doubling it to bytes.
const CELL_PUT_MAX: usize = 120;

/// Set by `run_app_in` for the whole run (afriui.md section 25): a second start while one app runs (the shell's
/// run, appget, a desk tile, a plan step) is refused before it touches anything.
static APP_RUNNING: AtomicBool = AtomicBool::new(false);
/// The running app's name and whether it is signed (a bundled or manifest app), for the cell window's title.
static mut APP_NAME: [u8; 24] = [0; 24];
static mut APP_NAME_LEN: usize = 0;
static mut APP_SIGNED: bool = false;

/// An app is running now (stage 4's Andika tile greys itself on this).
#[allow(dead_code)] // the desk's tile uses it in stage 4
pub fn app_running() -> bool {
    APP_RUNNING.load(Ordering::Relaxed)
}

fn refuse_nested(label: &str) -> bool {
    if APP_RUNNING.load(Ordering::Relaxed) {
        crate::kprintln!("[nia] REFUSED '{}': an app is already running", label);
        return true;
    }
    false
}

fn set_app_name(label: &str) {
    let b = label.as_bytes();
    let n = b.len().min(24);
    // SAFETY: single-threaded; no app is running yet (APP_RUNNING was just set by this run).
    unsafe {
        (&mut *addr_of_mut!(APP_NAME))[..n].copy_from_slice(&b[..n]);
        APP_NAME_LEN = n;
    }
}

fn set_app_signed(signed: bool) {
    // SAFETY: single-threaded; read by cell_open during the run that follows.
    unsafe { APP_SIGNED = signed };
}

/// The stack line for the desk drawn 16 call levels deep (`cell_event` at depth 16, afriui.md section 25): the
/// unused stack below this frame is painted, the desk runs, and the lowest word it changed shows how much was
/// left. Returns the painted range, or None when the stack bounds are not known.
const CELL_PAINT: usize = 0x5AFE_C311_5AFE_C311u64 as usize;

/// The stack probe is a test tool: it paints about 850 KB of stack, so it runs only after `nia --stackprobe`
/// (desk-cell-app sends it), until the next boot. A real app never pays for it (architect's follow-up 2).
static STACK_PROBE: AtomicBool = AtomicBool::new(false);

/// `nia --stackprobe`: turn the cell stack probe on.
pub fn stack_probe_on() {
    STACK_PROBE.store(true, Ordering::Relaxed);
    crate::kprintln!("[nia] stack probe on (test only)");
}

#[inline(never)]
fn cell_stack_paint() -> Option<(usize, usize)> {
    let (bottom, _top) = shell_stack();
    let here = core::ptr::addr_of!(bottom) as usize;
    let lo = (bottom + 64) & !7;
    let hi = here.saturating_sub(2048) & !7; // keep clear of this frame and the calls just below it
    if bottom == 0 || hi <= lo {
        return None;
    }
    let mut a = lo;
    while a < hi {
        // SAFETY: [lo, hi) is the unused part of the stack the interpreter runs on, below every live frame.
        unsafe { core::ptr::write_volatile(a as *mut usize, CELL_PAINT) };
        a += 8;
    }
    Some((lo, hi))
}

fn cell_stack_report((lo, hi): (usize, usize)) {
    let (bottom, _top) = shell_stack();
    let mut a = lo;
    // SAFETY: reads of the range painted above.
    while a < hi && unsafe { core::ptr::read_volatile(a as *const usize) } == CELL_PAINT {
        a += 8;
    }
    crate::kprintln!("[nia] cell stack: {} bytes left at depth {}", a.saturating_sub(bottom), MAX_CALL_DEPTH);
}

/// The one non-blocking console read the interpreter does outside `getkey` (the loop poll, `pollkey` and
/// `sleep_ms` all come here): Ctrl+C stops the app (checked before the queue limit, so a full queue never hides
/// it), any other key joins the queue (dropped when it is full).
fn console_key() -> Result<(), &'static str> {
    if crate::desk::cell_owned() {
        // Every byte is the desk's while the app owns a cell window: a key event, Ctrl+C (3) included, and Ctrl+\
        // the close request or, 3 s after an unanswered one, "Close anyway" (afriui.md section 25).
        while let Some(c) = crate::uart::getc() {
            crate::desk::cell_serial_byte(c);
        }
        return cell_forced_check();
    }
    // SAFETY: single-threaded interpreter; indexes stay below KEYQ_CAP.
    unsafe {
        if let Some(c) = crate::uart::getc() {
            if c == STOP_KEY {
                return Err(STOPPED);
            }
            if KEYQ_LEN < KEYQ_CAP {
                (*addr_of_mut!(KEYQ))[(KEYQ_HEAD + KEYQ_LEN) % KEYQ_CAP] = c;
                KEYQ_LEN += 1;
                KEYS_QUEUED += 1;
            }
        }
    }
    Ok(())
}

/// Host `pollkey()`: the next key, or -1 when there is none; never waits. The console is read first through
/// `console_key` (so a Ctrl+C TRAPs and is never returned or queued), then the oldest queued key is returned.
fn host_pollkey() -> Result<i32, &'static str> {
    console_key()?;
    if crate::desk::cell_owned() {
        return Ok(-1); // its keys are cell events (afriui.md section 25)
    }
    Ok(match keyq_pop() {
        Some(c) => c as i32,
        None => -1,
    })
}

/// The longest one `sleep_ms` call waits; an app that wants longer calls it again.
const SLEEP_MAX_MS: u64 = 10_000;

/// Host `sleep_ms(ms)`: wait `ms` milliseconds (unsigned, at most SLEEP_MAX_MS) on the always-running counter
/// (CNTPCT on ARM, the TSC on the PC), reading the console about every millisecond through `console_key`, so
/// keys are kept for pollkey/getkey and Ctrl+C stops the app at once. A spin, not wfi: it works whether
/// interrupts are masked or not (a wfi version on the 10 ms tick is a later item). Returns the milliseconds
/// slept, measured on the same counter.
fn host_sleep_ms(ms: i32) -> Result<i32, &'static str> {
    let ms = (ms as u32 as u64).min(SLEEP_MAX_MS);
    let hz = crate::timer::hz().max(1);
    let target = ms * hz / 1000; // the deadline, once, in counter units (at most 1e4 * ~5e9, fits in u64)
    let step = (hz / 1000).max(1); // about 1 ms between console reads
    let t0 = crate::timer::cycles();
    let mut last = t0;
    let mut elapsed = 0u64;
    while elapsed < target {
        let now = crate::timer::cycles();
        elapsed = now.wrapping_sub(t0);
        if now.wrapping_sub(last) >= step {
            last = now;
            cell_forced_check()?;
            crate::desk::app_tick(); // the desk keeps drawing while the app sleeps (only with a cell window)
            console_key()?;
        }
        core::hint::spin_loop();
    }
    Ok((elapsed * 1000 / hz).min(i32::MAX as u64) as i32)
}

// --- Module globals (2.92) --------------------------------------------------
// Each WASM global's value is held as a raw 64-bit cell (works for i32/i64/f32/
// f64 alike, like the value stack). A single shared array: one module runs at a
// time, mirroring WASM_MEM. Initialized from the module's global section.
static mut WASM_GLOBALS: [i64; MAX_GLOBALS] = [0; MAX_GLOBALS];

fn global_get(idx: usize) -> i64 {
    // SAFETY: caller bounds-checks idx < MAX_GLOBALS.
    unsafe { addr_of_mut!(WASM_GLOBALS).cast::<i64>().add(idx).read() }
}
fn global_set(idx: usize, v: i64) {
    // SAFETY: caller bounds-checks idx < MAX_GLOBALS.
    unsafe { addr_of_mut!(WASM_GLOBALS).cast::<i64>().add(idx).write(v) };
}

fn mem_read8(addr: usize) -> u8 {
    // SAFETY: bounds-checked read of the module's linear memory.
    unsafe {
        if addr < MEM_SIZE {
            *(addr_of_mut!(WASM_MEM) as *const u8).add(addr)
        } else {
            0
        }
    }
}
fn mem_write8(addr: usize, v: u8) {
    unsafe {
        if addr < MEM_SIZE {
            *(addr_of_mut!(WASM_MEM) as *mut u8).add(addr) = v;
        }
    }
}
fn mem_read16(addr: usize) -> u16 {
    u16::from_le_bytes([mem_read8(addr), mem_read8(addr + 1)])
}
fn mem_read32(addr: usize) -> i32 {
    i32::from_le_bytes([mem_read8(addr), mem_read8(addr + 1), mem_read8(addr + 2), mem_read8(addr + 3)])
}
fn mem_write32(addr: usize, v: i32) {
    let b = v.to_le_bytes();
    for i in 0..4 {
        mem_write8(addr + i, b[i]);
    }
}
fn mem_read64(addr: usize) -> i64 {
    let mut b = [0u8; 8];
    for i in 0..8 {
        b[i] = mem_read8(addr + i);
    }
    i64::from_le_bytes(b)
}
fn mem_write64(addr: usize, v: i64) {
    let b = v.to_le_bytes();
    for i in 0..8 {
        mem_write8(addr + i, b[i]);
    }
}

/// The trap for an access outside the app's 64 KiB of linear memory.
const OOB: &str = "out of bounds memory access";

/// The effective address of a `width`-byte access, as WebAssembly says: the i32 base is UNSIGNED (`as u32`, so
/// -1 is 4294967295, never usize::MAX), the offset is added with a check, and the whole access must end at or
/// before the end of memory; otherwise the app traps. After this, `addr + i` inside mem_read*/mem_write* can
/// never overflow (claude/nia-addr).
fn ea(base: i32, off: usize, width: usize) -> Result<usize, &'static str> {
    let a = (base as u32 as usize).checked_add(off).ok_or(OOB)?;
    match a.checked_add(width) {
        Some(end) if end <= MEM_SIZE => Ok(a),
        _ => Err(OOB),
    }
}

/// A host function's pointer and the number of bytes it will touch: the same rule as `ea`.
fn mem_range(ptr: i32, len: usize) -> Result<usize, &'static str> {
    ea(ptr, 0, len)
}

/// An i32 argument as an unsigned number (WebAssembly's i32 has no sign of its own), so -1 is 4294967295 and
/// never usize::MAX: values stay below 2^32, so adding two of them (tonga's `rx + rw`, `x + col`) cannot
/// overflow a 64-bit usize. Used for every draw argument (claude/nia-addr, architect's addition A).
fn u(v: i32) -> usize {
    v as u32 as usize
}

/// A length argument (i32, unsigned as WebAssembly says) capped at what the host function uses.
fn ulen(v: i32, cap: usize) -> usize {
    (v as u32 as usize).min(cap)
}

/// Read a NUL-free name string out of the app's linear memory into a small buffer.
fn read_name<'a>(ptr: i32, len: i32, buf: &'a mut [u8; 32]) -> Result<&'a str, &'static str> {
    let n = ulen(len, 32);
    let p = mem_range(ptr, n)?;
    for i in 0..n {
        buf[i] = mem_read8(p + i);
    }
    Ok(core::str::from_utf8(&buf[..n]).unwrap_or(""))
}

/// Host `file_write(name_ptr, name_len, data_ptr, data_len)` → bytes written / -1.
fn host_file_write(nptr: i32, nlen: i32, dptr: i32, dlen: i32) -> Result<i32, &'static str> {
    let mut nb = [0u8; 32];
    let name = read_name(nptr, nlen, &mut nb)?;
    let mut db = [0u8; 512];
    let dn = ulen(dlen, 512);
    let d = mem_range(dptr, dn)?;
    for i in 0..dn {
        db[i] = mem_read8(d + i);
    }
    if crate::vfs::write(name, &db[..dn]) {
        crate::kprintln!("    [wasm app] file_write('{}', {} bytes) -> ok", name, dn);
        Ok(dn as i32)
    } else {
        crate::kprintln!("    [wasm app] file_write('{}') -> failed", name);
        Ok(-1)
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > hay.len() {
        return false;
    }
    for i in 0..=(hay.len() - needle.len()) {
        if &hay[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}

fn to_lower(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        c + 32
    } else {
        c
    }
}

fn fmt_i64(mut v: i64, buf: &mut [u8]) -> usize {
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let mut tmp = [0u8; 24];
    let mut n = 0;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    } else {
        while v > 0 {
            tmp[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
    }
    let mut k = 0;
    if neg {
        buf[0] = b'-';
        k = 1;
    }
    for x in 0..n {
        buf[k + x] = tmp[n - 1 - x];
    }
    k + n
}

fn int_before(p: &[u8], i: usize) -> Option<i64> {
    let mut j = i as isize - 1;
    while j >= 0 && p[j as usize] == b' ' {
        j -= 1;
    }
    if j < 0 || !p[j as usize].is_ascii_digit() {
        return None;
    }
    let mut val = 0i64;
    let mut mul = 1i64;
    while j >= 0 && p[j as usize].is_ascii_digit() {
        val += ((p[j as usize] - b'0') as i64) * mul;
        mul *= 10;
        j -= 1;
    }
    Some(val)
}

fn int_after(p: &[u8], i: usize) -> Option<i64> {
    let mut j = i + 1;
    while j < p.len() && p[j] == b' ' {
        j += 1;
    }
    if j >= p.len() || !p[j].is_ascii_digit() {
        return None;
    }
    let mut val = 0i64;
    while j < p.len() && p[j].is_ascii_digit() {
        val = val * 10 + (p[j] - b'0') as i64;
        j += 1;
    }
    Some(val)
}

/// Real on-device arithmetic: find `A <op> B` in the prompt and compute it.
/// Integer floor square root (Newton's method) -- shared by the sqrt responder.
fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Squares and (integer) square roots -- "sqrt 144", "square root of 200", "square of 12",
/// "15 squared". All on-device. Checked before try_math so the phrasing (no +-*/ operator)
/// is recognized. Roots are floored; a negative sqrt argument falls through to the KB.
/// Trial-division primality test (shared by the "is N prime" responder).
fn is_prime_u64(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut d = 3u64;
    while d <= n / d {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

/// Primality queries -- "is 17 prime", "prime 97". On-device trial division; reports whether
/// the first integer in the prompt is prime. A negative or absent number falls through to the KB.
/// Prime factorization -- "factorize 60", "factors of 100". On-device trial division; writes
/// "N = p1 x p2 x ..." for the first integer in the prompt. Range-capped (2..=1e9) so the
/// factor list can never overrun the response buffer; out of range falls through to the KB.
fn try_factorize(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"factor") {
        return None;
    }
    let n = first_int(p)?;
    if !(2..=1_000_000_000).contains(&n) {
        return None;
    }
    let mut m = n as u64;
    let mut k = 0usize;
    k += fmt_i64(n, &mut out[k..]);
    for &c in b" = " {
        out[k] = c;
        k += 1;
    }
    let mut first = true;
    let mut d = 2u64;
    while d <= m / d {
        while m % d == 0 {
            if !first {
                for &c in b" x " {
                    out[k] = c;
                    k += 1;
                }
            }
            first = false;
            k += fmt_i64(d as i64, &mut out[k..]);
            m /= d;
        }
        d += 1;
    }
    if m > 1 {
        if !first {
            for &c in b" x " {
                out[k] = c;
                k += 1;
            }
        }
        k += fmt_i64(m as i64, &mut out[k..]);
    }
    for &c in b" (ON-DEVICE)" {
        out[k] = c;
        k += 1;
    }
    Some(k)
}

fn try_prime(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if contains(p, b"prime") {
        let n = first_int(p)?;
        if n < 0 {
            return None;
        }
        let suf: &[u8] = if is_prime_u64(n as u64) {
            b" IS PRIME (ON-DEVICE)"
        } else {
            b" IS NOT PRIME (ON-DEVICE)"
        };
        return Some(emit_num(out, b"", n, suf));
    }
    None
}

fn try_squares(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if contains(p, b"sqrt") || contains(p, b"square root") {
        let n = first_int(p)?;
        if n < 0 {
            return None;
        }
        let r = isqrt_u64(n as u64) as i64;
        return Some(emit_num(out, b"= ", r, b" (INTEGER SQRT, ON-DEVICE)"));
    }
    if contains(p, b"squared") || contains(p, b"square") {
        let n = first_int(p)?;
        return Some(emit_num(out, b"= ", n * n, b" (SQUARED, ON-DEVICE)"));
    }
    // Cube root BEFORE cube, since "cube root" also contains "cube".
    if contains(p, b"cube root") || contains(p, b"cbrt") {
        let n = first_int(p)?;
        if n < 0 {
            return None;
        }
        let n = n as u128;
        // Integer cube root by binary search: largest r with r^3 <= n. The bound 2^21 keeps
        // r^3 within i64 range (r <= 2097152 => r^3 <= ~2^63).
        let mut lo: u128 = 0;
        let mut hi: u128 = 2_097_152;
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            if mid * mid * mid <= n {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        return Some(emit_num(out, b"= ", lo as i64, b" (INTEGER CUBE ROOT, ON-DEVICE)"));
    }
    if contains(p, b"cubed") || contains(p, b"cube") {
        let n = first_int(p)?;
        // Compute in i128 and only answer if the cube fits i64 (else fall through honestly).
        let c = (n as i128).checked_mul(n as i128)?.checked_mul(n as i128)?;
        if c >= i64::MIN as i128 && c <= i64::MAX as i128 {
            return Some(emit_num(out, b"= ", c as i64, b" (CUBED, ON-DEVICE)"));
        }
        return None;
    }
    None
}

/// The nth Fibonacci number, F(0)=0, F(1)=1, computed iteratively on-device.
/// Answers "the 10th fibonacci number" / "fibonacci 20". Capped at n<=92 because
/// F(93) overflows i64 — a larger n falls through honestly rather than wrapping.
fn try_fib(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"fib") {
        return None;
    }
    let n = first_int(p)?;
    if !(0..=92).contains(&n) {
        return None;
    }
    let mut a: i64 = 0; // F(0)
    let mut b: i64 = 1; // F(1)
    let mut i = 0;
    while i < n {
        let t = a + b;
        a = b;
        b = t;
        i += 1;
    }
    Some(emit_num(out, b"= ", a, b" (NTH FIBONACCI, ON-DEVICE)"))
}

/// The factorial n! computed iteratively on-device. Answers "what is 5 factorial" /
/// "factorial of 6". Capped at n<=20 because 21! overflows i64 — a larger n falls
/// through honestly. 0!=1, 1!=1. Must be dispatched BEFORE try_factorize, whose "factor"
/// guard would otherwise swallow "factorial".
fn try_factorial(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"factorial") {
        return None;
    }
    let n = first_int(p)?;
    if !(0..=20).contains(&n) {
        return None;
    }
    let mut f: i64 = 1;
    let mut i = 2i64;
    while i <= n {
        f *= i;
        i += 1;
    }
    Some(emit_num(out, b"= ", f, b" (FACTORIAL, ON-DEVICE)"))
}

/// Integer exponentiation base^exp computed on-device. Answers "what is 2 to the power of 10"
/// / "2 ^ 10" -> 1024, using the first two non-negative integers as base and exponent. Guarded
/// on the word "power" or a literal '^'. The exponent is capped at 63 and the result is computed
/// with checked_pow, so an overflowing (or oversized) request falls through honestly by returning
/// None rather than wrapping. 0^0 = 1 (Rust's pow convention). Dispatched before try_math, whose
/// operator set does not include '^', so "2^10" is not mistaken for a different arithmetic op.
fn try_power(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"power") && !contains(p, b"^") {
        return None;
    }
    let (base, exp) = two_ints(p)?;
    if !(0..=63).contains(&exp) {
        return None;
    }
    let r = base.checked_pow(exp as u32)?;
    Some(emit_num(out, b"= ", r, b" (POWER, ON-DEVICE)"))
}

/// The digit sum of a non-negative integer, computed on-device. Answers "digit sum of 12345" /
/// "sum of the digits of 9999" -> the sum of decimal digits (12345 -> 15, 9999 -> 36, 0 -> 0),
/// using the first integer in the prompt. Guarded on "digit sum" / "digitsum" / "sum of digits"
/// / "sum of the digits"; a negative input falls through (None). Dispatched before try_math --
/// there is no operator in "sum of digits", so it would otherwise reach the knowledge base.
fn try_digitsum(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"digit sum")
        && !contains(p, b"digitsum")
        && !contains(p, b"sum of digits")
        && !contains(p, b"sum of the digits")
    {
        return None;
    }
    let n = first_int(p)?;
    if n < 0 {
        return None;
    }
    let mut v = n as u64;
    let mut s: i64 = 0;
    while v > 0 {
        s += (v % 10) as i64;
        v /= 10;
    }
    Some(emit_num(out, b"= ", s, b" (DIGIT SUM, ON-DEVICE)"))
}

/// Even/odd (parity) classification of an integer, checked on-device. Answers "is 42 even or odd"
/// / "is 13 odd" -> "42 is EVEN (CHECKED ON-DEVICE)" / "13 is ODD ...", using the first integer in
/// the prompt (negatives handled: -4 is EVEN, -3 is ODD). Guarded on "even" or "odd"; if no integer
/// is present it falls through (so "is seven even" -- no digits -- reaches the knowledge base).
/// Dispatched before try_math since the phrasing carries no arithmetic operator.
fn try_evenodd(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"even") && !contains(p, b"odd") {
        return None;
    }
    let n = first_int(p)?;
    let label: &[u8] = if n % 2 == 0 {
        b" is EVEN (CHECKED ON-DEVICE)"
    } else {
        b" is ODD (CHECKED ON-DEVICE)"
    };
    let mut k = fmt_i64(n, out);
    out[k..k + label.len()].copy_from_slice(label);
    k += label.len();
    Some(k)
}

/// Arithmetic MEAN of a LIST of non-negative integers in the prompt, checked on-device. Answers
/// "average of 10 20 30" -> "MEAN = 20 (of 3 values, sum 60, ON-DEVICE)"; when the sum is not evenly
/// divisible it also reports the remainder ("mean of 1 2 4" -> "MEAN = 2 r 1 (of 3 values, sum 7,
/// ...)"). Unlike try_math (a single binary op) it scans the WHOLE list. Guarded on "average" or
/// "mean" and needs at least two integers, so "what does X mean" (no list) falls through to the KB.
/// Dispatched before try_math since the phrasing carries no arithmetic operator.
fn try_average(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"average") && !contains(p, b"mean") {
        return None;
    }
    // Collect and sum every non-negative integer in the prompt.
    let mut sum: i64 = 0;
    let mut count: i64 = 0;
    let mut i = 0;
    while i < p.len() {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            sum += v;
            count += 1;
        } else {
            i += 1;
        }
    }
    if count < 2 {
        return None; // need at least two values to average
    }
    let q = sum / count;
    let r = sum % count;
    let mut k = 0usize;
    let pre = b"MEAN = ";
    out[..pre.len()].copy_from_slice(pre);
    k += pre.len();
    k += fmt_i64(q, &mut out[k..]);
    if r != 0 {
        let rp = b" r ";
        out[k..k + rp.len()].copy_from_slice(rp);
        k += rp.len();
        k += fmt_i64(r, &mut out[k..]);
    }
    let mid = b" (of ";
    out[k..k + mid.len()].copy_from_slice(mid);
    k += mid.len();
    k += fmt_i64(count, &mut out[k..]);
    let mid2 = b" values, sum ";
    out[k..k + mid2.len()].copy_from_slice(mid2);
    k += mid2.len();
    k += fmt_i64(sum, &mut out[k..]);
    let suf = b", ON-DEVICE)";
    out[k..k + suf.len()].copy_from_slice(suf);
    k += suf.len();
    Some(k)
}

/// MAXIMUM or MINIMUM of a LIST of non-negative integers in the prompt, checked on-device. Answers
/// "max of 3 8 5" -> "MAX = 8 (of 3 values, ON-DEVICE)" and "smallest of 10 42 7" -> "MIN = 7 (of 3
/// values, ON-DEVICE)". Like try_average it scans the WHOLE list. The mode is chosen by keyword:
/// max/maximum/largest/biggest/greatest -> MAX; min/minimum/smallest/least/lowest -> MIN (MAX wins if
/// both appear). Needs at least two integers, so a bare keyword with no list falls through to the KB.
/// Dispatched before try_math (no arithmetic operator in the phrasing).
fn try_maxmin(p: &[u8], out: &mut [u8]) -> Option<usize> {
    let want_max = contains(p, b"max")
        || contains(p, b"largest")
        || contains(p, b"biggest")
        || contains(p, b"greatest");
    let want_min = contains(p, b"min")
        || contains(p, b"smallest")
        || contains(p, b"least")
        || contains(p, b"lowest");
    if !want_max && !want_min {
        return None;
    }
    // Collect every non-negative integer, tracking the running max and min.
    let mut count: i64 = 0;
    let mut hi: i64 = 0;
    let mut lo: i64 = 0;
    let mut i = 0;
    while i < p.len() {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            if count == 0 || v > hi {
                hi = v;
            }
            if count == 0 || v < lo {
                lo = v;
            }
            count += 1;
        } else {
            i += 1;
        }
    }
    if count < 2 {
        return None; // need at least two values
    }
    // MAX takes precedence if both keyword families appear.
    let (label, val): (&[u8], i64) = if want_max {
        (b"MAX = ", hi)
    } else {
        (b"MIN = ", lo)
    };
    let mut k = 0usize;
    out[..label.len()].copy_from_slice(label);
    k += label.len();
    k += fmt_i64(val, &mut out[k..]);
    let mid = b" (of ";
    out[k..k + mid.len()].copy_from_slice(mid);
    k += mid.len();
    k += fmt_i64(count, &mut out[k..]);
    let suf = b" values, ON-DEVICE)";
    out[k..k + suf.len()].copy_from_slice(suf);
    k += suf.len();
    Some(k)
}

/// RANGE (spread = max - min) of a LIST of non-negative integers in the prompt, checked on-device.
/// Answers "range of 3 8 5" -> "RANGE = 5 (max 8 - min 3, of 3 values, ON-DEVICE)" and "spread of 10
/// 42 7" -> "RANGE = 35 (max 42 - min 7, ...)". The list-statistics sibling of try_average/try_maxmin;
/// like them it scans the WHOLE prompt. Guarded on "range" or "spread" and needs at least two
/// integers, so a bare keyword with no list falls through to the KB. Dispatched before try_math.
fn try_range(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"range") && !contains(p, b"spread") {
        return None;
    }
    let mut count: i64 = 0;
    let mut hi: i64 = 0;
    let mut lo: i64 = 0;
    let mut i = 0;
    while i < p.len() {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            if count == 0 || v > hi {
                hi = v;
            }
            if count == 0 || v < lo {
                lo = v;
            }
            count += 1;
        } else {
            i += 1;
        }
    }
    if count < 2 {
        return None;
    }
    let mut k = 0usize;
    let pre = b"RANGE = ";
    out[..pre.len()].copy_from_slice(pre);
    k += pre.len();
    k += fmt_i64(hi - lo, &mut out[k..]);
    let m1 = b" (max ";
    out[k..k + m1.len()].copy_from_slice(m1);
    k += m1.len();
    k += fmt_i64(hi, &mut out[k..]);
    let m2 = b" - min ";
    out[k..k + m2.len()].copy_from_slice(m2);
    k += m2.len();
    k += fmt_i64(lo, &mut out[k..]);
    let m3 = b", of ";
    out[k..k + m3.len()].copy_from_slice(m3);
    k += m3.len();
    k += fmt_i64(count, &mut out[k..]);
    let suf = b" values, ON-DEVICE)";
    out[k..k + suf.len()].copy_from_slice(suf);
    k += suf.len();
    Some(k)
}

/// SUM (total) of a LIST of non-negative integers in the prompt, checked on-device. Answers "sum of 3
/// 8 5" -> "SUM = 16 (of 3 values, ON-DEVICE)" and "total of 10 20 30" -> "SUM = 60 (...)". The
/// list-statistics sibling of try_average/try_maxmin/try_range; like them it scans the WHOLE prompt.
/// Guarded on "sum", "total", "add up", or "altogether" and needs at least two integers. Dispatched
/// AFTER try_digitsum (so "sum of digits" stays digit-sum) and before try_math.
fn try_sum(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"sum")
        && !contains(p, b"total")
        && !contains(p, b"add up")
        && !contains(p, b"altogether")
    {
        return None;
    }
    let mut sum: i64 = 0;
    let mut count: i64 = 0;
    let mut i = 0;
    while i < p.len() {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            sum += v;
            count += 1;
        } else {
            i += 1;
        }
    }
    if count < 2 {
        return None;
    }
    let mut k = 0usize;
    let pre = b"SUM = ";
    out[..pre.len()].copy_from_slice(pre);
    k += pre.len();
    k += fmt_i64(sum, &mut out[k..]);
    let mid = b" (of ";
    out[k..k + mid.len()].copy_from_slice(mid);
    k += mid.len();
    k += fmt_i64(count, &mut out[k..]);
    let suf = b" values, ON-DEVICE)";
    out[k..k + suf.len()].copy_from_slice(suf);
    k += suf.len();
    Some(k)
}

/// PRODUCT of a LIST of non-negative integers in the prompt, checked on-device. Answers "product of 3
/// 4 5" -> "PRODUCT = 60 (of 3 values, ON-DEVICE)" and "multiply 6 and 7" -> "PRODUCT = 42 (of 2
/// values, ON-DEVICE)". The multiplicative sibling of try_sum; like it, scans the WHOLE prompt.
/// Guarded on "product", "multiply", or "multiplied" and needs at least two integers, so a bare
/// keyword with no list falls through to the KB. A running-product overflow (checked_mul) also falls
/// through rather than reporting a bogus value, same convention as try_power's checked_pow. Dispatched
/// after try_sum and before try_math.
fn try_product(p: &[u8], out: &mut [u8]) -> Option<usize> {
    if !contains(p, b"product") && !contains(p, b"multiply") && !contains(p, b"multiplied") {
        return None;
    }
    let mut product: i64 = 1;
    let mut count: i64 = 0;
    let mut i = 0;
    while i < p.len() {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            product = product.checked_mul(v)?;
            count += 1;
        } else {
            i += 1;
        }
    }
    if count < 2 {
        return None; // need at least two values
    }
    let mut k = 0usize;
    let pre = b"PRODUCT = ";
    out[..pre.len()].copy_from_slice(pre);
    k += pre.len();
    k += fmt_i64(product, &mut out[k..]);
    let mid = b" (of ";
    out[k..k + mid.len()].copy_from_slice(mid);
    k += mid.len();
    k += fmt_i64(count, &mut out[k..]);
    let suf = b" values, ON-DEVICE)";
    out[k..k + suf.len()].copy_from_slice(suf);
    k += suf.len();
    Some(k)
}

fn try_math(p: &[u8], out: &mut [u8]) -> Option<usize> {
    for i in 0..p.len() {
        let op = p[i];
        if matches!(op, b'+' | b'-' | b'*' | b'x' | b'/') {
            if let (Some(a), Some(b)) = (int_before(p, i), int_after(p, i)) {
                let r = match op {
                    b'+' => a + b,
                    b'-' => a - b,
                    b'*' | b'x' => a * b,
                    b'/' => {
                        if b == 0 {
                            return None;
                        }
                        a / b
                    }
                    _ => return None,
                };
                let pre = b"ANSWER: ";
                let mut k = pre.len();
                out[..k].copy_from_slice(pre);
                k += fmt_i64(r, &mut out[k..]);
                let suf = b"   (COMPUTED ON-DEVICE, PRIVATELY)";
                out[k..k + suf.len()].copy_from_slice(suf);
                return Some(k + suf.len());
            }
        }
    }
    None
}

/// First integer found in `p` (optional leading `-`), or None.
fn first_int(p: &[u8]) -> Option<i64> {
    let mut i = 0;
    while i < p.len() {
        let neg = p[i] == b'-' && i + 1 < p.len() && p[i + 1].is_ascii_digit();
        if p[i].is_ascii_digit() || neg {
            if neg {
                i += 1;
            }
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            return Some(if neg { -v } else { v });
        }
        i += 1;
    }
    None
}

/// The first two non-negative integers found in `p`, or None if fewer than two.
fn two_ints(p: &[u8]) -> Option<(i64, i64)> {
    let mut vals = [0i64; 2];
    let mut n = 0usize;
    let mut i = 0;
    while i < p.len() && n < 2 {
        if p[i].is_ascii_digit() {
            let mut v = 0i64;
            while i < p.len() && p[i].is_ascii_digit() {
                v = v * 10 + (p[i] - b'0') as i64;
                i += 1;
            }
            vals[n] = v;
            n += 1;
        } else {
            i += 1;
        }
    }
    if n == 2 {
        Some((vals[0], vals[1]))
    } else {
        None
    }
}

/// GCD and LCM of the first two integers in `p` ("gcd of 12 and 18" -> 6,
/// "lcm of 4 and 6" -> 12). Computed on-device. Returns None if no gcd/lcm keyword.
fn try_gcd(p: &[u8], out: &mut [u8]) -> Option<usize> {
    let want_lcm = contains(p, b"lcm");
    let want_gcd = contains(p, b"gcd");
    if !want_lcm && !want_gcd {
        return None;
    }
    let (a, b) = two_ints(p)?;
    if a < 0 || b < 0 {
        return None;
    }
    // Euclid's algorithm on the two non-negative values.
    let (mut x, mut y) = (a as u64, b as u64);
    while y != 0 {
        let t = y;
        y = x % y;
        x = t;
    }
    let g = x;
    if want_lcm {
        // lcm = a / g * b (guard the product in u128); lcm(0, *) = 0.
        if g == 0 {
            return Some(emit_num(out, b"= ", 0, b" (LCM, ON-DEVICE)"));
        }
        let l = (a as u128 / g as u128) * b as u128;
        if l <= i64::MAX as u128 {
            return Some(emit_num(out, b"= ", l as i64, b" (LCM, ON-DEVICE)"));
        }
        return None; // overflow -> decline honestly
    }
    Some(emit_num(out, b"= ", g as i64, b" (GCD, ON-DEVICE)"))
}

/// Format `v` as lowercase hex digits (no prefix) into `buf`; returns the length.
fn fmt_hex(v: i64, buf: &mut [u8]) -> usize {
    let mut x = v as u64;
    if x == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 16];
    let mut n = 0;
    while x > 0 {
        let d = (x & 0xf) as u8;
        tmp[n] = if d < 10 { b'0' + d } else { b'A' + (d - 10) };
        x >>= 4;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

/// Write `pre` + `v` + `suf` into `out`; returns total length.
fn emit_num(out: &mut [u8], pre: &[u8], v: i64, suf: &[u8]) -> usize {
    let mut k = pre.len();
    out[..k].copy_from_slice(pre);
    k += fmt_i64(v, &mut out[k..]);
    out[k..k + suf.len()].copy_from_slice(suf);
    k + suf.len()
}

/// Percentages ("A% of B") and simple unit/base conversions — all on-device.
fn try_convert(p: &[u8], out: &mut [u8]) -> Option<usize> {
    // "A% of B"  ->  A*B/100
    for i in 0..p.len() {
        if p[i] == b'%' && contains(&p[i..], b"of") {
            let mut j = i;
            while j + 1 < p.len() && !(p[j] == b'o' && p[j + 1] == b'f') {
                j += 1;
            }
            if let (Some(a), Some(b)) = (int_before(p, i), int_after(p, j + 1)) {
                return Some(emit_num(out, b"ANSWER: ", a * b / 100, b"   (COMPUTED ON-DEVICE)"));
            }
        }
    }

    let n = first_int(p)?; // conversions all take one number
    // Temperature: Celsius <-> Kelvin (K = C + 273.15, integer-approx). Checked BEFORE the
    // Celsius/Fahrenheit branches because "celsius to kelvin" also contains "celsius".
    if contains(p, b"c to k") || contains(p, b"celsius to kelvin") {
        return Some(emit_num(out, b"= ", n + 273, b" KELVIN (APPROX)"));
    }
    if contains(p, b"k to c") || contains(p, b"kelvin to celsius") {
        return Some(emit_num(out, b"= ", n - 273, b" CELSIUS (APPROX)"));
    }
    if contains(p, b"c to f") || contains(p, b"celsius") {
        return Some(emit_num(out, b"= ", n * 9 / 5 + 32, b" FAHRENHEIT"));
    }
    if contains(p, b"f to c") || contains(p, b"fahrenheit") {
        return Some(emit_num(out, b"= ", (n - 32) * 5 / 9, b" CELSIUS"));
    }
    if contains(p, b"km to mi") || contains(p, b"km to miles") {
        return Some(emit_num(out, b"= ", n * 621 / 1000, b" MILES (APPROX)"));
    }
    if contains(p, b"mi to km") || contains(p, b"miles to km") {
        return Some(emit_num(out, b"= ", n * 1609 / 1000, b" KM (APPROX)"));
    }
    // Weight: kilograms <-> pounds (1 kg = 2.20462 lb; 1 lb = 0.453592 kg), integer-approx.
    if contains(p, b"kg to lb") || contains(p, b"kg to pound") {
        return Some(emit_num(out, b"= ", n * 2205 / 1000, b" LB (APPROX)"));
    }
    if contains(p, b"lb to kg") || contains(p, b"pounds to kg") || contains(p, b"pound to kg") {
        return Some(emit_num(out, b"= ", n * 454 / 1000, b" KG (APPROX)"));
    }
    // Length: meters <-> feet (1 m = 3.28084 ft; 1 ft = 0.3048 m), integer-approx.
    if contains(p, b"m to ft") || contains(p, b"meters to feet") || contains(p, b"metres to feet") {
        return Some(emit_num(out, b"= ", n * 3281 / 1000, b" FT (APPROX)"));
    }
    if contains(p, b"ft to m") || contains(p, b"feet to m") {
        return Some(emit_num(out, b"= ", n * 305 / 1000, b" M (APPROX)"));
    }
    // Length: inches <-> centimeters (1 in = 2.54 cm; 1 cm = 0.393701 in), integer-approx.
    if contains(p, b"in to cm") || contains(p, b"inch to cm") || contains(p, b"inches to cm") {
        return Some(emit_num(out, b"= ", n * 254 / 100, b" CM (APPROX)"));
    }
    if contains(p, b"cm to in") {
        return Some(emit_num(out, b"= ", n * 394 / 1000, b" IN (APPROX)"));
    }
    // Volume: gallons <-> liters (1 US gal = 3.785 L; 1 L = 0.264172 gal), integer-approx.
    if contains(p, b"gal to l") || contains(p, b"gallons to liters") || contains(p, b"gallon to liters") {
        return Some(emit_num(out, b"= ", n * 3785 / 1000, b" LITERS (APPROX)"));
    }
    if contains(p, b"l to gal") || contains(p, b"liters to gallons") || contains(p, b"litres to gallons") {
        return Some(emit_num(out, b"= ", n * 264 / 1000, b" GALLONS (APPROX)"));
    }
    if contains(p, b"hex") {
        let pre = b"IN HEX: 0X";
        let mut k = pre.len();
        out[..k].copy_from_slice(pre);
        k += fmt_hex(n, &mut out[k..]);
        return Some(k);
    }
    None
}

/// Asili AI v0 — an **on-device, private, capability-gated** responder.
///
/// Honest framing: this is a **rule-based responder** (plus real on-device
/// arithmetic) standing in for the quantized on-device model ([`docs/09`]). The
/// *architecture* is what's real — AI as a sandboxed OS service reached only via a
/// granted capability, prompt + reply never leaving the device. It handles a wide
/// set of questions so it feels useful; it is not a neural net yet.
/// Wall-clock seconds since 1970 from the board's clock, for `time` and the clock answer (claude/nia-time,
/// OS_REQUESTS item 7). nia.rs is shared by both kernels, so it reads the *resolved* RTC the way `vfs.rs` does:
/// on ARM virt the PL031 data register (device-tree corrected); on the PC a word `board::resolved()` has just
/// refreshed from the CMOS clock. A board with no clock we read (Pi, Pine64: rtc 0) gives 0; address 0 is
/// never read.
fn wall_secs() -> u32 {
    let base = crate::board::resolved().rtc;
    if base == 0 {
        return 0;
    }
    // SAFETY: a non-zero resolved RTC base is the active board's readable clock word (MMIO on ARM, the CMOS
    // shadow on the PC).
    unsafe { core::ptr::read_volatile(base as *const u32) }
}

fn asili_respond(prompt: &[u8], out: &mut [u8]) -> usize {
    let mut low = [0u8; 128];
    let n = prompt.len().min(128);
    for i in 0..n {
        low[i] = to_lower(prompt[i]);
    }
    let p = &low[..n];

    // 1) Real computation first: percentages/conversions, then arithmetic.
    if let Some(len) = try_convert(p, out) {
        return len;
    }
    if let Some(len) = try_squares(p, out) {
        return len;
    }
    // fibonacci before gcd/factorize/math so "the 10th fibonacci" computes the sequence
    if let Some(len) = try_fib(p, out) {
        return len;
    }
    if let Some(len) = try_gcd(p, out) {
        return len;
    }
    // factorial before factorize: "5 factorial" must not be caught by the "factor" guard
    if let Some(len) = try_factorial(p, out) {
        return len;
    }
    // factorize before prime so "prime factors of 60" factorizes (not a primality check)
    if let Some(len) = try_factorize(p, out) {
        return len;
    }
    if let Some(len) = try_prime(p, out) {
        return len;
    }
    // power before math: "2 to the power of 10" / "2^10" is exponentiation, and '^' is not in
    // try_math's operator set, so it would otherwise fall through to the knowledge base.
    if let Some(len) = try_power(p, out) {
        return len;
    }
    // digit sum before math: "sum of digits of 9999" has no operator, so it would otherwise
    // fall through to the knowledge base rather than summing the decimal digits.
    if let Some(len) = try_digitsum(p, out) {
        return len;
    }
    // even/odd before math: "is 42 even or odd" carries no arithmetic operator.
    if let Some(len) = try_evenodd(p, out) {
        return len;
    }
    // average/mean before math: "average of 10 20 30" scans a whole list, no operator.
    if let Some(len) = try_average(p, out) {
        return len;
    }
    // max/min before math: "max of 3 8 5" scans a whole list, no operator.
    if let Some(len) = try_maxmin(p, out) {
        return len;
    }
    // range/spread before math: "range of 3 8 5" scans a whole list, no operator.
    if let Some(len) = try_range(p, out) {
        return len;
    }
    // sum/total before math (but after digitsum): "sum of 3 8 5" scans a whole list, no operator.
    if let Some(len) = try_sum(p, out) {
        return len;
    }
    // product/multiply before math: "product of 3 4 5" / "multiply 6 and 7" scan a whole list, no
    // arithmetic operator (try_math only fires on +-*x/ symbols).
    if let Some(len) = try_product(p, out) {
        return len;
    }
    if let Some(len) = try_math(p, out) {
        return len;
    }

    // 2) The on-device clock (real hardware read).
    if contains(p, b"time") || contains(p, b"clock") || contains(p, b"date") {
        let t = wall_secs() as i64;
        let pre = b"THE ON-DEVICE CLOCK READS ";
        let mut k = pre.len();
        out[..k].copy_from_slice(pre);
        k += fmt_i64(t, &mut out[k..]);
        let suf = b" SECONDS SINCE 1970.";
        out[k..k + suf.len()].copy_from_slice(suf);
        return k + suf.len();
    }

    // 3) Knowledge base.
    // AfriCore sovereign-service entries are checked FIRST so a name like "afristore" matches its
    // own answer rather than the generic "store" (signing/verification) branch further down.
    let reply: &[u8] = if contains(p, b"afriid") || contains(p, b"afri id") {
        b"AFRIID IS THE SOVEREIGN IDENTITY SERVICE: SELF-OWNED, ON-DEVICE KEYS - YOU AUTHENTICATE WITHOUT A CENTRAL GATEKEEPER."
    } else if contains(p, b"afripay") || contains(p, b"afri pay") {
        b"AFRIPAY IS THE SOVEREIGN PAYMENT RAILS: OPEN, LOW-FEE VALUE TRANSFER - THE MONETIZATION LAYER WITHOUT A STORE TOLL."
    } else if contains(p, b"afripush") || contains(p, b"afri push") {
        b"AFRIPUSH IS THE NOTIFICATION SERVICE: CAPABILITY-GATED PUSH MESSAGES - APPS REACH YOU WITHOUT A THIRD-PARTY TRACKER."
    } else if contains(p, b"afrishare") || contains(p, b"afri share") {
        b"AFRISHARE IS THE PEER-TO-PEER SHARING SERVICE: DEVICE-TO-DEVICE APP AND FILE TRANSFER, NO CLOUD MIDDLEMAN REQUIRED."
    } else if contains(p, b"afristore") || contains(p, b"afri store") {
        b"AFRISTORE IS THE FEDERATED APP STORE: NO GATEKEEPER, SIDELOADING IS A RIGHT - ANYONE CAN PUBLISH AND ANYONE CAN HOST."
    } else if contains(p, b"who") || contains(p, b"your name") || contains(p, b"what are you") {
        b"I AM ASILI - AFRISOVERAS ON-DEVICE AI. PRIVATE BY DEFAULT: YOUR PROMPT NEVER LEFT THIS DEVICE."
    } else if contains(p, b"hello") || contains(p, b"hey") {
        b"HELLO! ASILI HERE - RUNNING ON-DEVICE, NO CLOUD, NO TRACKING."
    } else if contains(p, b"help") || contains(p, b"what can you") || contains(p, b"can you do") {
        b"TRY: MATH (6 X 7), 20% OF 150, SQRT 144, SQUARE OF 12, IS 17 PRIME, FACTORIZE 60, 100 C TO F, 5 KM TO MILES, 10 KG TO LB, 2 M TO FT, 255 IN HEX, THE TIME, OR ASK ABOUT AFRISOVERA/PRIVACY/SECURITY/VOICE/NETWORK."
    } else if contains(p, b"voice") || contains(p, b"speak") || contains(p, b"sound") || contains(p, b"tts") {
        b"ASILI VOICE SPEAKS ON-DEVICE VIA AN OPEN TTS PATH - NO CLOUD. IT DRIVES REAL AUDIO HARDWARE (VIRTIO-SOUND)."
    } else if contains(p, b"network") || contains(p, b"internet") || contains(p, b"ping") || contains(p, b"dns") {
        b"AFRISOVERA HAS A VIRTIO-NET DRIVER: ARP, IPV4, ICMP PING, AND UDP/DNS ALL WORK - APPS REACH THE NET ONLY WITH A CAPABILITY."
    } else if contains(p, b"sign") || contains(p, b"store") || contains(p, b"verif") {
        b"AFRISTORE HAS NO GATEKEEPER BUT IS VERIFIABLE: SHA-256 INTEGRITY + ED25519 PUBLISHER SIGNATURES, CHECKED ON-DEVICE."
    } else if contains(p, b"open source") || contains(p, b"license") || contains(p, b"free") {
        b"AFRISOVERA IS OPEN SOURCE (APACHE-2.0 CORE). YOU CAN READ, BUILD, FORK, AND REFLASH IT - SOVEREIGNTY BY DESIGN."
    } else if contains(p, b"cross") || contains(p, b"platform") || contains(p, b"raspberry") || contains(p, b"phone") || contains(p, b"board") {
        b"ONE CODEBASE, MANY MACHINES: THE SAME KERNEL RUNS ON QEMU VIRT AND A RASPBERRY PI (MMU, HEAP, FRAMEBUFFER)."
    } else if contains(p, b"sovereign") || contains(p, b"own") || contains(p, b"control") {
        b"SOVEREIGNTY MEANS YOU OWN THE DEVICE, THE CODE, AND THE DATA - NO REMOTE KILL SWITCH, NO GATEKEEPER."
    } else if contains(p, b"privacy") || contains(p, b"private") || contains(p, b"track") {
        b"PRIVACY IS THE DEFAULT: I RUN ON YOUR DEVICE BEHIND A CAPABILITY. NOTHING IS SENT TO ANY CLOUD."
    } else if contains(p, b"secur") || contains(p, b"malware") || contains(p, b"virus") || contains(p, b"safe") {
        b"MALWARE CANNOT BREATHE HERE - NO AMBIENT AUTHORITY. AN APP DOES ONLY WHAT ITS CAPABILITIES ALLOW."
    } else if contains(p, b"capabilit") {
        b"A CAPABILITY IS AN UNFORGEABLE TOKEN GRANTING ONE SPECIFIC RIGHT. NO AMBIENT AUTHORITY: AN APP HOLDS ONLY THE CAPABILITIES YOU GRANT."
    } else if contains(p, b"sandbox") {
        b"EVERY APP RUNS SANDBOXED IN ITS OWN WASM ADDRESS SPACE, REACHING THE OUTSIDE WORLD ONLY THROUGH CAPABILITY-GATED CALLS."
    } else if contains(p, b"sideload") {
        b"SIDELOADING IS A RIGHT: INSTALL ANY APP FROM ANY SOURCE. NO GATEKEEPER STORE CAN BLOCK IT (SEE DOCS/08)."
    } else if contains(p, b"bootloader") || contains(p, b"unlock") {
        b"THE BOOTLOADER IS UNLOCKABLE: YOU CAN REFLASH AND RUN YOUR OWN BUILD. THE DEVICE IS YOURS TO CONTROL."
    } else if contains(p, b"nia") {
        b"NIA IS THE WASM APP RUNTIME: EVERY APP IS A SANDBOXED WEBASSEMBLY MODULE, CAPABILITY-GATED, WRITEABLE IN ANY LANGUAGE."
    } else if contains(p, b"daraja") {
        b"DARAJA IS THE FOREIGN-APP BRIDGE: IT RUNS ANDROID APPS BOXED AND CAPABILITY-GATED (WINDOWS/IOS ARE HARDER - SEE DOCS/06)."
    } else if contains(p, b"sankofa") {
        b"SANKOFA UI IS THE DECLARATIVE, VULKAN-RENDERED UI FRAMEWORK - ONE DESIGN LANGUAGE, TUNED FOR WEAK GPUS."
    } else if contains(p, b"tonga") {
        b"TONGA IS THE DISPLAY COMPOSITOR: IT OWNS THE FRAMEBUFFER AND COMPOSITES APP-OWNED WINDOWS OVER CAPABILITY-SECURED IPC."
    } else if contains(p, b"mansa") {
        b"MANSA IS THE MICROKERNEL: A TINY RUST CAPABILITY KERNEL - ADDRESS SPACES, THREADS, IPC, CAPABILITIES. EVERYTHING ELSE IS USERSPACE."
    } else if contains(p, b"afrisovera") || contains(p, b"kernel") || contains(p, b" os") {
        b"AFRISOVERA IS A SOVEREIGN, OPEN, CAPABILITY-SECURE OS. YOU OWN THE DEVICE AND EVERYTHING ON IT."
    } else if contains(p, b"wasm") || contains(p, b"app") {
        b"APPS ARE WASM: WRITE ONCE IN ANY LANGUAGE, RUN ANYWHERE, SANDBOXED AND CAPABILITY-GATED."
    } else if contains(p, b"thank") {
        b"YOU ARE WELCOME."
    } else if contains(p, b"bye") {
        b"GOODBYE - AND YOUR DATA STAYS WITH YOU."
    } else {
        b"I AM ASILI V0 (RULE-BASED STUB; REAL MODEL = QUANTIZED ON-DEVICE LLM, DOCS/09). TRY: WHO ARE YOU, THE TIME, OR 6 X 7."
    };
    let m = reply.len().min(out.len());
    out[..m].copy_from_slice(&reply[..m]);
    m
}

/// Public entry for the kernel's `SYS_AI` syscall (used by EL0 apps / the
/// userspace Nia interpreter): run the on-device responder on `prompt`, write the
/// reply into `out`, return its length.
pub fn respond(prompt: &[u8], out: &mut [u8]) -> usize {
    asili_respond(prompt, out)
}

/// Host `ai(prompt_ptr, prompt_len, out_ptr, out_cap)` → reply length.
fn host_ai(pptr: i32, plen: i32, optr: i32, ocap: i32) -> Result<i32, &'static str> {
    let mut pb = [0u8; 128];
    let pn = ulen(plen, 128);
    let p = mem_range(pptr, pn)?;
    for i in 0..pn {
        pb[i] = mem_read8(p + i);
    }
    crate::kprintln!(
        "    [wasm app] ai(\"{}\")  [on-device, private]",
        core::str::from_utf8(&pb[..pn]).unwrap_or("?")
    );
    let mut ob = [0u8; 256];
    let on = asili_respond(&pb[..pn], &mut ob);
    let n = on.min(ulen(ocap, 256));
    let o = mem_range(optr, n)?;
    for i in 0..n {
        mem_write8(o + i, ob[i]);
    }
    Ok(n as i32)
}

/// Host `file_read(name_ptr, name_len, buf_ptr, buf_cap)` → bytes read / -1.
fn host_file_read(nptr: i32, nlen: i32, bptr: i32, cap: i32) -> Result<i32, &'static str> {
    let mut nb = [0u8; 32];
    let name = read_name(nptr, nlen, &mut nb)?;
    match crate::vfs::read_raw(name) {
        Some(bytes) => {
            let n = bytes.len().min(cap as u32 as usize);
            let b = mem_range(bptr, n)?;
            for i in 0..n {
                mem_write8(b + i, bytes[i]);
            }
            crate::kprintln!("    [wasm app] file_read('{}') -> {} bytes", name, n);
            Ok(n as i32)
        }
        None => {
            crate::kprintln!("    [wasm app] file_read('{}') -> not found", name);
            Ok(-1)
        }
    }
}

/// One WebAssembly name (a uleb length, then that many bytes) at `*c`, which must end at or before `end`
/// (the section's end). The adds are checked, so a 10-byte LEB length near usize::MAX is refused, never an
/// overflow (claude/nia-afri, architect's A1). Advances `*c` past the name.
fn name_in<'m>(m: &'m [u8], c: &mut usize, end: usize) -> Option<&'m [u8]> {
    if *c >= end {
        return None;
    }
    let (n, a) = uleb(m, *c);
    let start = c.checked_add(a)?;
    let stop = start.checked_add(n)?;
    if stop > end {
        return None;
    }
    *c = stop;
    Some(&m[start..stop])
}

/// The load-time line for an import from a module other than `afri` or `env`. The name is the app's own text,
/// so it is printed as escaped bytes: printable ASCII as it is, anything else (ESC, bidi, any byte from 0x80) as
/// \xNN, and at most 32 bytes of it (architect's A3).
fn refuse_module(module: &[u8]) {
    crate::kprint!("[nia] import from module ");
    print_quoted(module);
    crate::kprintln!(" (Nia offers only 'afri')");
}

/// An app's own bytes on the console, safely: in quotes, printable ASCII as it is, anything else (ESC, bidi,
/// any byte from 0x80, backslash, quote) as \xNN, at most 32 bytes, then "and N more bytes".
fn print_quoted(text: &[u8]) {
    crate::kprint!("'");
    for &b in text.iter().take(32) {
        if (b' '..=b'~').contains(&b) && b != b'\\' && b != b'\'' {
            crate::kprint!("{}", b as char);
        } else {
            crate::kprint!("\\x{:02x}", b);
        }
    }
    if text.len() > 32 {
        crate::kprint!("' and {} more bytes", text.len() - 32);
    } else {
        crate::kprint!("'");
    }
}

/// The capability bit for one permission name in an `afri.permissions` section (AFR-0003 section 6), exact
/// bytes. An unknown name gives None and the module is refused (fail closed): a newer app asking for a
/// permission this kernel does not know will not load here.
fn perm_bit(name: &[u8]) -> Option<u32> {
    Some(match name {
        b"print" => CAP_PRINT,
        b"putc" => CAP_PUTC,
        b"draw" => CAP_DRAW,
        b"file" => CAP_FILE,
        b"time" => CAP_TIME,
        b"input" => CAP_INPUT,
        b"ai" => CAP_AI,
        b"voice" => CAP_VOICE,
        b"net" => CAP_NET,
        b"random" => CAP_RANDOM,
        _ => return None,
    })
}

/// Most names an `afri.permissions` section may hold.
const MAX_PERMS: usize = 16;

// --- LEB128 (bounds-safe: never index past the module; malformed input just
// stops early instead of panicking, and the parser's length guards reject it) --
fn uleb(b: &[u8], start: usize) -> (usize, usize) {
    let mut c = start;
    let mut r = 0usize;
    let mut s = 0;
    while c < b.len() {
        let byte = b[c];
        c += 1;
        r |= ((byte & 0x7f) as usize) << s;
        if byte & 0x80 == 0 {
            break;
        }
        s += 7;
        if s >= usize::BITS as usize {
            break; // absurdly long LEB — stop rather than shift-overflow
        }
    }
    (r, c - start)
}
fn sleb64(b: &[u8], start: usize) -> (i64, usize) {
    let mut c = start;
    let mut r = 0i64;
    let mut s = 0u32;
    let mut byte = 0u8;
    while c < b.len() {
        byte = b[c];
        c += 1;
        r |= ((byte & 0x7f) as i64) << s;
        s += 7;
        if byte & 0x80 == 0 {
            break;
        }
        if s >= 64 {
            break;
        }
    }
    if s < 64 && (byte & 0x40) != 0 {
        r |= -1i64 << s;
    }
    (r, c - start)
}
fn sleb(b: &[u8], start: usize) -> (i32, usize) {
    let (v, a) = sleb64(b, start);
    (v as i32, a)
}

fn instr_end(code: &[u8], pc: usize) -> usize {
    if pc >= code.len() {
        return code.len();
    }
    let op = code[pc];
    let p = pc + 1;
    match op {
        0x02 | 0x03 | 0x04 => p + 1,
        0x0C | 0x0D | 0x10 | 0x20 | 0x21 | 0x22 | 0x23 | 0x24 => {
            let (_, a) = uleb(code, p);
            p + a
        }
        0x0E => {
            // br_table: a uleb count, then count+1 label ulebs (labels + default).
            // The count comes from the app: each label takes at least one byte, so a count
            // that is not below the bytes left is refused before the loop (and before
            // count + 1), and a read that takes no bytes stops it. Returning the body's end
            // makes matching() report an unclosed block (claude/nia-brtable, fuzz finding 1).
            let (count, a) = uleb(code, p);
            let mut q = p + a;
            if count >= code.len().saturating_sub(q) {
                return code.len();
            }
            for _ in 0..count + 1 {
                let (_, a2) = uleb(code, q);
                if a2 == 0 {
                    return code.len();
                }
                q += a2;
            }
            q
        }
        0x28..=0x3E => {
            let (_, a) = uleb(code, p);
            let (_, b) = uleb(code, p + a);
            p + a + b
        }
        0x41 | 0x42 => {
            // i32.const / i64.const — skip the signed-LEB immediate (sleb reads
            // up to 64 bits, so it returns the right byte count for i64 too).
            let (_, a) = sleb64(code, p);
            p + a
        }
        0x43 => p + 4, // f32.const — 4-byte IEEE-754 immediate
        0x44 => p + 8, // f64.const — 8-byte IEEE-754 immediate
        0x3F | 0x40 => p + 1, // memory.size / memory.grow — 1-byte memory index
        _ => p,
    }
}

fn matching(code: &[u8], from: usize) -> (Option<usize>, usize) {
    let mut pc = from;
    let mut depth = 0i32;
    let mut else_pc = None;
    loop {
        if pc >= code.len() {
            return (None, code.len());
        }
        let op = code[pc];
        match op {
            0x02 | 0x03 | 0x04 => {
                depth += 1;
                pc = instr_end(code, pc);
            }
            0x05 if depth == 0 => {
                else_pc = Some(pc + 1);
                pc += 1;
            }
            0x0B => {
                if depth == 0 {
                    return (else_pc, pc);
                }
                depth -= 1;
                pc += 1;
            }
            _ => pc = instr_end(code, pc),
        }
    }
}

// Nia speed, step 1 (claude/nia-fast): the block-end memo. `matching` walks forward to a block's `end` every
// time a block or an `if` is entered, so a loop with blocks inside paid for a scan of those blocks on every pass.
// The answer depends only on the code bytes, and they do not change while an app runs, so it is kept here
// after the first scan. A slot is used only when the place (the address of the block's first byte) AND the
// end of the function's code are the same as when it was stored, so it always gives what `matching` would.
// `parse` empties it before every app: a slot from an earlier app is never seen by the next.
// 64 slots x 24 bytes = 1.5 KiB in .bss. A miss, or a value too big to store, just calls `matching`.
const MEMO_SLOTS: usize = 64;
#[derive(Clone, Copy)]
struct Memo {
    at: usize,       // address of the byte after the block type (0 = empty slot)
    code_end: usize, // address just past the function's code
    end_pc: u32,
    else_pc: u32, // 0 = no else (a real else_pc is always at least 1)
}
const MEMO_EMPTY: Memo = Memo { at: 0, code_end: 0, end_pc: 0, else_pc: 0 };
static mut MEMO: [Memo; MEMO_SLOTS] = [MEMO_EMPTY; MEMO_SLOTS];
// `wasmbench` turns the memo off to time the old way in the same kernel; everything else leaves it on.
static mut MEMO_ON: bool = true;

fn memo_reset() {
    // SAFETY: single-threaded (one Nia app at a time, nested runs are refused).
    unsafe {
        *addr_of_mut!(MEMO) = [MEMO_EMPTY; MEMO_SLOTS];
    }
}

fn memo_on() -> bool {
    // SAFETY: single-threaded, as memo_reset.
    unsafe { MEMO_ON }
}

/// `matching(code, from)`, from the memo when it has it.
fn block_ends(code: &[u8], from: usize) -> (Option<usize>, usize) {
    // SAFETY: single-threaded, as memo_reset.
    unsafe {
        if !MEMO_ON {
            return matching(code, from);
        }
        let base = code.as_ptr() as usize;
        let at = base + from;
        let code_end = base + code.len();
        let slot = (at ^ (at >> 6)) % MEMO_SLOTS;
        let memo = &mut *addr_of_mut!(MEMO);
        let e = memo[slot];
        if e.at == at && e.code_end == code_end {
            let else_pc = if e.else_pc == 0 { None } else { Some(e.else_pc as usize) };
            return (else_pc, e.end_pc as usize);
        }
        let (else_pc, end_pc) = matching(code, from);
        let el = else_pc.unwrap_or(0);
        if end_pc <= u32::MAX as usize && el <= u32::MAX as usize {
            memo[slot] = Memo { at, code_end, end_pc: end_pc as u32, else_pc: el as u32 };
        }
        (else_pc, end_pc)
    }
}

// The value stack holds 64-bit cells so it can carry i64 values. i32 ops keep
// their exact semantics: `push` stores the i32 sign-extended into the cell and
// `pop` truncates the low 32 bits back out, so any pure-i32 sequence behaves
// byte-for-byte as before. i64 ops use `push_i64`/`pop_i64` to move the full
// 64-bit value. (In a well-typed module an i64 cell is only ever consumed by an
// i64 op, so the widening is transparent.)
//
// Nia guard: a push onto a full stack or a pop from an empty one never indexes
// out of range. It sets `bad` instead (the push is dropped, the pop yields 0),
// and the interpreter turns `bad` into a TRAP: at the top of every instruction,
// after the arguments of a call are popped, before a branch carries values and
// before a function hands back its results.
struct Stack {
    d: [i64; STACK_CAP],
    sp: usize,
    bad: bool,
}
impl Stack {
    fn new() -> Self {
        Stack { d: [0; STACK_CAP], sp: 0, bad: false }
    }
    fn push(&mut self, v: i32) {
        self.push_i64(v as i64); // sign-extend the i32 into the 64-bit cell
    }
    fn pop(&mut self) -> i32 {
        self.pop_i64() as i32 // truncate back to i32 (low 32 bits)
    }
    fn top_i64(&mut self) -> i64 {
        if self.sp == 0 {
            self.bad = true;
            return 0;
        }
        self.d[self.sp - 1]
    }
    fn push_i64(&mut self, v: i64) {
        if self.sp >= STACK_CAP {
            self.bad = true;
            return;
        }
        self.d[self.sp] = v;
        self.sp += 1;
    }
    fn pop_i64(&mut self) -> i64 {
        if self.sp == 0 {
            self.bad = true;
            return 0;
        }
        self.sp -= 1;
        self.d[self.sp]
    }
    /// The TRAP for a stack that went wrong, if it did.
    fn check(&self) -> Result<(), &'static str> {
        if self.bad {
            Err("value stack overflow or underflow")
        } else {
            Ok(())
        }
    }
    // f64 values ride in the same 64-bit cell as a raw IEEE-754 bit pattern, so
    // no separate float stack is needed.
    fn push_f64(&mut self, v: f64) {
        self.push_i64(v.to_bits() as i64);
    }
    fn pop_f64(&mut self) -> f64 {
        f64::from_bits(self.pop_i64() as u64)
    }
    // f32 rides in the low 32 bits of the cell (push/pop already sign-extend /
    // truncate; from_bits recovers the exact 32-bit pattern).
    fn push_f32(&mut self, v: f32) {
        self.push(v.to_bits() as i32);
    }
    fn pop_f32(&mut self) -> f32 {
        f32::from_bits(self.pop() as u32)
    }
}

struct Ctrl {
    is_loop: bool,
    // Set only by `if` (0x04): `else` (0x05) is refused in any other frame (claude/nia-ctrl). In a loop frame
    // cont_pc is the loop's start, so an unchecked `else` jumped back into the body without passing `branch()`,
    // the only place loop_poll runs: an endless loop Ctrl+C could not stop.
    is_if: bool,
    cont_pc: usize,
    stack_base: usize,
    arity: usize,
}

struct Module<'a> {
    bytes: &'a [u8],
    types: [(u16, u16); MAX_TYPES], // (params, results); more than 65,535 is refused at load
    func_type: [u8; MAX_FUNCS],     // a type index, below MAX_TYPES
    code_off: [u32; MAX_FUNCS],
    code_len: [u32; MAX_FUNCS],
    import_host: [u8; MAX_IMPORTS],
    import_type: [usize; MAX_IMPORTS],
    nfuncs: usize,
    ntypes: usize,
    import_count: usize,
    run_index: usize,
    caps: u32,
    // The app's `afri.permissions` section (claude/nia-perms, OS_REQUESTS item 8 step B): whether it has one,
    // and the permissions it names. With a section, `caps` is narrowed to `caps & asked` after the parse.
    listed: bool,
    asked: u32,
    // WASM functions active now (Nia guard). A fresh Module per run_app, so it
    // starts at 0 for every app; `invoke` brings it back down on every way out.
    depth: Cell<usize>,
}

pub fn run_demo() {
    run_app("compute: sum(1..10) via a loop", WASM_SUM, CAP_ALL);
    run_app("linear memory: print a string (putc cap GRANTED)", WASM_STR, CAP_PUTC);
    run_app("same app, putc cap DENIED -> must be refused", WASM_STR, 0);
}

/// The embedded demo modules, exposed so they can be seeded into the VFS as
/// `.wasm` files a user can then load and run.
pub fn hello_wasm() -> &'static [u8] {
    WASM_STR
}
pub fn sum_wasm() -> &'static [u8] {
    WASM_SUM
}
pub fn draw_wasm() -> &'static [u8] {
    WASM_DRAW
}
pub fn file_wasm() -> &'static [u8] {
    WASM_FILE
}
pub fn clock_wasm() -> &'static [u8] {
    WASM_CLOCK
}
pub fn type_wasm() -> &'static [u8] {
    WASM_TYPE
}
pub fn ai_wasm() -> &'static [u8] {
    WASM_AI
}
pub fn chat_wasm() -> &'static [u8] {
    WASM_CHAT
}
pub fn notify_wasm() -> &'static [u8] {
    WASM_NOTIFY
}
pub fn notify2_wasm() -> &'static [u8] {
    WASM_NOTIFY2
}
pub fn net_wasm() -> &'static [u8] {
    WASM_NET
}

// net.wasm: run() { print(net(0)); print(net(1)); } — pings then resolves DNS via
// the capability-gated `net` host function (needs CAP_NET + a virtio-net device).
#[rustfmt::skip]
static WASM_NET: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0D, 0x03, 0x60,
    0x01, 0x7F, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02,
    0x17, 0x02, 0x03, 0x65, 0x6E, 0x76, 0x03, 0x6E, 0x65, 0x74, 0x00, 0x00,
    0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x01,
    0x03, 0x02, 0x01, 0x02, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00,
    0x02, 0x0A, 0x10, 0x01, 0x0E, 0x00, 0x41, 0x00, 0x10, 0x00, 0x10, 0x01,
    0x41, 0x01, 0x10, 0x00, 0x10, 0x01, 0x0B,
];

/// Demonstrate capability-gated networking: run the same net app twice — once with
/// CAP_NET granted (it reaches the network) and once without (the host call is
/// refused). Proves capability security on a real resource.
pub fn net_demo() {
    if !crate::virtio::net_present() {
        crate::kprintln!("[nia] net demo needs a virtio-net device (see scripts/net-test.sh)");
        return;
    }
    run_app("net.wasm — CAP_NET GRANTED", WASM_NET, CAP_ALL);
    run_app("net.wasm — CAP_NET DENIED", WASM_NET, CAP_ALL & !CAP_NET);
}

pub fn rand_wasm() -> &'static [u8] {
    WASM_RAND
}

/// Andika, the text editor (native apps engineer, nia-apps): the first bundled app over 16 KiB, so it runs only from
/// these bytes (A7). Built by nia-apps tools/build.sh at a334eb1 (branch andika) on nightly-2026-11-10 with
/// immediate-abort panics; 29,238 bytes, SHA-256 c1568f5a1727baefc971b4fb6b0f9628cd10f0e8421418d7a4f7f86748043a90.
/// Its afri.permissions section is exactly what it uses: draw (the five cell calls) and file (file_read and
/// file_write). Not input or time, which the 12 Nov condition named: its keys come as cell events, under draw.
static WASM_ANDIKA: &[u8] = include_bytes!("andika.wasm");
pub fn andika_wasm() -> &'static [u8] {
    WASM_ANDIKA
}

// rand.wasm: run() { print(random()); print(random()); } — draws twice from the
// capability-gated `random` host function (needs CAP_RANDOM).
//   types: t0 ()->i32 [random], t1 (i32)->() [print], t2 ()->() [run]
//   imports: env.random:t0, env.print:t1
//   code: call random; call print; call random; call print; end
#[rustfmt::skip]
static WASM_RAND: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x0C, 0x03, 0x60, 0x00, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    0x02, 0x1A, 0x02,
        0x03, 0x65, 0x6E, 0x76, 0x06, 0x72, 0x61, 0x6E, 0x64, 0x6F, 0x6D, 0x00, 0x00,
        0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x01,
    0x03, 0x02, 0x01, 0x02,
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    0x0A, 0x0C, 0x01, 0x0A, 0x00, 0x10, 0x00, 0x10, 0x01, 0x10, 0x00, 0x10, 0x01, 0x0B,
];

/// Demonstrate capability-gated randomness: run the same app twice — once with
/// CAP_RANDOM granted (it draws two values) and once without (the host call is
/// refused). Proves the capability model over the RNG resource.
pub fn rand_demo() {
    run_app("rand.wasm — CAP_RANDOM GRANTED", WASM_RAND, CAP_ALL);
    run_app("rand.wasm — CAP_RANDOM DENIED", WASM_RAND, CAP_ALL & !CAP_RANDOM);
}

// dice.wasm: run() { print(random() % 6 + 1); } — a fair 1..6 die roll built on
// the capability-gated RNG. Uses i32.rem_u (0x70) so the u32 draw maps cleanly.
//   code: call random; i32.const 6; i32.rem_u; i32.const 1; i32.add; call print; end
#[rustfmt::skip]
static WASM_DICE: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x0C, 0x03, 0x60, 0x00, 0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    0x02, 0x1A, 0x02,
        0x03, 0x65, 0x6E, 0x76, 0x06, 0x72, 0x61, 0x6E, 0x64, 0x6F, 0x6D, 0x00, 0x00,
        0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x01,
    0x03, 0x02, 0x01, 0x02,
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    0x0A, 0x0E, 0x01, 0x0C, 0x00, 0x10, 0x00, 0x41, 0x06, 0x70, 0x41, 0x01, 0x6A, 0x10, 0x01, 0x0B,
];

// ops.wasm: run() { print((5 << 2) + (100 >> 3)) } = 20 + 12 = 32. Exercises the
// newly-added i32.shl (0x74) and i32.shr_u (0x76) opcodes.
//   code: const 5; const 2; shl; const 100; const 3; shr_u; add; call print; end
#[rustfmt::skip]
static WASM_OPS: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    0x03, 0x02, 0x01, 0x01,
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    0x0A, 0x12, 0x01, 0x10, 0x00,
        0x41, 0x05, 0x41, 0x02, 0x74, 0x41, 0xE4, 0x00, 0x41, 0x03, 0x76, 0x6A, 0x10, 0x00, 0x0B,
];

// --- wasmbench (claude/nia-fast): Nia speed, measured in the running kernel. ---------------------------
// Two apps in the shape rustc and clang emit (scripts/lib/bench-wasm.py writes the same bytes; --expect gives
// the print values): b1 runs an `if` and a `block` inside a 20,000-pass loop, b2 calls a small function with an
// `if` 5,000 times. Each is timed the old way (block-end memo off: a scan to every block's end on every entry)
// and the new way, three runs each, best run kept, in the same kernel, so the two numbers are like for like.
#[rustfmt::skip]
static WASM_BENCH_B1: [u8; 139] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0D, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x60, 0x01, 0x7F, 0x01, 0x7F, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05,
    0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72,
    0x75, 0x6E, 0x00, 0x01, 0x0A, 0x55, 0x01, 0x53, 0x01, 0x02, 0x7F, 0x41, 0x00, 0x21, 0x00, 0x02,
    0x40, 0x03, 0x40, 0x20, 0x00, 0x41, 0xA0, 0x9C, 0x01, 0x4E, 0x0D, 0x01, 0x20, 0x00, 0x41, 0x01,
    0x71, 0x04, 0x40, 0x20, 0x01, 0x20, 0x00, 0x41, 0x03, 0x6C, 0x41, 0x05, 0x73, 0x6A, 0x21, 0x01,
    0x05, 0x20, 0x01, 0x20, 0x00, 0x6B, 0x21, 0x01, 0x0B, 0x02, 0x40, 0x20, 0x00, 0x41, 0x07, 0x71,
    0x45, 0x0D, 0x00, 0x20, 0x01, 0x41, 0x01, 0x6A, 0x21, 0x01, 0x0B, 0x20, 0x00, 0x41, 0x01, 0x6A,
    0x21, 0x00, 0x0C, 0x00, 0x0B, 0x0B, 0x20, 0x01, 0x10, 0x00, 0x0B,
];
#[rustfmt::skip]
static WASM_BENCH_B2: [u8; 123] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0D, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x60, 0x01, 0x7F, 0x01, 0x7F, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05,
    0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x03, 0x02, 0x02, 0x01, 0x07, 0x07, 0x01, 0x03,
    0x72, 0x75, 0x6E, 0x00, 0x02, 0x0A, 0x44, 0x02, 0x15, 0x00, 0x20, 0x00, 0x41, 0x01, 0x71, 0x04,
    0x7F, 0x20, 0x00, 0x41, 0x03, 0x6A, 0x05, 0x20, 0x00, 0x41, 0x01, 0x6B, 0x0B, 0x0B, 0x2C, 0x01,
    0x02, 0x7F, 0x41, 0x00, 0x21, 0x00, 0x02, 0x40, 0x03, 0x40, 0x20, 0x00, 0x41, 0x88, 0x27, 0x4E,
    0x0D, 0x01, 0x20, 0x01, 0x20, 0x00, 0x10, 0x01, 0x6A, 0x21, 0x01, 0x20, 0x00, 0x41, 0x01, 0x6A,
    0x21, 0x00, 0x0C, 0x00, 0x0B, 0x0B, 0x20, 0x01, 0x10, 0x00, 0x0B,
];

/// One timed run of `run` in `bytes`, nothing printed by the runtime itself (only the app's own print).
/// Returns the time in microseconds, or the TRAP or parse error.
fn bench_once(bytes: &[u8]) -> Result<u64, &'static str> {
    putc_reset();
    keyq_reset();
    let m = parse(bytes, CAP_PRINT)?;
    let mut st = Stack::new();
    let f = crate::timer::hz();
    let t0 = crate::timer::cycles();
    let r = m.invoke(m.run_index, &mut st);
    let t1 = crate::timer::cycles();
    finish_putc();
    r?;
    let cyc = t1.wrapping_sub(t0);
    Ok(if f > 0 { cyc.saturating_mul(1_000_000) / f } else { 0 })
}

/// The best of three runs, with the memo on or off.
fn bench_best(bytes: &[u8], on: bool) -> Result<u64, &'static str> {
    // SAFETY: single-threaded; MEMO_ON goes back to true on every way out.
    unsafe { MEMO_ON = on };
    let mut best = u64::MAX;
    let mut out = Ok(0);
    for _ in 0..3 {
        match bench_once(bytes) {
            Ok(us) => best = best.min(us),
            Err(e) => {
                out = Err(e);
                break;
            }
        }
    }
    unsafe { MEMO_ON = true };
    out.map(|_| best)
}

/// Shell `wasmbench`: each bench app the old way and the new way, with the speed-up.
pub fn bench() {
    let apps: [(&str, &[u8]); 2] = [("b1 loop+if+block", &WASM_BENCH_B1), ("b2 calls", &WASM_BENCH_B2)];
    crate::kprintln!("[bench] Nia speed: old = scan to each block's end on entry, new = block-end memo");
    for (name, bytes) in apps {
        let old = bench_best(bytes, false);
        let new = bench_best(bytes, true);
        match (old, new) {
            (Ok(o), Ok(n)) => {
                let x10 = if n > 0 { o.saturating_mul(10) / n } else { 0 };
                crate::kprintln!(
                    "[bench] {}: old {} us, new {} us, {}.{}x",
                    name, o, n, x10 / 10, x10 % 10
                );
            }
            (Err(e), _) | (_, Err(e)) => crate::kprintln!("[bench] {}: TRAP: {}", name, e),
        }
    }
    crate::kprintln!("[bench] done");
}

/// Run a WASM module that uses the shift opcodes and prints (5<<2)+(100>>3) = 32.
pub fn ops_demo() {
    crate::kprintln!("[nia] ops.wasm - computing (5<<2)+(100>>3) with i32.shl/shr_u...");
    run_app("ops.wasm", WASM_OPS, CAP_ALL);
}

// i64.wasm: run() { print64((2^32 * 3) + 7) } = 12884901888 + 7 = 12884901895.
// Exercises i64.const (a value that does NOT fit in 32 bits), i64.mul, i64.add,
// and the print64 host. Imports env.print64 : (i64) -> ().
//   code: i64.const 4294967296; i64.const 3; i64.mul; i64.const 7; i64.add;
//         call print64; end
#[rustfmt::skip]
static WASM_I64: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i64)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7E, 0x00, 0x60, 0x00, 0x00,
    // import: env.print64 : type 0
    0x02, 0x0F, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x07, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x36, 0x34, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x12, 0x01, 0x10, 0x00,
        0x42, 0x80, 0x80, 0x80, 0x80, 0x10, // i64.const 4294967296 (2^32)
        0x42, 0x03,                         // i64.const 3
        0x7E,                               // i64.mul
        0x42, 0x07,                         // i64.const 7
        0x7C,                               // i64.add
        0x10, 0x00,                         // call print64
        0x0B,                               // end
];

/// Run a WASM module that does real 64-bit integer math -- (2^32 * 3) + 7 -- and
/// prints the result (12884901895) via the print64 host. Proves the interpreter
/// carries i64 values that do not fit in 32 bits.
pub fn i64_demo() {
    crate::kprintln!("[nia] i64.wasm - computing (2^32 * 3) + 7 with i64.mul/i64.add...");
    run_app("i64.wasm", WASM_I64, CAP_ALL);
}

// f64.wasm: run() { print( trunc_i32( (10.0 / 4.0) * 2.0 ) ) } = print(5).
// The 2.5 intermediate from 10.0/4.0 cannot come from integer division, so a
// correct result of 5 proves real IEEE-754 f64 math (f64.const/div/mul) plus the
// i32.trunc_f64_s conversion. Uses the existing i32 `print` host (no float print).
//   f64.const 10.0; f64.const 4.0; f64.div; f64.const 2.0; f64.mul;
//   i32.trunc_f64_s; call print; end
#[rustfmt::skip]
static WASM_F64: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x24, 0x01, 0x22,
        0x00,                                              // 0 locals
        0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x24, 0x40, // f64.const 10.0
        0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x40, // f64.const 4.0
        0xA3,                                              // f64.div  -> 2.5
        0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // f64.const 2.0
        0xA2,                                              // f64.mul  -> 5.0
        0xAA,                                              // i32.trunc_f64_s -> 5
        0x10, 0x00,                                        // call print
        0x0B,                                              // end
];

/// Run a WASM module that does real IEEE-754 f64 math -- (10.0 / 4.0) * 2.0 -- then
/// truncates to i32 and prints 5. The fractional 2.5 intermediate proves floating
/// point (integer division could not produce it).
pub fn f64_demo() {
    crate::kprintln!("[nia] f64.wasm - computing (10.0 / 4.0) * 2.0 with f64.div/f64.mul...");
    run_app("f64.wasm", WASM_F64, CAP_ALL);
}

// f32.wasm: run() { print( trunc_i32( (7.0 / 2.0) * 4.0 ) ) } = print(14).
// The 3.5 intermediate from 7.0/2.0 proves single-precision float math (f32.const/
// div/mul) plus i32.trunc_f32_s. Uses the existing i32 print host.
#[rustfmt::skip]
static WASM_F32: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x18, 0x01, 0x16,
        0x00,                         // 0 locals
        0x43, 0x00, 0x00, 0xE0, 0x40, // f32.const 7.0
        0x43, 0x00, 0x00, 0x00, 0x40, // f32.const 2.0
        0x95,                         // f32.div -> 3.5
        0x43, 0x00, 0x00, 0x80, 0x40, // f32.const 4.0
        0x94,                         // f32.mul -> 14.0
        0xA8,                         // i32.trunc_f32_s -> 14
        0x10, 0x00,                   // call print
        0x0B,                         // end
];

/// Run a WASM module that does real IEEE-754 f32 math -- (7.0 / 2.0) * 4.0 -- then
/// truncates to i32 and prints 14. The 3.5 intermediate proves single-precision
/// floating point.
pub fn f32_demo() {
    crate::kprintln!("[nia] f32.wasm - computing (7.0 / 2.0) * 4.0 with f32.div/f32.mul...");
    run_app("f32.wasm", WASM_F32, CAP_ALL);
}

// memsz.wasm: run() { print(memory.size) } = print(1). The module declares
// (memory 1); memory.size returns the current size in 64KiB pages = 1.
#[rustfmt::skip]
static WASM_MEMSZ: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // memory: (memory 1)
    0x05, 0x03, 0x01, 0x00, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x08, 0x01, 0x06,
        0x00,        // 0 locals
        0x3F, 0x00,  // memory.size (mem idx 0)
        0x10, 0x00,  // call print
        0x0B,        // end
];

/// Run a WASM module that reads its linear-memory size and prints it (1 page).
pub fn memsz_demo() {
    crate::kprintln!("[nia] memsz.wasm - memory.size (in 64KiB pages)...");
    run_app("memsz.wasm", WASM_MEMSZ, CAP_ALL);
}

// brtable.wasm: run() switches on index 1 via br_table across two nested blocks.
// Inside the inner block, label 0 = inner, label 1 = outer; index 1 breaks out of
// the OUTER block, skipping print(10), landing at print(20). Proves br_table's
// indexed multi-way branch (a WASM switch) selects the right target.
#[rustfmt::skip]
static WASM_BRTABLE: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x1A, 0x01, 0x18,
        0x00,                   // 0 locals
        0x02, 0x40,             // block $b1
        0x02, 0x40,             // block $b0
        0x41, 0x01,             // i32.const 1 (switch index)
        0x0E, 0x02, 0x00, 0x01, 0x00, // br_table [0,1] default 0
        0x0B,                   // end $b0
        0x41, 0x0A,             // i32.const 10
        0x10, 0x00,             // call print
        0x0F,                   // return
        0x0B,                   // end $b1
        0x41, 0x14,             // i32.const 20
        0x10, 0x00,             // call print
        0x0B,                   // end func
];

/// Run a WASM module that uses br_table (a switch) to select a branch by index.
pub fn brtable_demo() {
    crate::kprintln!("[nia] brtable.wasm - br_table switch on index 1 (expect print(20))...");
    run_app("brtable.wasm", WASM_BRTABLE, CAP_ALL);
}

// glob.wasm: a mutable i32 global g = 10. run() { g += 5; print(g) } = print(15).
// Exercises the global section (init expr), global.get, and global.set -- proving
// modules can hold mutable state across ops (the shadow stack pointer, counters).
#[rustfmt::skip]
static WASM_GLOB: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0
    0x02, 0x0D, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // global: one mutable i32 = 10  (valtype 0x7F, mut 0x01, init: i32.const 10, end)
    0x06, 0x06, 0x01, 0x7F, 0x01, 0x41, 0x0A, 0x0B,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x0F, 0x01, 0x0D,
        0x00,             // 0 locals
        0x23, 0x00,       // global.get 0   (10)
        0x41, 0x05,       // i32.const 5
        0x6A,             // i32.add        (15)
        0x24, 0x00,       // global.set 0   (g = 15)
        0x23, 0x00,       // global.get 0   (15)
        0x10, 0x00,       // call print
        0x0B,             // end
];

/// Run a WASM module with a mutable global initialized to 10, that adds 5 and
/// prints the result (15) -- proving global.get/global.set + init exprs work.
pub fn glob_demo() {
    crate::kprintln!("[nia] glob.wasm - mutable global (10 += 5) via global.get/global.set...");
    run_app("glob.wasm", WASM_GLOB, CAP_ALL);
}

// i64mem.wasm: run() { mem[0..8] = 0x1_0000_002A (i64.store); print64(i64.load mem[0]) }
// = 4294967338. Round-trips a value that does NOT fit in 32 bits through 64KiB
// linear memory, exercising i64.store + i64.load. Declares (memory 1).
//   code: i32.const 0; i64.const 4294967338; i64.store; i32.const 0; i64.load;
//         call print64; end
#[rustfmt::skip]
static WASM_I64MEM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i64)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7E, 0x00, 0x60, 0x00, 0x00,
    // import: env.print64 : type 0
    0x02, 0x0F, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x07, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x36, 0x34, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // memory: (memory 1)
    0x05, 0x03, 0x01, 0x00, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x16, 0x01, 0x14, 0x00,
        0x41, 0x00,                         // i32.const 0 (store address)
        0x42, 0xAA, 0x80, 0x80, 0x80, 0x10, // i64.const 4294967338 (0x1_0000_002A)
        0x37, 0x03, 0x00,                   // i64.store align=3 offset=0
        0x41, 0x00,                         // i32.const 0 (load address)
        0x29, 0x03, 0x00,                   // i64.load align=3 offset=0
        0x10, 0x00,                         // call print64
        0x0B,                               // end
];

// oob.wasm (claude/nia-addr): run() { print(1); speak(ptr = -1, len = 10); print(99) }. The one host-pointer
// case only a full-capability app reaches (unverified apps are denied voice first). Before nia-addr,
// `a[0] as usize + i` overflowed (-1 is usize::MAX) and panicked the debug kernel; now the pointer is unsigned
// (4294967295), the range is checked, and the app TRAPs with "out of bounds memory access". print(99) never runs.
#[rustfmt::skip]
static WASM_OOB: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i32)->()  [1]=(i32,i32)->()  [2]=()->()
    0x01, 0x0D, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x02, 0x7F, 0x7F, 0x00, 0x60, 0x00, 0x00,
    // import: env.print : type 0, env.speak : type 1
    0x02, 0x19, 0x02, 0x03, 0x65, 0x6E, 0x76, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00,
    0x03, 0x65, 0x6E, 0x76, 0x05, 0x73, 0x70, 0x65, 0x61, 0x6B, 0x00, 0x01,
    // function: one func of type 2; memory: (memory 1); export: "run" = func 2
    0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    // code
    0x0A, 0x13, 0x01, 0x11, 0x00,
        0x41, 0x01, 0x10, 0x00,             // print(1)
        0x41, 0x7F, 0x41, 0x0A, 0x10, 0x01, // speak(-1, 10)
        0x41, 0xE3, 0x00, 0x10, 0x00,       // print(99), never reached
        0x0B,                               // end
];

/// Run a full-capability module that calls `speak` with a pointer of -1: it must TRAP (out of bounds memory
/// access), not panic the kernel.
pub fn oob_demo() {
    crate::kprintln!("[nia] oob.wasm - speak() with a pointer of -1 (must TRAP, not panic)...");
    run_app("oob.wasm", WASM_OOB, CAP_ALL);
}

/// Run a WASM module that stores a >32-bit i64 to linear memory and loads it back
/// -- proving i64 values round-trip through the sandbox's 64KiB memory. Prints
/// 4294967338 via print64.
pub fn i64mem_demo() {
    crate::kprintln!("[nia] i64mem.wasm - round-tripping 4294967338 through linear memory (i64.store/load)...");
    run_app("i64mem.wasm", WASM_I64MEM, CAP_ALL);
}

// i64loc.wasm: run() { local i64 x; x = 0x2_0000_0005; print64(x) } = 8589934597.
// Holds a >32-bit value in an i64 LOCAL across local.set/local.get -- proving
// locals no longer truncate to 32 bits.
//   code: (local i64) i64.const 8589934597; local.set 0; local.get 0;
//         call print64; end
#[rustfmt::skip]
static WASM_I64LOC: &[u8] = &[
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
    // type: [0]=(i64)->()  [1]=()->()
    0x01, 0x08, 0x02, 0x60, 0x01, 0x7E, 0x00, 0x60, 0x00, 0x00,
    // import: env.print64 : type 0
    0x02, 0x0F, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x07, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x36, 0x34, 0x00, 0x00,
    // function: one func of type 1
    0x03, 0x02, 0x01, 0x01,
    // export: "run" = func 1
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    // code
    0x0A, 0x12, 0x01, 0x10,
        0x01, 0x01, 0x7E,                   // locals: 1 group of 1 i64
        0x42, 0x85, 0x80, 0x80, 0x80, 0x20, // i64.const 8589934597 (0x2_0000_0005)
        0x21, 0x00,                         // local.set 0
        0x20, 0x00,                         // local.get 0
        0x10, 0x00,                         // call print64
        0x0B,                               // end
];

/// Run a WASM module that stashes a >32-bit i64 in a local and reads it back --
/// proving i64 locals no longer truncate. Prints 8589934597 via print64.
pub fn i64loc_demo() {
    crate::kprintln!("[nia] i64loc.wasm - holding 8589934597 in an i64 local (local.set/get)...");
    run_app("i64loc.wasm", WASM_I64LOC, CAP_ALL);
}

/// Feed the parser/interpreter a series of MALFORMED modules and confirm each is
/// rejected gracefully (a clean error, never a kernel panic). This is the WASM
/// sandbox's DoS defense: a corrupt or hostile .wasm must not crash the OS.
pub fn fuzz() {
    // Each case is expected to yield a "[nia] parse error" or "[nia] TRAP", not
    // a panic. Valid magic where noted so the corruption is in the sections.
    let empty: &[u8] = &[];
    let magic_only: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
    // section id 1 (type), size uleb = 127, but no bytes follow → truncated section
    let trunc_section: &[u8] =
        &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x7F];
    // export section claiming a name longer than the module → truncated name
    let bad_name: &[u8] = &[
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x07, 0x05, 0x01, 0x40, 0x72, 0x75, 0x6E,
    ];
    // valid type/func/export but a code body with no `end` opcode → runs off end
    let no_end: &[u8] = &[
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
        0x03, 0x02, 0x01, 0x00,
        0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00,
        0x0A, 0x05, 0x01, 0x03, 0x00, 0x41, 0x01,
    ];
    // A 10-byte LEB (2^64 - 1) where the parser reads a length; each overflowed an unchecked add and
    // panicked the kernel until steps A and A2 (scripts/lib/afri-wasm.py af1 to af3,
    // scripts/lib/parse-wasm.py ov1 to ov4 make the same bytes).
    let ov_af1: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00];
    let ov_af2: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x00, 0x00, 0x60, 0x01, 0x7F, 0x00, 0x02, 0x13, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let ov_af3: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x00, 0x00, 0x60, 0x01, 0x7F, 0x00, 0x02, 0x18, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let ov_ov1: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x14, 0x01, 0x60, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let ov_ov2: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x15, 0x01, 0x60, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let ov_ov3: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x13, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let ov_ov4: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x13, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    // run's body is one byte, 0x7F: 127 local groups and no bytes for them (ov5)
    let ov_ov5: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x03, 0x01, 0x01, 0x7F];
    // br_table label counts from the app (claude/nia-brtable, fuzz finding 1; scripts/lib/brtable-wasm.py
    // makes the same bytes): bt1 is the fuzzer's input (about 2^42 labels, the interpreter's loop), bt2 a
    // 10-byte count inside a block (count + 1 overflowed in instr_end), bt3 about 2^42 inside a block.
    let bt1: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0C, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x01, 0x7E, 0x00, 0x60, 0x00, 0x00, 0x02, 0x1D, 0x02, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x04, 0x61, 0x66, 0x72, 0x69, 0x07, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x36, 0x34, 0x00, 0x01, 0x03, 0x02, 0x01, 0x02, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02, 0x0A, 0x27, 0x01, 0x25, 0x00, 0x41, 0x05, 0x10, 0x00, 0x41, 0xF8, 0xFF, 0x03, 0x42, 0x88, 0x3D, 0x0E, 0x98, 0xA8, 0xC0, 0xE0, 0x80, 0x81, 0x01, 0x37, 0x03, 0x00, 0x41, 0xF8, 0xFF, 0x03, 0x29, 0x03, 0x00, 0x10, 0x01, 0x41, 0xE3, 0x00, 0x10, 0x00, 0x0B];
    let bt2: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x15, 0x01, 0x13, 0x00, 0x02, 0x40, 0x41, 0x00, 0x0E, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x0B, 0x0B];
    let bt3: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x12, 0x01, 0x10, 0x00, 0x02, 0x40, 0x41, 0x00, 0x0E, 0x98, 0xA8, 0xC0, 0xE0, 0x80, 0x81, 0x01, 0x00, 0x0B, 0x0B];
    // Control frames (claude/nia-ctrl; scripts/lib/ctrl-wasm.py makes the same bytes): a branch back to a
    // `loop (result i32)` carries no value (lr1 TRAPped, lr2 copied a value from under the loop), a typed loop
    // falling through keeps its result (lr3), and `else` only in an `if` (fuzz finding 2: el1 is the
    // fuzzer's input, which froze the kernel; el2 `loop else`, el3 `block else`).
    let lr1: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x19, 0x01, 0x17, 0x01, 0x01, 0x7F, 0x03, 0x7F, 0x20, 0x00, 0x41, 0x01, 0x6A, 0x22, 0x00, 0x41, 0x03, 0x49, 0x0D, 0x00, 0x41, 0x07, 0x0B, 0x10, 0x00, 0x0B];
    let lr2: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x1D, 0x01, 0x1B, 0x01, 0x01, 0x7F, 0x41, 0x05, 0x03, 0x7F, 0x20, 0x00, 0x41, 0x01, 0x6A, 0x22, 0x00, 0x41, 0x03, 0x49, 0x0D, 0x00, 0x41, 0x07, 0x0B, 0x10, 0x00, 0x10, 0x00, 0x0B];
    let lr3: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x0B, 0x01, 0x09, 0x00, 0x03, 0x7F, 0x41, 0x2A, 0x0B, 0x10, 0x00, 0x0B];
    let el1: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0C, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x04, 0x70, 0x75, 0x74, 0x63, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x29, 0x01, 0x27, 0x01, 0x02, 0x7F, 0x41, 0x00, 0x21, 0x00, 0x02, 0x40, 0x03, 0x32, 0x20, 0x00, 0x2D, 0x00, 0x00, 0x21, 0x01, 0x20, 0x01, 0x05, 0x0D, 0x01, 0x20, 0x01, 0x10, 0x00, 0x20, 0x00, 0x41, 0x01, 0x6A, 0x21, 0x00, 0x0C, 0x00, 0x0B, 0x0B, 0x0B, 0x0B, 0x16, 0x02, 0x0C, 0x01, 0x2A, 0x41, 0x00, 0x0B, 0x10, 0x48, 0x45, 0x4C, 0x4C, 0x4F, 0x20, 0x46, 0x52, 0x4F, 0x4D, 0x20, 0x57, 0x41, 0x53, 0x4D, 0x00];
    let el2: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x08, 0x01, 0x06, 0x00, 0x03, 0x40, 0x05, 0x0B, 0x0B];
    let el3: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60, 0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x02, 0x01, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01, 0x0A, 0x08, 0x01, 0x06, 0x00, 0x02, 0x40, 0x05, 0x0B, 0x0B];
    let cases: [(&str, &[u8]); 22] = [
        ("empty", empty),
        ("magic-only (no run)", magic_only),
        ("truncated section", trunc_section),
        ("oversized export name", bad_name),
        ("code missing end", no_end),
        ("overflow: section size", ov_af1),
        ("overflow: import module-name length", ov_af2),
        ("overflow: import field-name length", ov_af3),
        ("overflow: type parameter count", ov_ov1),
        ("overflow: type result count", ov_ov2),
        ("overflow: export name length", ov_ov3),
        ("overflow: code body size", ov_ov4),
        ("local groups past the body's end", ov_ov5),
        ("br_table: count about 2^42", bt1),
        ("br_table: 10-byte count in a block", bt2),
        ("br_table: count about 2^42 in a block", bt3),
        ("loop result: branch back, empty stack", lr1),
        ("loop result: a value under the loop", lr2),
        ("loop result: falls through its end", lr3),
        ("else in a loop (fuzzer input)", el1),
        ("else in a loop", el2),
        ("else in a block", el3),
    ];
    crate::kprintln!("[fuzz] feeding {} malformed modules to the WASM parser...", cases.len());
    for (name, bytes) in cases.iter() {
        crate::kprintln!("[fuzz] case '{}' ({} bytes):", name, bytes.len());
        run_app(name, bytes, CAP_ALL); // must print an error, never panic
    }
    guard_cases();
    crate::kprintln!("[fuzz] survived: every malformed module was rejected, no kernel panic");
}

// --- Nia guard test modules (claude/nia-guard; made by a script, checked with the
// nia-apps emulator). Each one used to crash the kernel or overrun its stack. ---
// A function section that declares 129 functions, one past MAX_FUNCS (17 until claude/nia-a1a3): refused at
// the count, before any entry is read. The whole 129-function app is wasm-textrun's f129.
static WASM_BAD_129FUNCS: [u8; 18] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x81, 0x01,
];
static WASM_BAD_NOCODE: [u8; 27] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00,
];
static WASM_BAD_BROUT: [u8; 38] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x09, 0x01, 0x07, 0x00,
    0x02, 0x40, 0x0C, 0x05, 0x0B, 0x0B,
];
static WASM_BAD_DEEP: [u8; 132] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x67, 0x01, 0x65, 0x00,
    0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40,
    0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40,
    0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40,
    0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40, 0x02, 0x40,
    0x02, 0x40, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B,
    0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B, 0x0B,
    0x0B, 0x0B, 0x0B, 0x0B,
];
static WASM_BAD_EMPTYADD: [u8; 34] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x05, 0x01, 0x03, 0x00,
    0x6A, 0x0B,
];
static WASM_DEPTH16: [u8; 78] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74,
    0x00, 0x00, 0x03, 0x03, 0x02, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    0x0A, 0x1C, 0x02, 0x13, 0x00, 0x20, 0x00, 0x04, 0x40, 0x20, 0x00, 0x41, 0x01, 0x6B, 0x10, 0x01,
    0x05, 0x41, 0x10, 0x10, 0x00, 0x0B, 0x0B, 0x06, 0x00, 0x41, 0x0E, 0x10, 0x01, 0x0B,
];
static WASM_BAD_RECURSE: [u8; 79] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69, 0x6E, 0x74,
    0x00, 0x00, 0x03, 0x03, 0x02, 0x00, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x02,
    0x0A, 0x1D, 0x02, 0x13, 0x00, 0x20, 0x00, 0x04, 0x40, 0x20, 0x00, 0x41, 0x01, 0x6B, 0x10, 0x01,
    0x05, 0x41, 0x10, 0x10, 0x00, 0x0B, 0x0B, 0x07, 0x00, 0x41, 0xE8, 0x07, 0x10, 0x01, 0x0B,
];

// Where each call level's `Stack` sits on the kernel stack (index = depth), so
// `wasmfuzz` can report the frame per level. Written by `invoke_wasm`.
static mut NIA_SP_AT: [usize; MAX_CALL_DEPTH + 1] = [0; MAX_CALL_DEPTH + 1];

fn note_sp(depth: usize, addr: usize) {
    if depth <= MAX_CALL_DEPTH {
        // SAFETY: single-threaded interpreter; plain store into a static array.
        unsafe { (*addr_of_mut!(NIA_SP_AT))[depth] = addr };
    }
}

/// (bottom, top) of the stack the shell runs on, for the stack report.
#[cfg(target_arch = "aarch64")]
fn shell_stack() -> (usize, usize) {
    let e = crate::stackwater::boot();
    (e.bottom, e.top)
}
#[cfg(target_arch = "x86_64")]
fn shell_stack() -> (usize, usize) {
    let (b, t) = crate::stackinfo::kstack();
    (b as usize, t as usize)
}

fn stack_report() {
    #[cfg(target_arch = "aarch64")]
    crate::stackwater::report();
    #[cfg(target_arch = "x86_64")]
    crate::stackinfo::report();
}

/// 257 `i32.const 1` in a row: one more value than Nia's stack holds. Built here
/// rather than spelled out as 549 bytes.
fn flood_module(buf: &mut [u8; 600]) -> usize {
    const HEAD: [u8; 27] = [
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // magic, version
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type 0: () -> ()
        0x03, 0x02, 0x01, 0x00, // one function, type 0
        0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, // export "run" = func 0
    ];
    // body = 0 locals, 257 x (i32.const 1), end = 516 bytes (uleb 0x84 0x04);
    // section = count 1 + body size (2) + body = 519 bytes (uleb 0x87 0x04).
    const CODE_HEAD: [u8; 6] = [0x0A, 0x87, 0x04, 0x01, 0x84, 0x04];
    let mut n = 0;
    for b in HEAD.iter().chain(CODE_HEAD.iter()) {
        buf[n] = *b;
        n += 1;
    }
    buf[n] = 0x00;
    n += 1;
    for _ in 0..STACK_CAP + 1 {
        buf[n] = 0x41;
        buf[n + 1] = 0x01;
        n += 2;
    }
    buf[n] = 0x0B;
    n + 1
}

/// The Nia guard cases (part of `wasmfuzz`): every bad module must end in a
/// "[nia] parse error" or "[nia] TRAP" line, and a good module run after a TRAP
/// must still run normally.
fn guard_cases() {
    crate::kprintln!("[fuzz] nia guard: bad apps that used to crash the kernel");
    let mut flood = [0u8; 600];
    let fl = flood_module(&mut flood);
    let bad: [(&str, &[u8]); 6] = [
        ("guard: 129 functions", &WASM_BAD_129FUNCS),
        ("guard: code count differs", &WASM_BAD_NOCODE),
        ("guard: branch out of the function", &WASM_BAD_BROUT),
        ("guard: 33 nested blocks", &WASM_BAD_DEEP),
        ("guard: 257 values", &flood[..fl]),
        ("guard: add on an empty stack, last instruction", &WASM_BAD_EMPTYADD),
    ];
    for (name, bytes) in bad.iter() {
        run_app(name, bytes, CAP_ALL);
    }

    // Depth: 16 functions deep, print at the deepest level, measured.
    // SAFETY: single-threaded; reset the per-level record.
    unsafe { *addr_of_mut!(NIA_SP_AT) = [0; MAX_CALL_DEPTH + 1] };
    crate::kprintln!("[fuzz] stack before the depth-16 app:");
    stack_report();
    run_app("guard: 16 functions deep, print at the bottom", &WASM_DEPTH16, CAP_ALL);
    crate::kprintln!("[fuzz] stack after the depth-16 app:");
    stack_report();
    // SAFETY: read the record written by invoke_wasm.
    let at = unsafe { *addr_of_mut!(NIA_SP_AT) };
    let (bottom, top) = shell_stack();
    let (first, deepest) = (at[1], at[MAX_CALL_DEPTH]);
    if first != 0 && deepest != 0 && first > deepest {
        crate::kprintln!(
            "[fuzz] nia stack: {} bytes per call level, depth {} at {} KiB below the top, {} KiB left above the stack bottom",
            (first - deepest) / (MAX_CALL_DEPTH - 1),
            MAX_CALL_DEPTH,
            top.saturating_sub(deepest) / 1024,
            deepest.saturating_sub(bottom) / 1024
        );
    } else {
        crate::kprintln!("[fuzz] nia stack: depth {} was not reached", MAX_CALL_DEPTH);
    }

    // Unbounded recursion must TRAP, and the next app must run normally.
    run_app("guard: endless recursion", &WASM_BAD_RECURSE, CAP_ALL);
    run_app("guard: 16 deep again, after the TRAP", &WASM_DEPTH16, CAP_ALL);
    stop_key_cases();
}

// --- N1 / N2 test modules (claude/nia-n1; made by a script, checked with the
// nia-apps emulator). ---
static WASM_KEY_LOOP: [u8; 38] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02,
    0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x00, 0x0A, 0x09, 0x01, 0x07, 0x00,
    0x03, 0x40, 0x0C, 0x00, 0x0B, 0x0B,
];
static WASM_KEY_IGNORE: [u8; 62] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x60, 0x00, 0x00, 0x60, 0x00,
    0x01, 0x7F, 0x02, 0x0F, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x06, 0x67, 0x65, 0x74, 0x6B, 0x65,
    0x79, 0x00, 0x01, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00, 0x01,
    0x0A, 0x0C, 0x01, 0x0A, 0x00, 0x03, 0x40, 0x10, 0x00, 0x1A, 0x0C, 0x00, 0x0B, 0x0B,
];
static WASM_KEY_KEEP: [u8; 95] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0C, 0x03, 0x60, 0x00, 0x00, 0x60, 0x00,
    0x01, 0x7F, 0x60, 0x01, 0x7F, 0x00, 0x02, 0x1C, 0x02, 0x04, 0x61, 0x66, 0x72, 0x69, 0x06, 0x67,
    0x65, 0x74, 0x6B, 0x65, 0x79, 0x00, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70, 0x72, 0x69,
    0x6E, 0x74, 0x00, 0x02, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6E, 0x00,
    0x02, 0x0A, 0x1C, 0x01, 0x1A, 0x01, 0x01, 0x7F, 0x41, 0x80, 0xE0, 0x00, 0x21, 0x00, 0x03, 0x40,
    0x20, 0x00, 0x41, 0x01, 0x6B, 0x22, 0x00, 0x0D, 0x00, 0x0B, 0x10, 0x00, 0x10, 0x01, 0x0B,
];
static WASM_RET_BR: [u8; 69] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0C, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x60, 0x00, 0x01, 0x7F, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70,
    0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x03, 0x02, 0x02, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72,
    0x75, 0x6E, 0x00, 0x02, 0x0A, 0x0F, 0x02, 0x06, 0x00, 0x41, 0x07, 0x0C, 0x00, 0x0B, 0x06, 0x00,
    0x10, 0x01, 0x10, 0x00, 0x0B,
];
static WASM_RET_BRTABLE: [u8; 76] = [
    0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0C, 0x03, 0x60, 0x01, 0x7F, 0x00, 0x60,
    0x00, 0x00, 0x60, 0x00, 0x01, 0x7F, 0x02, 0x0E, 0x01, 0x04, 0x61, 0x66, 0x72, 0x69, 0x05, 0x70,
    0x72, 0x69, 0x6E, 0x74, 0x00, 0x00, 0x03, 0x03, 0x02, 0x02, 0x01, 0x07, 0x07, 0x01, 0x03, 0x72,
    0x75, 0x6E, 0x00, 0x02, 0x0A, 0x16, 0x02, 0x0D, 0x00, 0x02, 0x7F, 0x41, 0x07, 0x41, 0x05, 0x0E,
    0x01, 0x00, 0x01, 0x0B, 0x0B, 0x06, 0x00, 0x10, 0x01, 0x10, 0x00, 0x0B,
];

/// One key case: print the marker the suite's driver waits for (it sends the
/// listed bytes after `driver_wait_ms`), give the bytes `settle_ms` to land in
/// the console's receive ring, then run the app.
fn key_case(label: &str, bytes: &[u8], keys: &str, driver_wait_ms: u64, settle_ms: u64) {
    crate::kprintln!("[fuzz] keys (wait {} ms): {}", driver_wait_ms, keys);
    if settle_ms > 0 {
        crate::timer::busywait_ms(settle_ms);
    }
    run_app(label, bytes, CAP_ALL);
}

/// The N1 stop-key and N2 return-by-branch cases (part of `wasmfuzz`).
fn stop_key_cases() {
    crate::kprintln!("[fuzz] nia N2: a branch to the function's own label returns");
    run_app("n2: br 0 out of a () -> i32 function", &WASM_RET_BR, CAP_ALL);
    run_app("n2: br_table default = the function's label", &WASM_RET_BRTABLE, CAP_ALL);

    crate::kprintln!("[fuzz] nia N1: Ctrl+C stops an app that never ends");
    // Ctrl+C arrives while the loop runs (the driver waits 1 s first).
    key_case("n1: endless loop, Ctrl+C", &WASM_KEY_LOOP, "03", 1000, 0);
    // An app that reads and ignores every key: stopped by Ctrl+C through getkey.
    key_case("n1: app ignores every key, Ctrl+C", &WASM_KEY_IGNORE, "61 03", 0, 1500);
    // Keys before Ctrl+C go to the queue; the stop still happens.
    key_case("n1: keys then Ctrl+C, bare loop", &WASM_KEY_LOOP, "78 79 03", 0, 1500);
    // A key read by the loop poll reaches getkey afterwards (no key lost); the
    // 'x' 'y' left in the queue by the case above must not reach this app.
    key_case("n1: key during a loop reaches getkey", &WASM_KEY_KEEP, "6B", 0, 1500);
    // SAFETY: single-threaded; read the counter the loop poll keeps.
    let q = unsafe { KEYS_QUEUED };
    crate::kprintln!("[fuzz] keys the loop poll queued for getkey: {}", q);
    run_app("n1: a good app after the stops", &WASM_DEPTH16, CAP_ALL);
}

/// Roll a WASM dice app `n` times — each run draws from the capability-gated RNG
/// and prints a value in 1..=6. Shows a real user app built on CAP_RANDOM.
pub fn dice_demo(n: usize) {
    crate::kprintln!("[nia] dice.wasm - rolling a 1..6 die {} time(s) (CAP_RANDOM)", n);
    for _ in 0..n.clamp(1, 10) {
        run_app("dice.wasm", WASM_DICE, CAP_ALL);
    }
}

/// The bundled apps: one table for seeding the VFS, recording the integrity manifest (from these
/// compiled-in bytes) and restoring them after a disk load, so the lists cannot drift apart. appget refuses
/// these names. Since A7 a bundled name always RUNS from these bytes (run_file), never from a VFS file, and an
/// app over 16 KiB (the VFS's biggest file) is never written into the VFS: 16 KiB for disk apps, larger for
/// bundled apps.
pub const BUNDLED: [(&str, fn() -> &'static [u8]); 13] = [
    ("hello.wasm", hello_wasm),
    ("sum.wasm", sum_wasm),
    ("draw.wasm", draw_wasm),
    ("fileapp.wasm", file_wasm),
    ("clock.wasm", clock_wasm),
    ("type.wasm", type_wasm),
    ("ai.wasm", ai_wasm),
    ("chat.wasm", chat_wasm),
    ("notify.wasm", notify_wasm),
    ("notify2.wasm", notify2_wasm),
    ("net.wasm", net_wasm),
    ("rand.wasm", rand_wasm),
    ("andika.wasm", andika_wasm),
];

/// A bundled app's compiled-in bytes (A7): what `nia NAME` runs for a bundled name.
pub fn bundled(name: &str) -> Option<&'static [u8]> {
    BUNDLED.iter().find(|(n, _)| *n == name).map(|(_, bytes)| bytes())
}

/// Whether a bundled app also gets a VFS copy (A7): only one the VFS can hold. A bigger one exists only in the
/// kernel image; `ls` and the store list it as built in.
fn bundled_in_vfs(bytes: &[u8]) -> bool {
    bytes.len() <= crate::vfs::MAX_BIG_FILE_BYTES
}

/// At boot, before any disk load: write each bundled app the VFS can hold as a file (the one table the manifest
/// also uses).
pub fn seed_bundled() {
    for (name, bytes) in BUNDLED {
        let b = bytes();
        if bundled_in_vfs(b) {
            crate::vfs::write_full(name, b);
        }
    }
}

/// Bundled apps the boot check refused (bit i is BUNDLED[i]): never listed, never run.
static BUNDLED_REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn bundled_index(name: &str) -> Option<usize> {
    BUNDLED.iter().position(|(n, _)| *n == name)
}

fn bundled_refused(i: usize) -> bool {
    BUNDLED_REFUSED.load(Ordering::Relaxed) & (1 << i) != 0
}

/// A bundled app that passed the boot check (A7): the desk's Circle draws a tile only for these (afriui.md section 32).
pub fn bundled_ok(name: &str) -> bool {
    bundled_index(name).is_some_and(|i| !bundled_refused(i))
}

/// A bundled app that exists only in the kernel image (over 16 KiB) and passed the boot check: what `ls` and the
/// store list as built in.
pub fn built_in_only(name: &str) -> Option<&'static [u8]> {
    let i = bundled_index(name)?;
    let b = BUNDLED[i].1();
    if bundled_in_vfs(b) || bundled_refused(i) {
        None
    } else {
        Some(b)
    }
}

/// Whether `name` is a bundled app the VFS cannot hold (a VFS file of that name is the person's, never run).
pub fn is_big_bundled(name: &str) -> bool {
    bundled(name).map_or(false, |b| !bundled_in_vfs(b))
}

/// After the manifest is recorded (A7 condition 3, the architect's changes 2 and 3): every bundled app's bytes
/// must match it, and one over 16 KiB must carry its `afri.permissions` section, so what it gets is declared, not
/// CAP_ALL by default. An app that fails is named, refused (never listed, never run) and counted.
pub fn check_bundled() {
    let mut ok = 0;
    for (i, (name, bytes)) in BUNDLED.iter().enumerate() {
        let b = bytes();
        let fault = if !matches!(crate::integrity::check(name, b), crate::integrity::Verdict::Ok) {
            Some("MISMATCH with the integrity manifest")
        } else if !bundled_in_vfs(b) && !matches!(check_module(b), Ok((_, Some(_)))) {
            Some("over 16 KiB with no afri.permissions section")
        } else {
            None
        };
        match fault {
            None => ok += 1,
            Some(why) => {
                BUNDLED_REFUSED.fetch_or(1 << i, Ordering::Relaxed);
                crate::kprintln!("[nia] bundled app {}: {}; refused (not listed, not run)", name, why);
            }
        }
    }
    crate::kprintln!("[nia] bundled apps: {} of {} match the manifest", ok, BUNDLED.len());
}

/// After `vfs::load_from_disk`: a bundled name whose disk copy differs from the built-in bytes gets the
/// built-in copy back (an old disk after a kernel upgrade, or a tampered one), with one line; a missing one is
/// seeded again. A file with the name of a bundled app the VFS cannot hold is the person's own file: it is kept
/// (never deleted at boot) and never run, and `ls` says so (the architect's A7 change 1).
pub fn restore_bundled() {
    for (name, bytes) in BUNDLED {
        let want = bytes();
        if !bundled_in_vfs(want) {
            continue;
        }
        match crate::vfs::read_full(name) {
            Some(have) if have == want => {}
            Some(_) => {
                crate::vfs::write_full(name, want);
                crate::kprintln!("[vfs] {}: the disk copy differed from the bundled app; bundled copy restored", name);
            }
            None => {
                if !crate::vfs::write_full(name, want) {
                    crate::kprintln!("[vfs] {}: no free file slot to restore the bundled app", name);
                }
            }
        }
    }
}

/// Load a `.wasm` app from the filesystem and run it (granted all host caps).
pub fn run_file(name: &str) {
    // No re-entry (afriui.md section 25): checked before the file is read, the integrity line printed or the
    // window server reset, so the running app keeps everything it has.
    if refuse_nested(name) {
        return;
    }
    // A7: a bundled name runs the kernel's own bytes, in place. They are in the kernel image, so nothing the app
    // writes can change them (F1's reason for the copy does not apply) and they may be bigger than NIA_CODE. A VFS
    // file of that name is never read for running, so a disk or appget file cannot stand in for a system app.
    if let Some(bytes) = bundled(name) {
        if bundled_index(name).map_or(false, bundled_refused) {
            crate::kprintln!("[nia] REFUSED '{}': the bundled app failed its boot check", name);
            return;
        }
        crate::kprintln!("[nia] '{}' is built in: running the kernel's own copy", name);
        return run_verified(name, bytes, "the kernel image");
    }
    // The whole file (up to 16 KiB, claude/vfs-big), not the 512-byte view, copied into Nia's own buffer
    // (F1, claude/nia-fast): the hash, the parse and the run all see these bytes, and nothing the app writes to
    // the VFS (its own file included) can change them under it. After refuse_nested, so a refused nested run
    // never touches the running app's copy.
    let file = crate::vfs::read_full(name);
    if let Some(b) = file {
        if b.len() > CODE_CAP {
            // Never cut an app short (apps developer's review): today read_full gives at most 16 KiB.
            crate::kprintln!("[nia] REFUSED '{}': bigger than Nia's 16 KiB code limit", name);
            return;
        }
    }
    match file.map(copy_code) {
        Some(bytes) if bytes.len() >= 8 => run_verified(name, bytes, "the filesystem"),
        Some(_) => crate::kprintln!("[nia] '{}' is too small to be a WASM module", name),
        None => crate::kprintln!("[nia] no such file: {}  (try 'ls')", name),
    }
}

/// The integrity gate and the run, for an app's bytes from `from` (a bundled app's or Nia's copy of a file).
fn run_verified(name: &str, bytes: &[u8], from: &str) {
    // Integrity gate: verify the app's bytes against the manifest BEFORE
    // running it. Tamper/corruption defense at load, layered under the
    // capability sandbox. (Authorship, who signed it, is Ed25519, future.)
    // One verdict decides both the caps and the cell window's mark: only Verdict::Ok is signed.
    // Manifest apps keep every capability; anything else (a sideloaded app) runs without
    // file, net, AI and voice (claude/appget, change B).
    let (caps, signed) = match crate::integrity::check(name, bytes) {
        crate::integrity::Verdict::Tampered => {
            crate::kprintln!(
                "[nia] REFUSED '{}': integrity check FAILED (hash != manifest)",
                name
            );
            return;
        }
        crate::integrity::Verdict::Ok => {
            crate::kprintln!("[nia] integrity OK for '{}' (matches manifest)", name);
            (CAP_ALL, true)
        }
        crate::integrity::Verdict::Unverified => {
            crate::kprintln!(
                "[nia] '{}' unverified (not in manifest); sandboxed run allowed",
                name
            );
            crate::kprintln!(
                "[nia] '{}' runs without file, net, AI and voice (not in the manifest)",
                name
            );
            (CAP_SIDELOAD, false)
        }
    };
    crate::kprintln!("[nia] loaded '{}' from {} ({} bytes)", name, from, bytes.len());
    crate::tonga::wm_reset(); // fresh window server for a graphical app
    run_app_in(name, bytes, caps, true, signed); // the cell window's title marks an unsigned app
}

// F1 (claude/nia-fast): the code of the app run_file runs. 16 KiB of .bss, the VFS's biggest file. Written only
// by copy_code, which only run_file calls, after its nested-run refusal; one app runs at a time.
const CODE_CAP: usize = 16384;
static mut NIA_CODE: [u8; CODE_CAP] = [0; CODE_CAP];

/// Copy `src` into NIA_CODE and give back the copy (run_file has refused anything over CODE_CAP).
fn copy_code(src: &[u8]) -> &'static [u8] {
    let n = src.len().min(CODE_CAP);
    // SAFETY: single-threaded; no Module borrows NIA_CODE here, because nested runs are refused before this.
    unsafe {
        (&mut *addr_of_mut!(NIA_CODE))[..n].copy_from_slice(&src[..n]);
        &(&*core::ptr::addr_of!(NIA_CODE))[..n]
    }
}

/// The capability one host function needs, and its name (the same table `host_call` enforces).
fn host_cap(host: u8) -> (u32, &'static str) {
    match host {
        HOST_PRINT => (CAP_PRINT, "print"),
        HOST_PRINTQ => (CAP_PRINT, "print64"),
        HOST_PUTC => (CAP_PUTC, "putc"),
        HOST_WIN_CREATE | HOST_WIN_FILL | HOST_WIN_PRESENT | HOST_WIN_TEXT | HOST_WIN_TEXT_RUN => (CAP_DRAW, "draw"),
        HOST_CELL_OPEN | HOST_CELL_PUT | HOST_CELL_CURSOR | HOST_CELL_SIZE | HOST_CELL_EVENT => (CAP_DRAW, "draw"),
        HOST_FILE_WRITE | HOST_FILE_READ => (CAP_FILE, "file"),
        HOST_TIME | HOST_SLEEP => (CAP_TIME, "time"),
        HOST_GETKEY | HOST_POLLKEY => (CAP_INPUT, "input"),
        HOST_AI => (CAP_AI, "AI"),
        HOST_SPEAK => (CAP_VOICE, "voice"),
        HOST_NET => (CAP_NET, "net"),
        HOST_RANDOM => (CAP_RANDOM, "random"),
        _ => (0, "unknown"),
    }
}

/// Print `label` and the capability names in `mask` on one line (`none` when the mask is empty).
pub fn print_caps(label: &str, mask: u32, none: &str) {
    if mask == 0 {
        crate::kprintln!("{}{}", label, none.trim_start());
        return;
    }
    let names = [
        (CAP_PRINT, "print"), (CAP_PUTC, "putc"), (CAP_DRAW, "draw"), (CAP_FILE, "file"), (CAP_TIME, "time"),
        (CAP_INPUT, "input"), (CAP_AI, "AI"), (CAP_VOICE, "voice"), (CAP_NET, "net"), (CAP_RANDOM, "random"),
    ];
    let mut line = [0u8; 96];
    let mut n = 0;
    for (bit, nm) in names {
        if mask & bit != 0 && n + nm.len() + 2 <= line.len() {
            if n > 0 {
                line[n..n + 2].copy_from_slice(b", ");
                n += 2;
            }
            line[n..n + nm.len()].copy_from_slice(nm.as_bytes());
            n += nm.len();
        }
    }
    crate::kprintln!("{}{}", label, core::str::from_utf8(&line[..n]).unwrap_or("?"));
}

/// Parse a module exactly as `run_file` would and NEVER run it (appget). All capabilities are allowed for the
/// parse only, so imports are listed, not refused. Returns the capabilities its imports need, and what its
/// `afri.permissions` section asks for (None when it has no section). A section refused here is refused at
/// launch too: one parser, one rule.
pub fn check_module(bytes: &[u8]) -> Result<(u32, Option<u32>), &'static str> {
    if bytes.len() < 8 {
        return Err("too small to be a WASM module");
    }
    let m = parse(bytes, CAP_ALL)?;
    let mut need = 0u32;
    for i in 0..m.import_count.min(MAX_IMPORTS) {
        need |= host_cap(m.import_host[i]).0;
    }
    Ok((need, if m.listed { Some(m.asked) } else { None }))
}

fn run_app(label: &str, bytes: &[u8], caps: u32) {
    run_app_in(label, bytes, caps, false, true); // the built-in demos are the kernel's own bytes: signed
}

/// `from_file`: the app came from the VFS (`run_file`), so an app with no `afri.permissions` section gets a
/// line saying it runs on the v0 grant. Built-in demos (`run_app`) print nothing new.
/// `signed`: the app's bytes are the kernel's own or match the manifest (integrity Verdict::Ok). Set here, after the
/// re-entry check, so a refused nested start never changes the running app's mark; back to false after the run, so
/// a run path that forgets it shows "unsigned" (fail closed, the architect's cell stage 2 follow-up).
fn run_app_in(label: &str, bytes: &[u8], caps: u32, from_file: bool, signed: bool) {
    if refuse_nested(label) {
        return;
    }
    APP_RUNNING.store(true, Ordering::Relaxed);
    set_app_name(label);
    set_app_signed(signed);
    run_app_body(label, bytes, caps, from_file);
    set_app_signed(false);
    APP_RUNNING.store(false, Ordering::Relaxed);
}

fn run_app_body(label: &str, bytes: &[u8], caps: u32, from_file: bool) {
    crate::kprintln!();
    crate::kprintln!("[nia] === {} ===", label);
    crate::kprintln!("[nia] loading module ({} bytes), granted caps = {:#04x}", bytes.len(), caps);
    putc_reset();
    keyq_reset(); // N1: one app's keys never reach the next
    match parse(bytes, caps) {
        Ok(m) => {
            if m.listed {
                print_caps("[nia] permissions asked: ", m.asked, "none");
                print_caps("[nia] permissions granted: ", m.caps, "none");
                print_caps("[nia] permissions withheld: ", m.asked & !m.caps, "nothing");
            } else if from_file {
                crate::kprintln!("[nia] permissions: none listed; granted by kind (v0)");
            }
            crate::kprintln!("[nia] executing 'run' in the sandbox...");
            let mut st = Stack::new();
            let r = m.invoke(m.run_index, &mut st);
            crate::desk::cell_app_ended(); // its cell window closes, on the clean return and on the TRAP alike
            finish_putc(); // the app's last putc line ends before the run's own line, whichever way it ended
            match r {
                Ok(()) => crate::kprintln!("[nia] 'run' returned cleanly"),
                Err(e) => crate::kprintln!("[nia] TRAP: {}", e),
            }
        }
        Err(e) => crate::kprintln!("[nia] parse error: {}", e),
    }
}

fn parse(m: &[u8], caps: u32) -> Result<Module<'_>, &'static str> {
    if m.len() < 8 || &m[0..4] != b"\0asm" || &m[4..8] != &[1, 0, 0, 0] {
        return Err("not a WASM v1 module");
    }
    // Clear linear memory + module globals for this instance.
    // SAFETY: single-threaded.
    unsafe {
        for b in (*addr_of_mut!(WASM_MEM)).iter_mut() {
            *b = 0;
        }
        for g in (*addr_of_mut!(WASM_GLOBALS)).iter_mut() {
            *g = 0;
        }
    }
    memo_reset(); // no block-end memo from an earlier app (claude/nia-fast)
    let mut md = Module {
        bytes: m,
        types: [(0, 0); MAX_TYPES],
        func_type: [0; MAX_FUNCS],
        code_off: [0; MAX_FUNCS],
        code_len: [0; MAX_FUNCS],
        import_host: [HOST_UNKNOWN; MAX_IMPORTS],
        import_type: [0; MAX_IMPORTS],
        nfuncs: 0,
        ntypes: 0,
        import_count: 0,
        run_index: usize::MAX,
        caps,
        listed: false,
        asked: 0,
        depth: Cell::new(0),
    };
    // Code bodies seen (section 10). Must equal the function count (section 3).
    let mut ncode = 0usize;
    let mut c = 8;
    while c < m.len() {
        let id = m[c];
        c += 1;
        let (size, a) = uleb(m, c);
        c += a;
        // Reject a section that claims to extend past the module (truncated /
        // hostile input): every section must fit within the module bytes. The add is
        // checked: a 10-byte LEB size near usize::MAX overflowed `c + size` and
        // panicked the kernel (claude/nia-afri, architect's A1).
        let end = match c.checked_add(size) {
            Some(e) if e <= m.len() => e,
            _ => return Err("truncated section"),
        };
        match id {
            0 => {
                // A custom section: its name, then its bytes. Only `afri.permissions` is read; any other
                // (`name`, `producers`, ...) is skipped as before.
                let sname = name_in(m, &mut c, end).ok_or("truncated custom section name")?;
                if sname == b"afri.permissions" {
                    if md.listed {
                        return Err("two afri.permissions sections");
                    }
                    md.listed = true;
                    if c >= end {
                        return Err("bad afri.permissions section");
                    }
                    let (cnt, a) = uleb(m, c);
                    c += a;
                    if cnt > MAX_PERMS {
                        return Err("too many permissions (Nia holds 16)");
                    }
                    for _ in 0..cnt {
                        let pn = name_in(m, &mut c, end).ok_or("bad afri.permissions section")?;
                        match perm_bit(pn) {
                            Some(bit) => md.asked |= bit,
                            None => {
                                crate::kprint!("[nia] unknown permission ");
                                print_quoted(pn);
                                crate::kprintln!();
                                return Err("unknown permission");
                            }
                        }
                    }
                    if c != end {
                        return Err("bad afri.permissions section");
                    }
                }
            }
            1 => {
                let (cnt, a) = uleb(m, c);
                c += a;
                if cnt > MAX_TYPES {
                    return Err("too many types (Nia holds 32)");
                }
                for i in 0..cnt {
                    // 0x60, then the parameter and result type vectors (one byte each), read with
                    // name_in so the counts are checked adds within the section (step A2).
                    if c >= end {
                        return Err("truncated type");
                    }
                    c += 1;
                    let np = name_in(m, &mut c, end).ok_or("truncated type")?.len();
                    let nr = name_in(m, &mut c, end).ok_or("truncated type")?.len();
                    md.types[i] = (
                        u16::try_from(np).map_err(|_| "too many parameters")?,
                        u16::try_from(nr).map_err(|_| "too many results")?,
                    );
                }
                md.ntypes = cnt;
            }
            2 => {
                let (cnt, a) = uleb(m, c);
                c += a;
                for _ in 0..cnt {
                    let module = name_in(m, &mut c, end).ok_or("truncated import name")?;
                    let name = name_in(m, &mut c, end).ok_or("truncated import name")?;
                    // Imports come from module `afri` (`env` too, for v0 apps: the 23
                    // built-in apps use it). Exact bytes, no case folding or prefix.
                    if module != b"afri" && module != b"env" {
                        refuse_module(module);
                        return Err("import from a module Nia does not offer");
                    }
                    if c >= end {
                        return Err("truncated import name");
                    }
                    let kind = m[c];
                    c += 1;
                    let (desc, a) = uleb(m, c);
                    c += a;
                    if kind == 0 {
                        if md.import_count >= MAX_IMPORTS {
                            return Err("too many imported functions (Nia holds 16)");
                        }
                        if desc >= md.ntypes {
                            return Err("import has a bad type index");
                        }
                        {
                            md.import_host[md.import_count] = match name {
                                b"print" => HOST_PRINT,
                                b"putc" => HOST_PUTC,
                                b"win_create" => HOST_WIN_CREATE,
                                b"win_fill" => HOST_WIN_FILL,
                                b"win_present" => HOST_WIN_PRESENT,
                                b"file_write" => HOST_FILE_WRITE,
                                b"file_read" => HOST_FILE_READ,
                                b"time" => HOST_TIME,
                                b"getkey" => HOST_GETKEY,
                                b"win_text" => HOST_WIN_TEXT,
                                b"ai" => HOST_AI,
                                b"speak" => HOST_SPEAK,
                                b"net" => HOST_NET,
                                b"random" => HOST_RANDOM,
                                b"print64" => HOST_PRINTQ,
                                b"pollkey" => HOST_POLLKEY,
                                b"sleep_ms" => HOST_SLEEP,
                                b"cell_open" => HOST_CELL_OPEN,
                                b"cell_put" => HOST_CELL_PUT,
                                b"cell_cursor" => HOST_CELL_CURSOR,
                                b"cell_size" => HOST_CELL_SIZE,
                                b"cell_event" => HOST_CELL_EVENT,
                                b"win_text_run" => HOST_WIN_TEXT_RUN,
                                _ => HOST_UNKNOWN,
                            };
                            md.import_type[md.import_count] = desc; // type index
                        }
                        md.import_count += 1;
                    }
                }
            }
            3 => {
                let (cnt, a) = uleb(m, c);
                c += a;
                if cnt > MAX_FUNCS {
                    return Err("too many functions (Nia holds 128)");
                }
                for i in 0..cnt {
                    let (ti, a) = uleb(m, c);
                    c += a;
                    if ti >= md.ntypes {
                        return Err("function has a bad type index");
                    }
                    md.func_type[i] = u8::try_from(ti).map_err(|_| "function has a bad type index")?;
                }
                md.nfuncs = cnt;
            }
            7 => {
                let (cnt, a) = uleb(m, c);
                c += a;
                for _ in 0..cnt {
                    let name = name_in(m, &mut c, end).ok_or("truncated export name")?;
                    if c >= end {
                        return Err("truncated export name");
                    }
                    let kind = m[c];
                    c += 1;
                    let (idx, a) = uleb(m, c);
                    c += a;
                    if kind == 0 && name == b"run" {
                        md.run_index = idx;
                    }
                }
            }
            10 => {
                let (cnt, a) = uleb(m, c);
                c += a;
                if cnt > MAX_FUNCS {
                    return Err("too many functions (Nia holds 128)");
                }
                for i in 0..cnt {
                    // a body's size, then its bytes: within the section, a checked add (step A2)
                    let body = name_in(m, &mut c, end).ok_or("truncated code body")?;
                    md.code_off[i] = u32::try_from(c - body.len()).map_err(|_| "module too big")?;
                    md.code_len[i] = u32::try_from(body.len()).map_err(|_| "module too big")?;
                }
                ncode = cnt;
            }
            6 => {
                // global section: per global -> value type, mutability, then a
                // constant init expression (one const instr + `end`). We store the
                // initial value's raw bits; `c = end` resyncs the section after.
                let (cnt, a) = uleb(m, c);
                c += a;
                for i in 0..cnt {
                    if c + 2 > m.len() {
                        break;
                    }
                    c += 2; // skip valtype + mutability bytes
                    if c >= m.len() {
                        break;
                    }
                    let opc = m[c];
                    let val: i64 = match opc {
                        0x41 => {
                            let (v, a) = sleb(m, c + 1);
                            c += 1 + a;
                            v as i64
                        }
                        0x42 => {
                            let (v, a) = sleb64(m, c + 1);
                            c += 1 + a;
                            v
                        }
                        0x43 => {
                            if c + 5 > m.len() {
                                break;
                            }
                            let mut b = [0u8; 4];
                            b.copy_from_slice(&m[c + 1..c + 5]);
                            c += 5;
                            f32::from_le_bytes(b).to_bits() as i64
                        }
                        0x44 => {
                            if c + 9 > m.len() {
                                break;
                            }
                            let mut b = [0u8; 8];
                            b.copy_from_slice(&m[c + 1..c + 9]);
                            c += 9;
                            f64::from_le_bytes(b).to_bits() as i64
                        }
                        _ => {
                            c += 1;
                            0
                        }
                    };
                    if i < MAX_GLOBALS {
                        global_set(i, val);
                    }
                    if c < m.len() && m[c] == 0x0B {
                        c += 1; // skip the `end` opcode
                    }
                }
            }
            11 => {
                // data section: apply active segments to linear memory
                let (cnt, a) = uleb(m, c);
                c += a;
                for _ in 0..cnt {
                    let (flags, a) = uleb(m, c);
                    c += a;
                    let mut off = 0usize;
                    if flags == 0 {
                        // offset expr: i32.const N, end
                        if c < m.len() && m[c] == 0x41 {
                            let (v, a) = sleb(m, c + 1);
                            off = v as u32 as usize; // unsigned, as WebAssembly says
                            c += 1 + a;
                        }
                        c += 1; // 0x0B end
                    }
                    let (len, a) = uleb(m, c);
                    c += a;
                    if c.checked_add(len).map_or(true, |e| e > m.len()) {
                        return Err("truncated data segment");
                    }
                    if off.checked_add(len).map_or(true, |end| end > MEM_SIZE) {
                        return Err("data segment past the end of memory");
                    }
                    for i in 0..len {
                        mem_write8(off + i, m[c + i]);
                    }
                    c += len;
                }
            }
            _ => {}
        }
        c = end;
    }
    if ncode != md.nfuncs {
        return Err("code count differs from function count");
    }
    if md.run_index == usize::MAX {
        return Err("no exported 'run'");
    }
    // The grant, once, after every section (a custom section may come anywhere). The section can only narrow,
    // never widen: what an app gets is always within what its kind allows (`caps`).
    if md.listed {
        md.caps = caps & md.asked;
    }
    Ok(md)
}

impl<'a> Module<'a> {
    fn host_call(&self, fidx: usize, caller: &mut Stack) -> Result<(), &'static str> {
        let host = if fidx < MAX_IMPORTS { self.import_host[fidx] } else { HOST_UNKNOWN };
        // A host function that may print closes an open putc line first. The quiet ones (time, getkey, random,
        // pollkey, sleep_ms) print nothing, so a prompt written with putc stays on its line while the app waits for
        // a key. A refusal below prints a DENIED line, so it closes the line itself.
        if !matches!(host, HOST_PUTC | HOST_TIME | HOST_GETKEY | HOST_RANDOM | HOST_POLLKEY | HOST_SLEEP) {
            putc_break();
        }

        // print64(i64) takes a single 64-bit argument, so it must pop the full
        // i64 value (the generic arg loop below pops i32 cells). Handle it here.
        if host == HOST_PRINTQ {
            if (self.caps & CAP_PRINT) == 0 {
                crate::kprintln!("    [nia] DENIED: app called host 'print64' without the capability");
                return Err("capability denied");
            }
            let v = caller.pop_i64();
            caller.check()?;
            let mut buf = [0u8; 24];
            let n = fmt_i64(v, &mut buf);
            crate::kprintln!("    [wasm app] print64({})", core::str::from_utf8(&buf[..n]).unwrap_or("?"));
            return Ok(());
        }

        let ti = if fidx < MAX_IMPORTS { self.import_type[fidx] } else { 0 };
        if ti >= MAX_TYPES {
            return Err("bad type index");
        }
        let (np, nr) = (usize::from(self.types[ti].0), usize::from(self.types[ti].1));
        let mut a = [0i32; 8];
        for k in (0..np.min(8)).rev() {
            a[k] = caller.pop();
        }
        caller.check()?; // never run a host function on missing arguments

        let (req, name): (u32, &str) = match host {
            HOST_PRINT => (CAP_PRINT, "print"),
            HOST_PUTC => (CAP_PUTC, "putc"),
            HOST_WIN_CREATE => (CAP_DRAW, "win_create"),
            HOST_WIN_FILL => (CAP_DRAW, "win_fill"),
            HOST_WIN_PRESENT => (CAP_DRAW, "win_present"),
            HOST_FILE_WRITE => (CAP_FILE, "file_write"),
            HOST_FILE_READ => (CAP_FILE, "file_read"),
            HOST_TIME => (CAP_TIME, "time"),
            HOST_GETKEY => (CAP_INPUT, "getkey"),
            HOST_WIN_TEXT => (CAP_DRAW, "win_text"),
            HOST_AI => (CAP_AI, "ai"),
            HOST_SPEAK => (CAP_VOICE, "speak"),
            HOST_NET => (CAP_NET, "net"),
            HOST_RANDOM => (CAP_RANDOM, "random"),
            HOST_POLLKEY => (CAP_INPUT, "pollkey"),
            HOST_SLEEP => (CAP_TIME, "sleep_ms"),
            HOST_CELL_OPEN => (CAP_DRAW, "cell_open"),
            HOST_CELL_PUT => (CAP_DRAW, "cell_put"),
            HOST_CELL_CURSOR => (CAP_DRAW, "cell_cursor"),
            HOST_CELL_SIZE => (CAP_DRAW, "cell_size"),
            HOST_CELL_EVENT => (CAP_DRAW, "cell_event"),
            HOST_WIN_TEXT_RUN => (CAP_DRAW, "win_text_run"),
            _ => (0xFFFF_FFFF, "unknown"),
        };
        if host == HOST_UNKNOWN || (self.caps & req) == 0 {
            putc_break();
            crate::kprintln!("    [nia] DENIED: app called host '{}' without the capability", name);
            return Err("capability denied");
        }

        let mut result: Option<i32> = None;
        match host {
            HOST_PRINT => crate::kprintln!("    [wasm app] print({}) = {:#x}", a[0], a[0] as u32),
            HOST_PUTC => putc_byte(a[0] as u8),
            HOST_WIN_CREATE => {
                crate::framebuffer::set_console(false); // switch the screen to graphics
                result = Some(crate::tonga::win_create(NIA_OWNER, u(a[0]), u(a[1])) as i32);
            }
            HOST_WIN_FILL => crate::tonga::win_fill(u(a[0]), u(a[1]), u(a[2]), u(a[3]), u(a[4]), a[5] as u32),
            HOST_WIN_PRESENT => crate::tonga::win_present(),
            HOST_FILE_WRITE => {
                result = Some(host_file_write(a[0], a[1], a[2], a[3])?);
            }
            HOST_FILE_READ => {
                result = Some(host_file_read(a[0], a[1], a[2], a[3])?);
            }
            HOST_TIME => result = Some(wall_secs() as i32), // epoch seconds; wraps in 2038 as an i32
            HOST_GETKEY => {
                // The next key: first any the loop poll queued, else block on the
                // console. Ctrl+C stops the app (N1), so an app that ignores
                // every key can still be left. While the app owns a cell window its keys
                // are cell events, so getkey returns -2 (no window for it) at once
                // (afriui.md section 25).
                cell_forced_check()?;
                if crate::desk::cell_owned() {
                    result = Some(crate::desk::cell_abi::ERR_NO_WINDOW);
                } else {
                    let k = match keyq_pop() {
                        Some(c) => c,
                        None => loop {
                            cell_forced_check()?;
                            if let Some(c) = crate::uart::getc() {
                                break c;
                            }
                            core::hint::spin_loop();
                        },
                    };
                    if k == STOP_KEY {
                        return Err(STOPPED);
                    }
                    result = Some(k as i32);
                }
            }
            HOST_WIN_TEXT => {
                crate::tonga::win_text(u(a[0]), u(a[1]), u(a[2]), a[3] as u8, a[4] as u32);
            }
            HOST_WIN_TEXT_RUN => {
                // win_text_run(win, x, y, ptr, len, color): the length is capped at one row first, then the bytes
                // are checked (a run past the 64 KiB is a TRAP), then each glyph goes through win_text, which
                // clips to the window; x + 8 * i saturates, so a huge x stops at the edge (afriui.md section 21).
                let n = ulen(a[4], TEXT_RUN_MAX);
                let p = mem_range(a[3], n)?;
                let (id, x, y, color) = (u(a[0]), u(a[1]), u(a[2]), a[5] as u32);
                for i in 0..n {
                    crate::tonga::win_text(id, x.saturating_add(8 * i), y, mem_read8(p + i), color);
                }
            }
            HOST_AI => {
                // ai(prompt_ptr, prompt_len, out_ptr, out_cap) -> out_len.
                // On-device, capability-gated, private. Reads the prompt from the
                // app's memory and writes the reply back into it.
                result = Some(host_ai(a[0], a[1], a[2], a[3])?);
            }
            HOST_SPEAK => {
                // speak(text_ptr, text_len): Asili Voice. Real audio needs an
                // audio driver + an on-device TTS model (Piper-class, docs/14);
                // v0 emits the text as a placeholder for the spoken output.
                let mut tb = [0u8; 160];
                let tn = ulen(a[1], 160);
                let t = mem_range(a[0], tn)?;
                for i in 0..tn {
                    tb[i] = mem_read8(t + i);
                }
                crate::kprintln!(
                    "    [asili voice] (would speak aloud): \"{}\"",
                    core::str::from_utf8(&tb[..tn]).unwrap_or("?")
                );
                // 3.264 rewire: 3.256 made speak() AUDIBLE (the 3.254 finding) but
                // wired it to sound_play_tone(), which takes NO argument — so a Nia
                // app's speak() was TEXT-INDEPENDENT by construction (the 3.260
                // finding). Point it at the same text-dependent formant synthesizer
                // the EL0 speak syscall already calls (process.rs:897). Still guarded
                // on sound_present() so a guest booted WITHOUT -device
                // virtio-sound-device behaves exactly as before — every one of the
                // 686 green suites runs without an audiodev. This buys FORMANT
                // speech, NOT Piper; the neural model stays unclaimed (docs/14).
                if crate::virtio::sound_present() && !crate::voice::say(&tb[..tn]) {
                    // 3.343: report say()'s bool, which every caller has discarded
                    // since 2.24. SILENT on success, so the serial output of a
                    // passing boot is unchanged.
                    crate::kprintln!("[voice] say() returned FALSE (utterance did not complete)");
                }
            }
            HOST_NET => {
                // net(op) -> 1 on success / 0 on failure. Reached only with CAP_NET.
                // op 0 = ping the gateway (ICMP), op 1 = resolve a name (UDP/DNS).
                crate::kprintln!("    [wasm app] net({}) via CAP_NET (capability-gated)", a[0]);
                let ok = if a[0] == 0 {
                    crate::virtio::net_ping()
                } else {
                    crate::virtio::net_dns()
                };
                result = Some(if ok { 1 } else { 0 });
            }
            HOST_POLLKEY => {
                result = Some(host_pollkey()?);
            }
            HOST_SLEEP => {
                result = Some(host_sleep_ms(a[0])?);
            }
            HOST_CELL_OPEN => {
                // SAFETY: single-threaded interpreter; the name and verdict run_app_in stored for this run.
                let (name, signed) = unsafe { (&(&*addr_of_mut!(APP_NAME))[..APP_NAME_LEN], APP_SIGNED) };
                result = Some(crate::desk::app_cell_open(name, signed, a[0], a[1]));
            }
            HOST_CELL_PUT => {
                // len is in cells: negative is -5 before any cast, then capped at one row (120) before it is
                // doubled to bytes, and the bytes must lie in linear memory (a TRAP otherwise, as for every pointer).
                result = Some(if a[4] < 0 {
                    crate::desk::cell_abi::ERR_BAD_ARG
                } else {
                    let n = (a[4] as usize).min(CELL_PUT_MAX);
                    let p = mem_range(a[3], 2 * n)?;
                    // SAFETY: single-threaded interpreter; p + 2n is inside WASM_MEM (mem_range).
                    let cells = unsafe { &(&*core::ptr::addr_of!(WASM_MEM))[p..p + 2 * n] };
                    crate::desk::app_cell_put(a[0], a[1], a[2], cells)
                });
            }
            HOST_CELL_CURSOR => result = Some(crate::desk::app_cell_cursor(a[0], a[1], a[2], a[3])),
            HOST_CELL_SIZE => result = Some(crate::desk::app_cell_size(a[0])),
            HOST_CELL_EVENT => {
                let deep = self.depth.get() >= MAX_CALL_DEPTH && STACK_PROBE.load(Ordering::Relaxed);
                let mark = if deep { cell_stack_paint() } else { None };
                result = Some(crate::desk::app_cell_event(a[0]));
                if let Some(m) = mark {
                    cell_stack_report(m);
                }
            }
            HOST_RANDOM => {
                // random() -> i32. Reached only with CAP_RANDOM. A fresh draw from
                // the hardware-seeded PRNG.
                result = Some(crate::rng::next_u32() as i32);
            }
            _ => {}
        }
        if nr > 0 {
            caller.push(result.unwrap_or(0));
        }
        Ok(())
    }

    fn invoke(&self, fidx: usize, caller: &mut Stack) -> Result<(), &'static str> {
        if fidx < self.import_count {
            return self.host_call(fidx, caller);
        }
        // Nia guard: one exit point for the depth count, so a TRAP, an early
        // return and a normal return all bring it back down.
        let d = self.depth.get();
        if d >= MAX_CALL_DEPTH {
            return Err("call depth limit (16 functions deep)");
        }
        self.depth.set(d + 1);
        let r = self.invoke_wasm(fidx, caller, d + 1);
        self.depth.set(d);
        r
    }

    fn invoke_wasm(&self, fidx: usize, caller: &mut Stack, depth: usize) -> Result<(), &'static str> {
        let li = fidx - self.import_count;
        if li >= self.nfuncs {
            return Err("bad function index");
        }
        let ft = usize::from(self.func_type[li]);
        if ft >= MAX_TYPES {
            return Err("bad type index");
        }
        let (nparams, nresults) = (usize::from(self.types[ft].0), usize::from(self.types[ft].1));
        if nparams > LOCALS_CAP {
            return Err("too many params");
        }
        // Locals are 64-bit cells so i64 values survive in local.get/set/tee and
        // as function args. i32 locals are transparent: a pushed i32 is already
        // sign-extended in its cell, so moving it full-width preserves the low 32.
        let mut locals = [0i64; LOCALS_CAP];
        for k in (0..nparams).rev() {
            locals[k] = caller.pop_i64();
        }
        caller.check()?;
        // u32 to usize widens on both kernels (64-bit); no value is lost.
        let (co, cl) = (self.code_off[li] as usize, self.code_len[li] as usize);
        if co + cl > self.bytes.len() {
            return Err("truncated code body");
        }
        let body = &self.bytes[co..co + cl];
        let (ngroups, a) = uleb(body, 0);
        let mut c = a;
        let mut nlocals = nparams;
        for _ in 0..ngroups {
            // A body claiming more local groups than it has bytes used to run `c` past its end, and
            // `&body[c..]` below panicked (claude/nia-parse, step A2).
            if c >= body.len() {
                return Err("truncated code body");
            }
            let (n, a) = uleb(body, c);
            c += a + 1;
            for _ in 0..n {
                if nlocals >= LOCALS_CAP {
                    return Err("too many locals");
                }
                locals[nlocals] = 0;
                nlocals += 1;
            }
        }
        if c > body.len() {
            return Err("truncated code body");
        }
        let code = &body[c..];
        let mut st = Stack::new();
        // For `wasmfuzz`'s stack measurement: where this level's frames sit.
        note_sp(depth, &st as *const Stack as usize);
        self.run_body(code, &mut locals, &mut st)?;
        // Before the results are handed back: the last instruction may have
        // popped an empty stack, and there must be a value for every result.
        st.check()?;
        if st.sp < nresults {
            return Err("value stack underflow at return");
        }
        for k in 0..nresults {
            let v = st.d[st.sp - nresults + k];
            caller.push_i64(v); // full-width move (i32 cells are already sign-extended)
        }
        Ok(())
    }

    fn run_body(&self, code: &[u8], locals: &mut [i64], st: &mut Stack) -> Result<(), &'static str> {
        let mut ctrl: [Ctrl; CTRL_CAP] =
            core::array::from_fn(|_| Ctrl { is_loop: false, is_if: false, cont_pc: 0, stack_base: 0, arity: 0 });
        let mut cp = 0usize;
        let mut pc = 0usize;
        loop {
            if pc >= code.len() {
                return Err("code ran off end (missing end opcode)");
            }
            st.check()?;
            let op = code[pc];
            match op {
                0x00 => return Err("unreachable"),
                0x01 => pc += 1,
                0x02 | 0x03 | 0x04 => {
                    if pc + 1 >= code.len() {
                        return Err("code ran off end (block type)");
                    }
                    if cp >= CTRL_CAP {
                        return Err("blocks nested too deep (Nia holds 32)");
                    }
                    let bt = code[pc + 1];
                    let arity = if bt == 0x40 { 0 } else { 1 };
                    let body_start = pc + 2;
                    // A loop's label is its start, so it never needs its end: no scan at all (claude/nia-fast).
                    let (else_pc, end_pc) = if op == 0x03 && memo_on() {
                        (None, 0)
                    } else {
                        block_ends(code, body_start)
                    };
                    if op == 0x04 {
                        let cond = st.pop();
                        ctrl[cp] = Ctrl { is_loop: false, is_if: true, cont_pc: end_pc + 1, stack_base: st.sp, arity };
                        cp += 1;
                        pc = if cond != 0 { body_start } else { else_pc.unwrap_or(end_pc) };
                    } else {
                        let cont = if op == 0x03 { body_start } else { end_pc + 1 };
                        // A loop's label is its start and carries no values (WebAssembly 1.0), whatever its result
                        // type: a branch back keeps nothing. Its result stays on the stack when it falls through
                        // its `end`, which never reads arity (claude/nia-ctrl).
                        let arity = if op == 0x03 { 0 } else { arity };
                        ctrl[cp] = Ctrl { is_loop: op == 0x03, is_if: false, cont_pc: cont, stack_base: st.sp, arity };
                        cp += 1;
                        pc = body_start;
                    }
                }
                0x05 => {
                    // Only an `if` frame can meet its `else` (claude/nia-ctrl, fuzz finding 2).
                    if cp == 0 || !ctrl[cp - 1].is_if {
                        return Err("else outside an if");
                    }
                    pc = ctrl[cp - 1].cont_pc - 1;
                }
                0x0B => {
                    if cp == 0 {
                        return Ok(());
                    }
                    cp -= 1;
                    pc += 1;
                }
                0x0C => {
                    let (l, _) = uleb(code, pc + 1);
                    if branch(l, &mut ctrl, &mut cp, st, &mut pc)? {
                        return Ok(()); // N2: br to the function's own label = return
                    }
                }
                0x0D => {
                    let (l, a) = uleb(code, pc + 1);
                    if st.pop() != 0 {
                        if branch(l, &mut ctrl, &mut cp, st, &mut pc)? {
                            return Ok(());
                        }
                    } else {
                        pc = pc + 1 + a;
                    }
                }
                0x0E => {
                    // br_table: pop an i32 index; branch to labels[index] if it is
                    // in range, else to the default label (indexed multi-way branch).
                    // The count comes from the app; refuse one the body cannot hold (each
                    // label is at least one byte) and stop on a read that takes no bytes, so
                    // a huge count cannot spin here (claude/nia-brtable, fuzz finding 1).
                    let (count, a) = uleb(code, pc + 1);
                    let mut p = pc + 1 + a;
                    if count >= code.len().saturating_sub(p) {
                        return Err("truncated br_table");
                    }
                    let idx = st.pop();
                    let mut chosen = 0usize;
                    let mut found = false;
                    for k in 0..count {
                        let (l, a2) = uleb(code, p);
                        if a2 == 0 {
                            return Err("truncated br_table");
                        }
                        p += a2;
                        if !found && idx >= 0 && (idx as usize) == k {
                            chosen = l;
                            found = true;
                        }
                    }
                    let (deflt, a3) = uleb(code, p);
                    if a3 == 0 {
                        return Err("truncated br_table");
                    }
                    if !found {
                        chosen = deflt;
                    }
                    if branch(chosen, &mut ctrl, &mut cp, st, &mut pc)? {
                        return Ok(());
                    }
                }
                0x0F => return Ok(()),
                0x10 => {
                    let (fidx, a) = uleb(code, pc + 1);
                    pc = pc + 1 + a;
                    self.invoke(fidx, st)?;
                }
                0x1A => {
                    st.pop();
                    pc += 1;
                }
                0x20 => {
                    let (i, a) = uleb(code, pc + 1);
                    pc += 1 + a;
                    if i >= LOCALS_CAP {
                        return Err("local index out of range");
                    }
                    st.push_i64(locals[i]); // full-width (transparent for i32 locals)
                }
                0x21 => {
                    let (i, a) = uleb(code, pc + 1);
                    pc += 1 + a;
                    if i >= LOCALS_CAP {
                        return Err("local index out of range");
                    }
                    locals[i] = st.pop_i64();
                }
                0x22 => {
                    let (i, a) = uleb(code, pc + 1);
                    pc += 1 + a;
                    if i >= LOCALS_CAP {
                        return Err("local index out of range");
                    }
                    locals[i] = st.top_i64(); // tee: peek (leaves the value on the stack)
                }
                0x23 => {
                    // global.get
                    let (i, a) = uleb(code, pc + 1);
                    pc += 1 + a;
                    if i >= MAX_GLOBALS {
                        return Err("global index out of range");
                    }
                    st.push_i64(global_get(i));
                }
                0x24 => {
                    // global.set
                    let (i, a) = uleb(code, pc + 1);
                    pc += 1 + a;
                    if i >= MAX_GLOBALS {
                        return Err("global index out of range");
                    }
                    global_set(i, st.pop_i64());
                }
                0x28 | 0x2C | 0x2D | 0x2E | 0x2F => {
                    // i32.load / i32.load8_s / i32.load8_u / i32.load16_s / i32.load16_u
                    let (_al, a) = uleb(code, pc + 1);
                    let (off, b) = uleb(code, pc + 1 + a);
                    pc += 1 + a + b;
                    let w = match op {
                        0x28 => 4,
                        0x2C | 0x2D => 1,
                        0x2E | 0x2F => 2,
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    };
                    let addr = ea(st.pop(), off, w)?; // unsigned base, checked, inside 64 KiB or TRAP
                    let v = match op {
                        0x28 => mem_read32(addr),
                        0x2C => (mem_read8(addr) as i8) as i32,
                        0x2D => mem_read8(addr) as i32,
                        0x2E => (mem_read16(addr) as i16) as i32,
                        0x2F => mem_read16(addr) as i32,
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    };
                    st.push(v);
                }
                0x36 | 0x3A | 0x3B => {
                    // i32.store / i32.store8 / i32.store16 (the low 16 bits)
                    let (_al, a) = uleb(code, pc + 1);
                    let (off, b) = uleb(code, pc + 1 + a);
                    pc += 1 + a + b;
                    let v = st.pop();
                    let w = match op {
                        0x36 => 4,
                        0x3A => 1,
                        0x3B => 2,
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    };
                    let addr = ea(st.pop(), off, w)?; // unsigned base, checked, inside 64 KiB or TRAP
                    match op {
                        0x36 => mem_write32(addr, v),
                        0x3A => mem_write8(addr, v as u8),
                        0x3B => {
                            // i32.store16, written as i64.store16 is (ea has checked both bytes).
                            let b2 = (v as u16).to_le_bytes();
                            mem_write8(addr, b2[0]);
                            mem_write8(addr + 1, b2[1]);
                        }
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    }
                }
                0x29 | 0x30 | 0x31 | 0x32 | 0x33 | 0x34 | 0x35 => {
                    // i64.load and the narrow i64 loads (8/16/32-bit, sign- or
                    // zero-extended into a 64-bit value). Address is an i32.
                    let (_al, a) = uleb(code, pc + 1);
                    let (off, b) = uleb(code, pc + 1 + a);
                    pc += 1 + a + b;
                    let w = match op {
                        0x29 => 8,
                        0x30 | 0x31 => 1,
                        0x32 | 0x33 => 2,
                        0x34 | 0x35 => 4,
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    };
                    let addr = ea(st.pop(), off, w)?; // unsigned base, checked, inside 64 KiB or TRAP
                    let v: i64 = match op {
                        0x29 => mem_read64(addr),                    // i64.load
                        0x30 => (mem_read8(addr) as i8) as i64,      // i64.load8_s
                        0x31 => mem_read8(addr) as i64,              // i64.load8_u
                        0x32 => (mem_read16(addr) as i16) as i64,    // i64.load16_s
                        0x33 => mem_read16(addr) as i64,             // i64.load16_u
                        0x34 => mem_read32(addr) as i64,             // i64.load32_s (sign-extend)
                        _ => (mem_read32(addr) as u32) as i64,       // 0x35 i64.load32_u (zero-extend)
                    };
                    st.push_i64(v);
                }
                0x37 | 0x3C | 0x3D | 0x3E => {
                    // i64.store and the narrow i64 stores (truncate to 8/16/32).
                    let (_al, a) = uleb(code, pc + 1);
                    let (off, b) = uleb(code, pc + 1 + a);
                    pc += 1 + a + b;
                    let v = st.pop_i64();
                    let w = match op {
                        0x37 => 8,
                        0x3C => 1,
                        0x3D => 2,
                        0x3E => 4,
                        _ => return Err("unsupported opcode"), // not reached: the arm lists every opcode
                    };
                    let addr = ea(st.pop(), off, w)?; // unsigned base, checked, inside 64 KiB or TRAP
                    match op {
                        0x37 => mem_write64(addr, v),                // i64.store
                        0x3C => mem_write8(addr, v as u8),           // i64.store8
                        0x3D => {
                            let b2 = (v as u16).to_le_bytes();
                            mem_write8(addr, b2[0]);
                            mem_write8(addr + 1, b2[1]);
                        } // i64.store16
                        _ => mem_write32(addr, v as i32),            // 0x3E i64.store32
                    }
                }
                0x3F => {
                    // memory.size -> current linear memory size in 64KiB pages.
                    // (skip the 1-byte memory-index immediate)
                    st.push((MEM_SIZE / 65536) as i32);
                    pc += 2;
                }
                0x40 => {
                    // memory.grow(delta) -> old size, or -1 if it can't grow. Our
                    // linear memory is a fixed 64KiB static, so growth always fails.
                    let _delta = st.pop();
                    st.push(-1);
                    pc += 2;
                }
                0x41 => {
                    let (v, a) = sleb(code, pc + 1);
                    pc += 1 + a;
                    st.push(v);
                }
                0x42 => {
                    // i64.const — full 64-bit signed immediate.
                    let (v, a) = sleb64(code, pc + 1);
                    pc += 1 + a;
                    st.push_i64(v);
                }
                0x43 => {
                    // f32.const — 4-byte little-endian IEEE-754 immediate.
                    if pc + 5 > code.len() {
                        return Err("truncated f32.const");
                    }
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&code[pc + 1..pc + 5]);
                    st.push_f32(f32::from_le_bytes(b));
                    pc += 5;
                }
                0x44 => {
                    // f64.const — 8-byte little-endian IEEE-754 immediate.
                    if pc + 9 > code.len() {
                        return Err("truncated f64.const");
                    }
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&code[pc + 1..pc + 9]);
                    st.push_f64(f64::from_le_bytes(b));
                    pc += 9;
                }
                0x45 => {
                    let v = st.pop();
                    st.push((v == 0) as i32);
                    pc += 1;
                }
                0x50 => {
                    // i64.eqz
                    let v = st.pop_i64();
                    st.push((v == 0) as i32);
                    pc += 1;
                }
                0x51..=0x5A => {
                    // i64 comparisons: pop two i64, push an i32 boolean.
                    let b = st.pop_i64();
                    let a = st.pop_i64();
                    let r = match op {
                        0x51 => a == b,
                        0x52 => a != b,
                        0x53 => a < b,
                        0x54 => (a as u64) < (b as u64),
                        0x55 => a > b,
                        0x56 => (a as u64) > (b as u64),
                        0x57 => a <= b,
                        0x58 => (a as u64) <= (b as u64),
                        0x59 => a >= b,
                        _ => (a as u64) >= (b as u64), // 0x5A i64.ge_u
                    };
                    st.push(r as i32);
                    pc += 1;
                }
                0x46..=0x4F => {
                    let b = st.pop();
                    let a = st.pop();
                    let r = match op {
                        0x46 => a == b,
                        0x47 => a != b,
                        0x48 => a < b,
                        0x49 => (a as u32) < (b as u32),
                        0x4A => a > b,
                        0x4B => (a as u32) > (b as u32),
                        0x4C => a <= b,
                        0x4D => (a as u32) <= (b as u32),
                        0x4E => a >= b,
                        _ => (a as u32) >= (b as u32),
                    };
                    st.push(r as i32);
                    pc += 1;
                }
                0x6A..=0x73 => {
                    let b = st.pop();
                    let a = st.pop();
                    let r = match op {
                        0x6A => a.wrapping_add(b),
                        0x6B => a.wrapping_sub(b),
                        0x6C => a.wrapping_mul(b),
                        0x6D => {
                            if b == 0 {
                                return Err("div by zero");
                            }
                            a.wrapping_div(b)
                        }
                        0x6E => {
                            if b == 0 {
                                return Err("div by zero");
                            }
                            ((a as u32) / (b as u32)) as i32 // i32.div_u
                        }
                        0x6F => {
                            if b == 0 {
                                return Err("rem by zero");
                            }
                            a.wrapping_rem(b)
                        }
                        0x70 => {
                            if b == 0 {
                                return Err("rem by zero");
                            }
                            ((a as u32) % (b as u32)) as i32 // i32.rem_u
                        }
                        0x71 => a & b,
                        0x72 => a | b,
                        0x73 => a ^ b,
                        _ => return Err("unsupported arith opcode"),
                    };
                    st.push(r);
                    pc += 1;
                }
                0x74..=0x78 => {
                    // i32 shifts + rotates. The shift amount is masked to 0..31
                    // (WASM semantics) which also avoids a Rust shift-overflow panic.
                    let b = st.pop();
                    let a = st.pop();
                    let s = (b as u32) & 31;
                    let r = match op {
                        0x74 => a.wrapping_shl(s),             // i32.shl
                        0x75 => a.wrapping_shr(s),             // i32.shr_s (arithmetic)
                        0x76 => ((a as u32) >> s) as i32,      // i32.shr_u (logical)
                        0x77 => a.rotate_left(s),              // i32.rotl
                        _ => a.rotate_right(s),                // 0x78 i32.rotr
                    };
                    st.push(r);
                    pc += 1;
                }
                0x7C..=0x8A => {
                    // i64 arithmetic, bitwise, and shift/rotate ops. Same shape
                    // as the i32 block but on full 64-bit values; div/rem guard
                    // against divide-by-zero (TRAP, never a Rust panic) and shift
                    // amounts mask to 0..63 (WASM semantics + no shift overflow).
                    let b = st.pop_i64();
                    let a = st.pop_i64();
                    let r = match op {
                        0x7C => a.wrapping_add(b),
                        0x7D => a.wrapping_sub(b),
                        0x7E => a.wrapping_mul(b),
                        0x7F => {
                            if b == 0 {
                                return Err("i64 div by zero");
                            }
                            a.wrapping_div(b) // i64.div_s (wrapping handles MIN/-1)
                        }
                        0x80 => {
                            if b == 0 {
                                return Err("i64 div by zero");
                            }
                            ((a as u64) / (b as u64)) as i64 // i64.div_u
                        }
                        0x81 => {
                            if b == 0 {
                                return Err("i64 rem by zero");
                            }
                            a.wrapping_rem(b) // i64.rem_s
                        }
                        0x82 => {
                            if b == 0 {
                                return Err("i64 rem by zero");
                            }
                            ((a as u64) % (b as u64)) as i64 // i64.rem_u
                        }
                        0x83 => a & b,
                        0x84 => a | b,
                        0x85 => a ^ b,
                        0x86 => a.wrapping_shl((b as u64 & 63) as u32), // i64.shl
                        0x87 => a.wrapping_shr((b as u64 & 63) as u32), // i64.shr_s
                        0x88 => ((a as u64) >> (b as u64 & 63)) as i64, // i64.shr_u
                        0x89 => a.rotate_left((b as u64 & 63) as u32),  // i64.rotl
                        _ => a.rotate_right((b as u64 & 63) as u32),    // 0x8A i64.rotr
                    };
                    st.push_i64(r);
                    pc += 1;
                }
                0x61 | 0x63 | 0x64 => {
                    // f64 comparisons: pop two f64, push an i32 boolean.
                    let b = st.pop_f64();
                    let a = st.pop_f64();
                    let r = match op {
                        0x61 => a == b, // f64.eq
                        0x63 => a < b,  // f64.lt
                        _ => a > b,     // 0x64 f64.gt
                    };
                    st.push(r as i32);
                    pc += 1;
                }
                0x5B | 0x5D | 0x5E => {
                    // f32 comparisons: pop two f32, push an i32 boolean.
                    let b = st.pop_f32();
                    let a = st.pop_f32();
                    let r = match op {
                        0x5B => a == b, // f32.eq
                        0x5D => a < b,  // f32.lt
                        _ => a > b,     // 0x5E f32.gt
                    };
                    st.push(r as i32);
                    pc += 1;
                }
                0x92..=0x95 => {
                    // f32 arithmetic (IEEE-754; div-by-zero yields inf/NaN, no trap).
                    let b = st.pop_f32();
                    let a = st.pop_f32();
                    let r = match op {
                        0x92 => a + b, // f32.add
                        0x93 => a - b, // f32.sub
                        0x94 => a * b, // f32.mul
                        _ => a / b,    // 0x95 f32.div
                    };
                    st.push_f32(r);
                    pc += 1;
                }
                0xA8 => {
                    // i32.trunc_f32_s: f32 -> i32, rounding toward zero. Rust's
                    // saturating `as` (documented simplification vs WASM's trap).
                    let v = st.pop_f32();
                    st.push(v as i32);
                    pc += 1;
                }
                0xB2 => {
                    // f32.convert_i32_s: i32 -> f32.
                    let v = st.pop();
                    st.push_f32(v as f32);
                    pc += 1;
                }
                0xA0..=0xA3 => {
                    // f64 arithmetic. IEEE-754 semantics: divide-by-zero yields
                    // +/-inf or NaN (defined behavior), so there is no trap here.
                    let b = st.pop_f64();
                    let a = st.pop_f64();
                    let r = match op {
                        0xA0 => a + b, // f64.add
                        0xA1 => a - b, // f64.sub
                        0xA2 => a * b, // f64.mul
                        _ => a / b,    // 0xA3 f64.div
                    };
                    st.push_f64(r);
                    pc += 1;
                }
                0xAA => {
                    // i32.trunc_f64_s: f64 -> i32, rounding toward zero. WASM traps
                    // on NaN / out-of-range; Rust's saturating `as` is a safe,
                    // documented simplification (no UB, no panic).
                    let v = st.pop_f64();
                    st.push(v as i32);
                    pc += 1;
                }
                0xA7 => {
                    // i32.wrap_i64: keep the low 32 bits.
                    let v = st.pop_i64();
                    st.push(v as i32);
                    pc += 1;
                }
                0xAC => {
                    // i64.extend_i32_s: sign-extend.
                    let v = st.pop();
                    st.push_i64(v as i64);
                    pc += 1;
                }
                0xAD => {
                    // i64.extend_i32_u: zero-extend. The cell holds the i32 sign-extended, so go
                    // through u32 or a negative i32 would stay negative.
                    let v = st.pop();
                    st.push_i64((v as u32) as i64);
                    pc += 1;
                }
                0xB7 => {
                    // f64.convert_i32_s: i32 -> f64.
                    let v = st.pop();
                    st.push_f64(v as f64);
                    pc += 1;
                }
                0x1B => {
                    // select: pop cond, b, a; push cond != 0 ? a : b
                    let cond = st.pop();
                    let b = st.pop();
                    let a = st.pop();
                    st.push(if cond != 0 { a } else { b });
                    pc += 1;
                }
                other => {
                    crate::kprintln!("[nia] unsupported opcode {:#x}", other);
                    return Err("unsupported opcode");
                }
            }
        }
    }
}

fn branch(
    l: usize,
    ctrl: &mut [Ctrl],
    cp: &mut usize,
    st: &mut Stack,
    pc: &mut usize,
) -> Result<bool, &'static str> {
    // Ok(true) means "return from the function" (N2): label l == cp is the
    // function's own label, which WASM defines as a return carrying the
    // function's results; `invoke` takes them from the top of the stack with
    // its underflow check. A label past that is not open: TRAP (Nia guard).
    if l == *cp {
        return Ok(true);
    }
    if l > *cp {
        return Err("branch out of the function");
    }
    let target = *cp - 1 - l;
    if ctrl[target].is_loop {
        loop_poll()?; // N1: an endless loop can be stopped with Ctrl+C
    }
    let arity = ctrl[target].arity;
    st.check()?;
    if st.sp < arity || arity > 4 {
        return Err("value stack underflow at branch");
    }
    let mut tmp = [0i64; 4];
    for k in 0..arity {
        tmp[k] = st.d[st.sp - arity + k];
    }
    st.sp = ctrl[target].stack_base;
    for k in 0..arity {
        st.push_i64(tmp[k]);
    }
    if ctrl[target].is_loop {
        *cp = target + 1;
    } else {
        *cp = target;
    }
    *pc = ctrl[target].cont_pc;
    Ok(false)
}
