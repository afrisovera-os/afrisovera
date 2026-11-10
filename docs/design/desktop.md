# The AfriSovera desktop (Tonga shell) -- design spec

Owner: desktop and apps designer. Written 8 Nov 2026 for roadmap step 6 (`docs/05-roadmap.md`): *one desktop UI,
two device kinds* -- the phone in convergence mode and the PC. Code: `kernel/src/desk.rs` (new, shared by the ARM and
PC kernels), drawn on the shared framebuffer (`kernel/src/framebuffer.rs`), shown on a PC screen by
`kernel/src/pc/screen.rs`. The older `tonga.rs` stays the window server for app-owned surfaces (`wm`, Nia apps).

## 1. What it must feel like

Gold on black, like the brand splash (`framebuffer::brand_splash`): calm, dark, warm, with gold used sparingly for
what matters (focus, the clock, the brand mark). African identity comes from the brand monument, a kente-weave band and
earth colours, never from clip-art. Every pixel is drawn by our own code with fill primitives and the 8x8 bitmap
font; no images are loaded. Everything on screen is real: a status icon or setting that the system cannot back with
real state is not drawn (no fake battery percentage, no fake Wi-Fi bars).

## 2. Layout (640x480 logical pixels)

```
+---------------------------------------------------------------+  y=0
| [A] AfriSovera          14:32             net  AC  bell(2)   |  top bar, 22 px
|=== kente band (4 px) ==========================================|  y=22..24
|                                                               |
|      +----------------------------------+                     |
|      | [ic] Files               _   x   |  title bar 20 px    |
|      |----------------------------------|                     |
|      |  window body                     |                     |
|      +----------------------------------+                     |
|                     (wallpaper: monument, faint)              |
|            +-------------------------------------+            |
|            | [::] [Files] [Term] [Set] [About]   |  dock      |  floating, 8 px above the bottom
+---------------------------------------------------------------+  y=480
```

- **Top bar** (22 px, near-black `#0B0B0D`, gold hairline below it then a 4 px woven kente band):
  - left: the launcher button (a small gold monument mark) and the wordmark "AfriSovera";
  - centre: the clock, `HH:MM` from the hardware RTC (PL031 on ARM, CMOS on PC), UTC until time zones exist;
  - right: network (a wired/link glyph: gold when a DHCP lease is held, dim when the link is up without a lease,
    crossed out when there is no network device), power (a plug and "AC" until a battery driver reports a level; a
    battery outline with a real fill once one does), and the notification bell with a count badge.
- **Dock** (floating, centred, 52 px high, rounded, `#141519` with a 1 px `#2C2E36` line): the launcher tile, then one
  tile per app: Files, Terminal, Settings, About. A running app has a small gold dot under its tile; the focused app's
  dot is a wider gold bar. A click opens the app, or focuses it, or restores it when minimised.
- **Launcher** (opened by the launcher button, the dock's first tile or the Super key): a panel above the dock with
  a large grid of the installed apps (icon + name), then "This device" (network address, memory, files) and the
  uptime. Esc or a click outside closes it.
- **Notifications**: a ring of the last 8 notices (`desk::notify`). New ones show as a toast under the top-right of
  the bar for a few seconds; the bell opens the full list. Welcome, network lease and app events post notices.

## 3. Windows

- Title bar 20 px. Focused: `#22232A` with a 2 px gold line along its top and a white title. Unfocused: `#18191E`,
  dim title, no gold. The app's 12 px icon sits at the left of the title.
- Buttons at the right, 18x14 hit areas: minimise (a bar) and close (an X). The close button turns red (`#C1121F`)
  under the pointer. (Maximise is the touch profile's default, see 6.)
- Body `#15161A`, a 1 px border (`#2C2E36`, gold-dark `#5C4A1E` when focused), corners cut by one pixel, and a two-step
  shadow under it.
- Focus: one focused window at a time, always the top of the stacking order. A click anywhere on a window raises and
  focuses it; closing or minimising the focused window passes focus to the next window down.

## 4. Typography and colour

- Font: the built-in 8x8 bitmap font, drawn proportionally (`framebuffer::draw_string` measures each glyph's ink).
  Body text at 1x (8 px), headings at 2x (16 px). Lower case is drawn on both kernels in the desktop (the PC font's
  lower-case table, `pc/font_lower.rs`, is shared with the ARM build for the desktop only; the ARM text console keeps
  its upper-case look). A real outline font (rasterised Noto Sans or a commissioned face) is the next step once the
  framebuffer grows (section 7).
- Palette (one table in `desk.rs`; every colour on screen comes from it):

| Token | Hex | Use |
|---|---|---|
| night | `#0B0B0D` | top bar, deepest background |
| wall-top / wall-bottom | `#121016` / `#050405` | wallpaper gradient |
| surface | `#15161A` | focused window bodies, panels (unfocused: `#101114`) |
| surface-2 | `#1E1F25` | title bars, selected rows |
| line | `#2C2E36` | borders, separators |
| text | `#ECE7DC` | warm white |
| text-dim | `#A8A296` | labels, unfocused titles |
| gold | `#F4CB57` | brand gold: focus, clock, brand mark (the splash's wordmark gold) |
| gold-deep | `#C99E44` | icons, active toggles |
| gold-dark | `#5C4A1E` | focused border, faint monument |
| red | `#C1121F` | close, errors (the pan-African red of the console rule) |
| green | `#2FA05A` | OK / online (the console rule's green, lifted for contrast on black) |
| ochre | `#C27A2C` | Files icon |
| indigo | `#3D4C8F` | Settings icon |

- Kente band: two 2 px rows of 8 px blocks (gold, black, green, gold, red, black), the second row offset by half a block so it reads as woven cloth, not a dashed line (design review D1, docs/DESIGN_LOG.md).

## 5. Icons

Drawn from rectangles at 28x28 (dock) and 12x12 (title bars), on a rounded tile of the app's colour:
- Files: an ochre folder (tab + body);
- Terminal: a charcoal screen with a gold `>_` prompt;
- Settings: an indigo tile with three slider rows (knobs gold);
- About: a black tile with the gold monument (twin sails and needle), the same shape as the splash;
- Launcher: a 3x3 grid of gold dots.

## 6. Two device kinds, one desktop

The same code and the same apps run on both kernels. An **input profile** picks the metrics:

| | Pointer (PC; phone docked to a screen) | Touch (phone screen) |
|---|---|---|
| Windows | float, move by the title bar, overlap | open maximised under the top bar, one in front |
| Title bar | 20 px, minimise + close | 28 px, a 44 px wide close at the right |
| Dock | floating, 36 px tiles | full-width app bar, 44 px tiles (44 px minimum touch target) |
| Launcher | panel above the dock | the same grid, larger tiles |
| Lists | 16 px rows | 40 px rows with separators |
| Notifications | toast under the top bar, top right | snackbar above the app bar (never over a title bar) |
| Text input | the keyboard | the keyboard (on-screen keyboard: later) |

Default: the PC kernel uses Pointer; the ARM (phone) kernel uses Touch, and switches to Pointer when docked (today:
`desk mode pointer` or the Settings switch; later: when an external display or a mouse appears). Taps and clicks are
the same event (`down` at a point, `up`), so hit-testing is one code path.

### Input on the PC
- Mouse (USB, PS/2, touchpad, touchscreen): click to open/focus, drag a title bar to move, title-bar buttons.
- Keyboard: **Super** opens/closes the launcher; **Alt+Tab** focuses the next window; **Alt+F4** closes the focused
  window; **Super+Down** minimises it; **Super+Arrow** (left/right/up) moves it by 16 px; **Esc** closes the launcher or the
  notification list. Other keys go to the shell, and the Terminal window shows it.
- Every action is also a shell command, `desk ...`, so the desktop can be scripted and tested on both kernels:
  `desk open|close|min|focus <app>`, `desk next`, `desk tap <x> <y>`, `desk drag <x> <y> <dx> <dy>`, `desk key <name>`,
  `desk launcher`, `desk mode pointer|touch`, `desk notify <text>`, `desk list`.

### How input reaches the desktop (both kernels)
Interrupt handlers (USB/PS/2 mouse, keyboard) never draw: they post small events (`down x y`, `move x y`, `up`, `key`)
to a lock-free ring in `desk.rs`. The shell's idle loop (`cpu_getc`, before `wfi`) drains the ring, hit-tests, updates
the window state and redraws, in mainline context. So no drawing happens inside an interrupt and the ARM and PC
kernels run identical desktop code; on the PC the screen copy (`screen::tick`) then shows the new frame.

## 7. Resolutions beyond 640x480

Today `framebuffer::WIDTH/HEIGHT` are constants (640x480) and the PC scales that frame by a whole number onto the
real screen (`pc/screen.rs`). The desktop is written to grow:
1. **Now:** no desktop coordinate is a literal screen size. The bar, dock, launcher, window placement and clamping all
   derive from `W`/`H` and a unit `U` (= 1 at 640x480), so changing the constants re-lays out the desktop.
2. **Next (framebuffer):** `WIDTH/HEIGHT` become run-time values (`fb::width()/height()`), with the buffer taken from
   physical frames for the chosen mode (ramfb accepts any size; on the PC the GOP mode). Logical size = physical size /
   integer UI scale, e.g. a 1920x1080 laptop runs at 960x540 logical at 2x, so text stays crisp and the layout gains
   real room rather than just bigger pixels.
3. **Then (UI scale):** `U` becomes 1, 1.5 or 2 by pixel density; tiles, bars and the font scale by `U`. Pointer
   coordinates arrive already in logical pixels (xhci.rs scales absolute devices to the framebuffer size).
4. Suites grade layout by role (the bar's band, the dock's row, a window's gold focus line) found relative to `W`/`H`,
   so they survive a mode change.

## 8. Apps (v1, drawn by the desktop; real data only)

- **Files**: the files in the VFS (name, size, an icon by type: text, WASM, other). Click a file to select it; its
  first lines (printable text) or its size and type show in a preview pane.
- **Terminal**: the AfriSh shell. Shows the last lines the shell printed and the line being typed, with a cursor;
  keyboard input goes to the shell as before, so every command works. Desktop log lines (`[desk] ...`) are hidden.
- **Settings**: Display (logical size, input profile switch), Desktop (wallpaper monument on/off, 24-hour clock),
  Network (device, lease), System (cores, memory, boots). Switches take effect at once; they are not yet saved
  across a reboot.
- **About**: the monument, "AfriSovera OS", the kernel (Mansa), desktop (Tonga), architecture, cores, memory, uptime
  and the line "Own your device - own your future".

## 9. Testing

- `desk-test.sh` (ARM): `gui`, scripted `desk` commands (open, tap, close, focus, mode), `desk list` graded line by
  line, and the framebuffer screenshot graded by pixels (top bar, kente band, dock, a focused window's gold line).
- `pc-desk-test.sh` (PC): the same desktop driven by the USB mouse and keyboard (dock clicks, a title-bar drag,
  Alt+Tab, Alt+F4, Super), graded on serial and on the screenshot.
- `pc-desktop-test.sh` keeps grading the dock click on the PC (its Files tile now opens the Files window).
- Each grader has an offline selftest with red and green arms, as every suite in `scripts/` does.

## 10. Not in v1 (said plainly)

App-owned desktop windows over IPC (apps here are drawn by the desktop), resizing, an on-screen keyboard, saved
settings, time zones, a battery driver, Wi-Fi bars, a real outline font, sizes other than 640x480.
