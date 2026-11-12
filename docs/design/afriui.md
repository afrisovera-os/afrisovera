# Sovereign UI: AfriSovera's own design language

Owner's mandate (9 Nov 2026, go given 10 Nov): a distinct, coherent, fast, sovereign look for the AfriSovera desktop,
PC first and phone later. Not a copy of macOS, Windows or Linux that is only a little nicer. We win on identity,
speed and coherence, not on feature count.

Written 10 Nov 2026 by the desktop designer (afri-desktop). This file is the blueprint (law 0) for every change to the
look of the desktop. The older spec, `docs/design/desktop.md`, still describes the apps, the input model, the `desk`
commands and the phone's current desktop; where the two disagree on looks, this file wins for the PC.

## 1. Why the old look had to go

The 8 Nov desktop is good work, but its shape is macOS: a menu bar along the top, a floating dock of icons centred
at the bottom, rounded windows with a close button at the right. Gold on black makes it pretty; it does not make it
ours. Anyone who sees a screenshot thinks "a Mac theme". That is the thing the owner asked us to stop.

## 2. Five principles

1. **Ours at a glance.** A screenshot with the words cut out must still say AfriSovera. We get there with shapes and
   patterns that come from African making (carved and stamped edges, woven cloth, mud cloth, the brand monument), not
   from clip-art, flags or masks.
2. **Fast on old machines.** Every pixel is a flat fill drawn by our own code: no transparency, no blur, no images, no
   floating point. A whole frame must draw in well under 10 ms on a 2012 laptop with no graphics acceleration (one CPU
   core writing a plain framebuffer). Motion, when it comes, is short and can be switched off.
3. **Height is precious.** Most laptops are wide and short (1366x768, 1920x1080). The system's own bar goes down the
   LEFT side, so windows keep the full height of the screen.
4. **Every app wears its cloth.** Each app has a woven band of its own colours. The cloth is how you find a window, how
   you know which one has focus, and how the app shows in the Spine. One idea, used everywhere.
5. **Honest.** Nothing on screen pretends: no fake battery levels, no fake Wi-Fi bars, no buttons that do nothing
   (carried over from desktop.md section 1).

## 3. Shape: the cut corner

Every panel, window, tile and button has its corners **cut at 45 degrees** (a chamfer), never rounded. Cut corners
read as carved wood and stamped Adinkra blocks, they are crisp at small sizes and in an 8x8 pixel font world, and they
are cheaper to draw than round ones (one subtraction a row, no square roots).

One rule decides which corners are cut (review S1 asked whether the lopsided cut was a gimmick; making it a rule
answers that):
- **Containers** (windows, the notification list, toasts) cut only two corners: top left and bottom right (8 px on a
  window). This lopsided cut is the signature shape of the system: a window looks like a carved tag. Nothing in
  macOS, Windows or GNOME looks like it.
- **Controls** (tiles, buttons, chips, switches) cut all four corners (2 to 6 px by size).
- **The Circle** (the launcher) is the one octagon: a square with all four corners cut deep.

## 4. Colour: Earth and Night

Warm, dark and earthy, with gold kept for what matters. Every colour on screen comes from one table in the code.

| Token | Hex | Use |
|---|---|---|
| night | `#100C0A` | the Spine, the deepest dark (warm black, not grey) |
| soil-top / soil-bottom | `#1B1512` / `#0A0807` | wallpaper gradient |
| mud | `#241C17` | mud-cloth marks on the wallpaper (very faint) |
| surface | `#1E1814` | window bodies, panels |
| surface-off | `#161210` | bodies of windows without focus |
| lintel / lintel-off | `#2A211B` / `#1C1613` | title bars with and without focus |
| line | `#4A3B2F` | borders, separators (lifted from `#3A2E25`, review S1) |
| shea (text) | `#F2EADC` | main text, warm white |
| dim | `#B0A391` | labels, titles without focus |
| gold | `#F2C14E` | focus, the clock, the monument |
| gold-deep | `#C8963A` | icons, switches that are on |
| gold-dark | `#5E4721` | panel borders, the Circle's ring |
| terracotta | `#C4562B` | hover on close, warnings |
| red | `#C1121F` | errors and close under the pointer |
| malachite | `#2E9E6A` | OK, online |
| ochre | `#D08A2E` | Files |
| adire (indigo) | `#34508F` | Settings |
| shadow | `#050403` | the hard relief shadow |

**Cloths** (each a woven band of three colours, two offset rows like kente strips):

| App | Cloth |
|---|---|
| Files | ochre, night, gold |
| Terminal | malachite, night, gold |
| Settings | adire, shea, gold |
| About | gold, red, malachite (the classic kente of the brand band) |
| The system (Spine edge) | gold-deep, night, malachite, gold-deep, red, night (the old kente band, kept) |

## 5. Type

Today: the built-in 8x8 bitmap font, drawn proportionally, with lower case (unchanged). Rules: body text 1x, window
and panel headings 2x, never more than two sizes in one panel, labels in `dim`, values in `shea`, one gold item per
panel at most. Later slice: our own bitmap face (8x16, drawn by us, reviewed) so text is taller and easier to read at
higher resolutions. No outline font until the framebuffer is bigger than 640x480.

## 6. The model: Spine, Circle, Cloth

```
+----+---------------------------------------------------------+  y=0
|mon |                                                         |
|----|    +=\===== Files ==========================[_][x]+     |  the focused window: its cloth
|[F]•|    |                                              |     |  along the top of the lintel
|[T] |    |   window body                                |     |
|[S] |    |                                              |     |
|[A] |    +----------------------------------------------\     |  cut corners: top left, bottom right
|    |                                                         |
|    |            (wallpaper: soil, mud-cloth marks,           |
|net |             the monument faint)                         |
| AC |                                                         |
|bell|                                                         |
|14: |                                                         |
|32  |                                                         |
+----+---------------------------------------------------------+  y=480
 the Spine (44 px) with a woven edge
```

- **The Spine** (left edge, full height, 40 px of night plus a 4 px woven edge in the system cloth). Top: the monument
  button, which opens the Circle. Then one cut-corner tile per app. A **bead** on the left edge marks an app: a short
  gold bead when its window is open, a long gold bead when it has focus. Bottom: network, power, the notification bell
  with its count, and the clock (hours over minutes, gold). Notices and the bell's list open beside the Spine.
- **Windows.** Lintel (title bar) 26 px. The focused window shows its **cloth** (a 6 px woven band, three rows) along
  the top of the lintel and a gold-deep border; others show a plain line and dim title (review S1: 4 px of cloth and a
  gold-dark border did not show focus at a glance). Minimise and close at the right, cut-corner
  buttons; close goes red under the pointer. A hard shadow 4 px down and right, like a relief carving. Same focus,
  stacking and keyboard rules as desktop.md section 3.
- **The Circle** (Super, or the monument button): the launcher. An octagon in the middle of the free space with the
  monument at its centre and the apps standing in a ring around it, like people round a fire ("I am because we are").
  Up to 8 apps stand in the ring (pinned ones first); more go to a flat grid under the octagon, never a second ring
  (review S1: a second ring is hard to remember and to walk with arrow keys). Below the ring: this device (network, memory, files) and how long
  it has been up. Arrow keys walk round the ring (later slice); Esc or a click outside closes it.
- **Wallpaper.** Soil gradient, faint mud-cloth marks (small dots and chevrons on a grid, one shade above the soil),
  and the monument standing faint in the middle. The marks are drawn each frame from fills, so there is no image.

## 7. Motion (later slice)

At most 3 frames (about 60 ms) for anything: a window unfolds from its lintel downward, the Circle grows from the
monument. No easing curves that need floating point. A Settings switch turns all motion off. Motion is on by default,
and the tests run with it on (slice 6, section 17): every `desk` command typed in the shell, except `tap` and `drag`,
first shows a running motion's last frame, so a script only ever sees final frames, which are the same with motion
on or off; the suite `pc-deskmotion` tests the motion itself.

## 8. Speed budget and how we keep it

- Flat fills only, clipped once per rectangle; cut corners cost one subtraction per row.
- The mud-cloth wallpaper is at most about 1,300 small fills a frame.
- Later slice: redraw only the part of the screen that changed (damage rectangles), so moving the pointer or a clock
  tick does not repaint the whole screen.
- The frame stays at 640x480 logical pixels for now; the PC scales it to the screen (desktop.md section 7). Every
  layout number comes from the screen size and the Spine width, so a bigger logical screen later re-lays out.

## 9. Phone later

The phone keeps the 8 Nov desktop for now: the ARM kernel image must stay byte-identical until the architect agrees.
The PC builds the new desktop from `kernel/src/afriui/desk.rs`, a copy of `kernel/src/desk.rs` with the new look;
the phone still builds `kernel/src/desk.rs`. When the phone moves over (touch profile: the Spine becomes a bottom bar
in portrait, tiles 44 px), the two files become one again and the old one is removed. Until then a behaviour fix in
one file must be made in both.

## 10. Slices

1. **Theme, Spine and window frame** (this round): the palette, cut corners, the Spine replacing the top bar and dock,
   the new window frame with cloths, the Circle launcher, mud-cloth wallpaper, notices beside the Spine. Suites
   `pc-desk` and `pc-desktop` updated to the new places; the ARM `desk` suite stays on the phone.
2. Keyboard walk of the Circle and the Spine (arrows, Enter), motion with its off switch.
3. Damage-rectangle redraw and a frame-time readout (`desk perf`).
4. Our own 8x16 bitmap face.
5. Desktop at the screen's native size (with the F9b work), then the phone.

## 11. Testing

Suites grade layout by role, found from the screen size: the Spine's night and its woven edge, a tile, the focused
window's cloth, a body. `pc-desk` drives the desktop with the USB mouse and keyboard; `pc-desktop` clicks the Spine's
Files tile. Screenshots for review come from `scripts/desk-shots.sh` run on GitHub Actions (not on this PC).

## 12. Review S1 (Gemini Pro, 10 Nov 2026), what changed

Full table in `docs/DESIGN_LOG.md` (Sovereign S1). Taken: a thicker cloth (6 px) and a gold-deep focus border, lighter
lines, a flat grid for apps past eight, the corner rule in section 3. Not taken, with reasons: cutting all four window
corners (the lopsided cut is the identity; the rule makes it consistent), and moving the clock and bell to the top of
the Spine (the bottom is where people coming from Windows already look for them, and the top of the Spine is where the
Circle button and the apps belong).


## 13. The whole screen (slice 1b, 10 Nov 2026, field item F9)

Asked by the OS lead after the owner merged slice 1: the desktop must fill the laptop's screen at its own resolution,
not sit as a 640x480 box in a dark frame.

- **A canvas of the screen's size, PC only.** The desktop draws into its own buffer (`kernel/src/pc/canvas.rs`),
  sized from the firmware's screen mode when the kernel starts. The text console keeps the shared 640x480
  framebuffer (and the phone keeps everything): the ARM image does not change and Tonga is not touched.
- **A whole-number UI scale**, so the 8x8 font stays sharp: scale = the smaller of width/800 and height/500, at
  least 1, at most 4. Logical size = screen size / scale. Examples: 1366x768 runs at 1x (1366x768 logical),
  1920x1080 at 2x (960x540), 2560x1440 at 2x (1280x720), 3840x2160 at 4x (960x540). `screen ui 1..4` overrides it
  until the next restart; `screen ui` shows it. The canvas holds at most 1920x1200 logical pixels; a bigger logical
  size (a very wide screen at 1x) is centred in the dark frame.
- **What follows the canvas while the desktop is shown:** the screen copy (whole-number scale, no stretching), the
  mouse pointer's range, and every layout number (the Spine is full height, the Circle centred in the free space).
  `clear` returns to the 640x480 console exactly as before.
- **Speed:** the same flat fills; a 960x540 canvas is 1.7 times the pixels of 640x480, a 1366x768 one 3.4 times.
  The copy to the screen is one row built and copied per logical row, as before.
- **Later:** windows open in a cascade from the top left whatever the size; placing them relative to the canvas
  centre and remembering where they were moved is part of slice 2.

## 14. Slice 2 (10 Nov 2026): windows placed by the screen, the keyboard walk, model and paint apart

- **Windows open round the middle of the free space** (right of the Spine), in a cascade, whatever the screen's
  size: each app has a place given as an offset from that centre (Files up and left, Terminal down, Settings and
  About between), clamped so the lintel stays on the screen. On a big screen they no longer bunch in the top left.
  Sizes stay as they are (they are in logical pixels, so the UI scale already makes them bigger on big screens).
- **The keyboard walk.** While the Circle is open, the arrow keys walk round the ring (Right and Down go clockwise,
  Left and Up back), the chosen app gets a gold cut frame, and Enter opens it; Esc closes the Circle as before. On
  the Spine, Super+1 to Super+4 open, focus or restore its apps in order (Files, Terminal, Settings, About), as a
  click on the tile does. `desk key up|down|left|right|enter|super-1..4` script the same keys.
- **Model and paint apart** (architect's review): `kernel/src/afriui/desk.rs` keeps the model (state, the input
  ring, windows and focus, hit-testing geometry, the `desk` commands); `kernel/src/afriui/paint.rs` holds the
  theme (palette, cloths, cut-corner primitives, text, icons) and every drawing function. Paint reads the model;
  the model never draws. When the phone moves over, it keeps this model and gets a phone paint (or the same one with
  the touch profile), instead of a third copy of the desktop.

## 15. Slice 3 (10 Nov 2026): a change repaints only what it touches

Until now every change (a window dragged one step, a close button lit, the clock's minute) redrew the whole desktop:
about 64 ms in QEMU for 960x540, far more pixels than changed.

- **What changed is found, not guessed.** Before the desktop handles a batch of input it takes a small snapshot of
  the scene (each window's place, open or minimised, focus and stacking; the Circle and its chosen app; the
  notification list and the bell's count; the toast; the lit close button). After the batch it compares: each
  window that moved, opened, closed or changed focus damages its old and new box (shadow included) and the Spine (its
  beads); the Circle, the list and the toast damage their own boxes. Changes the snapshot cannot see mark their box
  themselves: new Terminal text (Terminal's window), the clock's minute (the Spine's foot), a click in Files or
  Settings (that window).
- **At most 8 damaged boxes** per frame; a ninth folds them all into one box round them. Anything the desktop cannot
  place (a new UI scale, the input profile, a script command) redraws everything, as before. If something asked for
  a redraw and no box was found, the whole desktop is redrawn: correctness first.
- **Paint draws each box on its own:** every fill is clipped to the box, and a window, the Spine, the Circle, the list
  or the toast that does not touch the box is skipped whole. The wallpaper draws only the box's rows and marks.
- **Checked, not trusted:** `desk verify` hashes every row of the desktop as the boxes left it, redraws everything,
  and counts the rows that differ; pc-desk requires 0 after its mouse and keyboard walk. `desk perf` counts full and
  partial frames and times them apart.

## 16. Slice 4 (10 Nov 2026): the Sovereign face

The owner noticed the odd letters of the old 8x8 font (the lower-case "g" in Settings, tails that sit on the line).
Gemini and the architect both named it the weakest part of the look. Slice 4 gives the PC desktop its own bitmap
face, drawn by us, glyph by glyph (no other font copied), in `kernel/src/afriui/face.rs`:

- **A cell of 8 x 16, drawn on a 5-pixel-wide, 12-row grid inside it:** capitals and figures 9 rows tall, the
  small letters 6 rows tall (x-height), real descenders of 3 rows (g, j, p, q, y and the comma hang below the
  line), ascenders as tall as the capitals. The rest of the cell is left for accents later.
- **The look:** plain, open shapes at one-pixel strokes, so it stays sharp at 1x and doubles cleanly at 2x; the
  round letters cut their corners at 45 degrees, as the windows do; a single-storey "g" with an open tail; the
  zero is slashed so it never reads as O.
- **Proportional for the desktop, fixed for the Terminal:** text advances by each glyph's own width plus one
  pixel; the Terminal keeps 6-pixel cells (every glyph fits 5 wide), so its columns stay aligned.
- **Line heights follow:** the Terminal and the file preview step 13 pixels a line (was 11); the lintel centres the
  9-row capitals. The text console and the phone keep the old font (shared files, ARM image).
- The 95 printable ASCII characters only; other bytes draw as "?".

## 17. Slice 6, motion (11 Nov 2026) [law 0]

The last item of section 10 not built yet (slice 2 left it out). Section 7 is the rule: at most 3 frames, about
60 ms, no floating point, a Settings switch turns it off.

**What moves (only these two):**
- **A window unfolds from its lintel downward** when it opens or is restored from the Spine: frame 1 shows the lintel
  and the top third of the body, frame 2 two thirds, frame 3 the whole window. The body is clipped, not scaled, so
  each frame is the same drawing code with a shorter clip. Close and minimise stay instant (going away should never
  make you wait).
- **The Circle grows from its Spine button:** frames 1 and 2 draw the ring's outer cut-square at one third and two
  thirds of its size, centred on its final place, with no tiles; frame 3 is today's Circle. Closing it is instant.

**How it is built:**
- The model (`desk.rs`) gets one `Anim { what, app, start, step }` in `St`, set by `open`, `activate` (restore) and
  the launcher; paint (`paint.rs`) reads `step` to clip the window body or size the ring. One motion at a time: a new
  one finishes the old at once.
- Frames are driven by time from `idle()` (mainline, as today), each frame 2 ticks (20 ms) after the last. A late
  frame is skipped, never queued: if 6 ticks have passed, the final frame is drawn. Each frame damages only the
  window's or the Circle's box (slice 3), so a frame costs at most one window's redraw.
- **The off switch:** Settings has a fourth row "Motion" under the clock row, and `desk motion on|off` for scripts.
  Off means frame 3 at once, exactly the behaviour before slice 6.
- **Tests stay deterministic without changing every suite:** every `desk` command from the shell, except `tap` and
  `drag`, shows a running motion's last frame first (`tap` and `drag` are input, as a click is: they only release a
  hold). So the existing suites see only final frames, motion on or off. A close, a minimise, an input-profile switch,
  `screen ui` and `clear` also finish a motion; a new motion finishes the old one (one at a time).
- **No log line per frame:** frames are drawn from `idle()` while the shell waits, so a log line would land after
  the prompt in every suite. `desk motion` reports instead: on or off, how many motions have run, and the last two
  with the tick of each frame (0 = skipped) and `early` when something finished them.
- **`desk motion hold <1|2> <app|circle>`** (test only, marked so in `desk help`): starts that motion and stops its
  frame clock at that frame. The next `desk` command, any input event or `desk motion off` releases it. It changes
  nothing else: the same model and the same damage.
- **Suite `pc-deskmotion`** (PC at 640x480, UI scale 1, two boots; selftest 14/14): motion on by default; About's
  frames at least 2 ticks apart, frame 3 drawn; a held Files finished by the next command; a tap on the Settings tile
  while the Circle is held at frame 1 opens Settings (input uses the final geometry) and ends the Circle's motion
  early; the Settings row turns motion off and an open then runs none; `desk verify` 0 rows after every case;
  `desk perf` names the slowest motion frame. A screenshot with Files held at frame 1 over About: below Files' first
  third, About's body; above it, Files' body; left of About, not Files.
- **Speed:** `desk perf` reports the slowest frame the motion clock drew. Budget: under 16 ms a frame at 1920x1080
  (one window box). Not gated: QEMU is not a real machine, so the owner's HP gives the real number. If a real machine
  is slower, the skip rule keeps the motion at about 60 ms.

**The architect's answers** (`ARCHITECT_REVIEWS.md`, "AfriUI slice 6, motion, plan gate"): the ARM image may move
once, with the P2 proof; default on; `hold` may ship, on the conditions above. The questions as asked:
1. **The ARM image moves once more** (`afriui/desk.rs` and `paint.rs` are shared since P2). Same proof as P2:
   the cloud runner like-for-like md5 and llvm-size, the `desk` suite and a full ARM walk before the merge.
2. **Default on or off?** I propose **on** (the blueprint wants the desktop to feel alive, and the off switch is one
   tap away), with the test rule above so no suite depends on it. Off by default is the safe alternative.
3. **`desk motion hold <step>`** is a test-only command in the shipped kernel (like `desk verify`). OK, or behind
   the selftest build only?

Size: about 150 lines in `desk.rs` and `paint.rs`, a Settings row, one new suite. One branch, one Actions run at the
end (minutes rule), the cloud runner for the rest.

## 18. The faster console (11 Nov 2026) [law 0]

The architect's gated item (2) from the F6 review (`ARCHITECT_REVIEWS.md`, "F6 handshake resend"). The kernel's text
console in `framebuffer.rs` draws pixel by pixel in Rust loops, and at opt-level 0 one new line with a scroll takes
over 100 ms. `uart::_print` holds interrupts masked for all of that, so the timer and the network go blind on both
kernels. Target: one console line with a scroll in under 5 ms at opt-level 0, on both kernels.

**What changes (only `framebuffer.rs`, plus one shell verb):**
- `con_scroll` moves each pixel row with `core::ptr::copy` (memmove from compiler_builtins, which is optimised even
  in a debug build), not a pixel loop.
- `fill_rect` clips once, fills the first row, then copies that row down. The first row is filled by doubling copies
  (1, 2, 4, ... pixels with `copy_nonoverlapping`), not with `slice::fill`. `fill` is generic, so it is compiled
  in our crate at opt-level 0, about as slow as the loop it replaces. Results are the same pixels as before for
  every input, including clipped and empty rectangles.
- The console's glyph (`con_putc`) builds each 8-pixel row of a character with its background in a small array and
  copies it in once, instead of one background fill plus up to 64 one-pixel fills.
- Nothing else changes: `blit`, `draw_char` and `draw_string` keep their code (they call `fill_rect`, so they get
  faster too), and `uart::_print`'s locking stays as it is.

**How it is measured and proved:**
- A new shell verb, `fbcon time`, runs on both kernels while the console is shown. It writes 8 full-width lines at
  the bottom row, so each one ends in a scroll, and prints the fastest and slowest time in microseconds. It also
  times the scroll on its own, and runs two self-checks:
  - after every scroll, the moved rows hash the same as they did before, and the new bottom row is all background;
  - the last line's pixels match the font's glyph bits, pixel by pixel.
  It prints: `[fbcon] one line with a scroll: fastest A us, slowest B us over 8 lines; the scroll alone: C us`,
  `[fbcon] scroll check: 8 of 8 scrolls moved every row up one text row` and
  `[fbcon] glyph check: 76 chars, 0 pixels differ`.
- The verb lands first in its own commit with the old drawing code, so "before" and "after" come from the same probe.
- A new suite, `fbcon-time`, boots the ARM kernel and the PC kernel, runs `fbcon time` on each and grades:
  - both self-checks;
  - the fastest line under 5000 us on each kernel (QEMU's timing varies, so the fastest of 8 is graded, and the
    slowest is printed but not graded);
  - no panic.
  Its grader selftest runs first.
- The proof, as for P2: the cloud runner like-for-like md5 and `llvm-size` of branch and base (945f70a4), the console suites
  (`pc-screen`, `desk`, `pc-desk`), `pi` and the board-* suites green, and a full ARM walk before the merge. The ARM
  image moves once. Then one Actions run on the os2 pool.

## 19. The row-at-a-time blit (12 Nov 2026) [law 0]

After section 18, `framebuffer::blit` is the last pixel-by-pixel copy in `framebuffer.rs`. It calls `put` once
per pixel, and `put` checks the bounds each time. It has two callers:
- `pc/conback.rs` `restore()`, which puts the live console back after scrollback. It runs under the print lock, in
  the same interrupts-off stretch as the console, and copies the whole console block (616 x 384 pixels).
- `tonga.rs`, the compositor, which copies an app's surface into its window body.

Target: the console-sized blit in under 5 ms at opt-level 0, on both kernels.

**What changes (only `framebuffer.rs`, plus one shell verb):**
- `blit` clips the block to the screen once, then copies each row with one `core::ptr::copy` (memmove from
  compiler_builtins, optimised even in a debug build). Clipped, partial and empty blocks give the same pixels as
  the pixel loop did: columns at or past `WIDTH` and rows at or past `HEIGHT` are left out, and source pixel
  (i, j) still comes from `src[j * stride + i]`.
- The architect's conditions (ARCHITECT_REVIEWS, 12 Nov):
  - The clip adds with `saturating_add`, so a huge `w` or `h` cannot wrap. `fill_rect` gets the same.
  - The `# Safety` contract reads: `src` is valid for `(h-1)*stride + w` pixels, and must not overlap the
    destination rows. The memmove only protects within one row, since rows are copied top to bottom. Neither caller
    passes the framebuffer as the source.
  - Tonga's width floor, in its own commit: `win_create` keeps a window at least 2 pixels wide. Until now an app's
    0- or 1-pixel window made the compositor's `w - 2` wrap and the kernel fault. persist-big boot 6 runs an app
    from the disk that opens a 1-pixel window, and checks that it runs to its end and the shell still answers.
- `put` (the per-pixel write with its bounds check) had `blit` as its only caller, so it is removed.
- Nothing else changes: the two callers and their arguments stay as they are.

**How it is measured and proved:**
- A new shell verb, `fbcon blit`, runs on both kernels while the console is shown. It is added in its own commit,
  on the old code, so "before" and "after" come from the same probe. It times 8 blits each of:
  - the console block (the size `restore()` copies);
  - the whole screen, 640 x 480.
  The source is one row on the stack with stride 0, so the probe adds no static memory. It then runs two checks:
  - **copy:** a 24 x 8 block from a 32-pixel-stride source on the stack, with every pixel different, must land
    pixel for pixel;
  - **clip:** a 64 x 64 block put 20 pixels from the right edge and 10 from the bottom must fill exactly the
    20 x 10 corner that fits, and leave the pixel just outside it unchanged.
  Afterwards it redraws the console (`console_reset`), so the screen is left as `clear` leaves it.
  It prints:
  - `[fbblit] console block 616x384: fastest A us; whole screen 640x480: fastest B us`
  - `[fbblit] copy check: 192 of 192 pixels right; clip check: 200 of 200 inside, edge unchanged`
- The `fbcon-time` suite types `fbcon blit` after `fbcon time` in the same two boots (ARM ramfb, then the PC
  kernel), so there are no new boots or ports. On each kernel it grades:
  - the fastest console-block blit under 5000 us;
  - both checks.
  The grader selftest gains cases for the new lines.
- Proof as for section 18:
  - the cloud runner like-for-like md5 and `llvm-size` for the base and the branch, in the same job;
  - the before and after numbers;
  - the console, desk, pi and board-* suites, and a full ARM walk;
  - one os2 Actions run.

The ARM image moves once more. The lead has agreed, since the public export of 1e816508 sets the baseline first.

## 20. Fast window create and present (12 Nov 2026) [law 0]

After section 19, Tonga's window server is the slowest drawing left behind interrupts masked. Both of its calls
run in syscall context. A probe that was never pushed timed them on a cloud runner (b0a8f5da, fastest of 8):

| kernel | `win_create` 240x130 | `win_create` 40x20 | `win_present`, 1 window |
|---|---|---|---|
| ARM | 5120 us | 5114 us | 5874 us |
| PC | 6035 us | 5993 us | 5689 us |

The size makes no difference to `win_create`, because it clears all 240 x 130 = 31,200 pixels of the surface in
a pixel loop with a bounds check on each.

Target: `win_create` and `win_present` (one window) each under 1000 us at opt-level 0, on both kernels, with the
same pixels as before.

**What changes (only `tonga.rs`, plus one shell verb):**
- `win_create` clears the surface with one `core::ptr::write_bytes` (memset from compiler_builtins, optimised even
  in a debug build). The surface is all zero afterwards, as before.
- `win_present`: the probe splits its time into the wallpaper (the gradient and the bar), the two lines of text,
  and the windows (chrome and blit). The fix goes where the time is, and that choice is sent to the architect with
  the before numbers, before the fix commit. The likely place is the gradient: 480 one-row `fill_rect` calls,
  each with its own `lerp`. A row of the same colour would be filled once and copied down for every row that
  shares it. Whatever the fix, the screen must match the old code pixel for pixel.
- Nothing else changes. The callers (nia.rs, process.rs, pc/proc.rs) and their arguments stay as they are.
- The `saturating_add` guard in `win_fill` and `win_text` stays out. No app can reach an overflow today: Nia passes
  32-bit values, and only built-in code runs at EL0. **The first loader of outside EL0 code must add it, with a test
  that feeds 64-bit values,** because pc/proc.rs passes full registers (`f.rcx`, `f.r8`, ...) straight to `win_fill`
  (the architect's condition 2; the same line goes in the nia-apps OS_REQUESTS, kept by the apps developer).

**How it is measured and proved:**
- A new shell verb, `fbcon wm`, in its own commit on the old code, so before and after use the same probe. It times
  8 rounds of:
  - `win_create` at 240x130 and at 40x20;
  - `win_present` with that one window, and its three parts on their own.

  It then runs two checks:
  - **clear:** draw into a window, reset, create it again, and every pixel of the new surface must be 0;
  - **match:** a hash of the whole screen after `win_present` with two windows, each with a known pattern drawn in
    (one 240x130, one 40x20, covering both the clipped and the unclipped chrome). The hash is fixed in the suite
    from the old code's run, so the new code must give exactly the same screen.

  Afterwards it resets the window server and redraws the console. It prints:
  - `[fbwm] win_create 240x130: fastest A us; 40x20: fastest B us; win_present: fastest C us (wallpaper D, text E, windows F)`
  - `[fbwm] clear check: 31200 of 31200 pixels zero; screen hash H`
- The `fbcon-time` suite types `fbcon wm` after `fbcon blit`, in the same two boots. On each kernel it grades:
  - `win_create` (both sizes) under 1000 us;
  - `win_present` under 1000 us;
  - the clear check exact;
  - the screen hash equal to the old code's hash for that kernel.

  The grader selftest gains cases for the new lines.
- Proof as for sections 18 and 19:
  - the cloud runner like-for-like md5 and `llvm-size`;
  - before and after numbers;
  - the desk, nia, wasm-*, persist-big and fbcon-time suites, and a full ARM walk;
  - one os2 Actions run.

The ARM image moves (tonga.rs, shell.rs).

**The architect's conditions (ARCHITECT_REVIEWS, 12 Nov) and what the measurement changed:**
- `fbcon wm` refuses with one line when a window is in use, the desk is shown, or the console is hidden, and leaves
  the window table alone. The suite proves it on ARM: after `wm` (3 app windows) and `clear`, `fbcon wm` must print
  `[fbwm] refused: 3 window(s) in use, desk hidden, console shown ...`.
- The deferred guard is recorded as above.
- The split on the old code (the cloud runner, 74d967b6): win_present was wallpaper 4113 us, text 1322 us, windows 358 us
  (ARM), and 4041, 1351, 328 us (PC). The fix, inside `tonga.rs` only:
  - the wallpaper is one `fill_rect` per colour band below the bar (about 95 bands), not one call and one `lerp`
    per row;
  - the bar and its text are drawn once and copied back after that, from a 70 KiB static (`BAR_PIXELS`).
- The `win_present` target becomes 3000 us (`WM_PRESENT_BUDGET_US`). It repaints the whole wallpaper, and a
  whole-screen blit on its own takes 0.8 to 1.3 ms on ARM under QEMU, so 1000 us cannot be met without damage
  tracking. The old code took about 6000 us, so the budget still catches a slide back. `win_create` keeps 1000 us.
