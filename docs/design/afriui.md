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

## 21. Fast window fill (12 Nov 2026) [law 0]

After section 20, `win_fill` is the slowest drawing call left in Tonga, and it is the one every Nia app draws with
(nia.rs `HOST_WIN_FILL`; pc/proc.rs too). It still writes one pixel at a time through an indexed loop with a bounds
check on each. A probe that was never pushed timed it on a cloud runner (5556f499, fastest of 8):

| call | ARM | PC |
|---|---|---|
| `win_fill` 240x130 | 5770 us | 6604 us |
| `win_fill` 40x20 | 154 us | 173 us |
| `win_text`, 10 glyphs | 190 us | 204 us |
| `fb::draw_string`, 26 characters at scale 1 | 697 us | 701 us |

Target: `win_fill` at both sizes under 1000 us at opt-level 0, on both kernels, with the same pixels as before.

**What changes (only `tonga.rs`):**
- `win_fill` clips the rectangle once, writes its first row, then copies that row into each row below with
  `core::ptr::copy_nonoverlapping` (memcpy from compiler_builtins, fast even in a debug build). The surface stride
  stays `CW`. The pixels written are exactly the ones the old loop wrote.
- The clip uses `saturating_add` (`rx + rw` and `ry + rh`), so a huge width or height from a 64-bit caller stops at
  the surface edge instead of wrapping. This closes the deferred guard of section 20 for `win_fill`; `win_text` is
  glyph-sized and keeps its own check.
- Nothing else changes. The callers and their arguments stay as they are.

**How it is measured and proved:**
- In its own commit, on the old code, `fbcon wm` gains:
  - the fill time, 8 rounds of `win_fill` over a whole 240x130 window and a whole 40x20 one;
  - a clip check: a fill that hangs past the right and bottom edges of a 240x130 window (from 200,100, size
    100x100) must colour exactly the 40 x 30 = 1200 pixels inside and leave the other 30,000 alone, and a fill with
    `usize::MAX` width and height from 230,120 must colour exactly the 10 x 10 = 100 pixels in the corner.

  It prints `[fbwm] win_fill 240x130: fastest A us; 40x20: fastest B us; clip check: C of 1200 inside, D of 100
  corner, rest unchanged`. On the old code the `usize::MAX` case wraps, so the probe runs it only after the fix;
  the old code prints `corner not run`.
- The `fbcon-time` suite grades on both kernels: both fills under 1000 us (`WM_BUDGET_US`), the clip check exact,
  and the existing screen hash `3962d25c879f188c` unchanged (it is drawn with `win_fill`, so it proves the same
  pixels). The grader selftest gains cases for the new line.
- Proof as for sections 19 and 20: the cloud runner like-for-like md5 and `llvm-size`, before and after numbers, a full ARM
  walk, one os2 Actions run.

The ARM image moves (tonga.rs). Rollback: git revert, or `backup.py rollback` on tonga.rs and fbcon-time-test.sh.

**The architect's conditions (ARCHITECT_REVIEWS, 12 Nov):**
- An empty clip returns before anything is written. The probe fills from the right edge (`CW`, 0) and with width 0;
  each must colour 0 pixels (`0 from empty fills`).
- `win_text` is closed in the same commit: `x + col` and `y + row` become `saturating_add`, since pc/proc.rs passes
  64-bit values. The probe writes two glyphs at `usize::MAX - 3` (once in x, once in y), which must colour 0 pixels
  (`0 from far text`). With this, the deferred guard of section 20 is closed for both calls.
- The `rest unchanged` check covers the whole CW*CH buffer.

The line becomes `[fbwm] win_fill 240x130: fastest A us; 40x20: fastest B us; clip check: 1200 of 1200 inside, 100
of 100 corner, 0 from empty fills, 0 from far text, rest unchanged`.

The probe on the old code (the cloud runner, cd8aeaa0): `win_fill` 240x130 took 4627 us on ARM and 4800 us on PC, 40x20 125
and 126 us; the clip check gave 1200 of 1200 inside with the rest unchanged; the screen hash stayed
`3962d25c879f188c`.

## 22. Tonga bar text and the screenshot pointer (12 Nov 2026) [law 0]

Two small things seen in the owner's screenshots of 12 Nov (master bdbfb5da):

- The Tonga status bar reads TONGA WM and APP SURFACES joined by a spaced double hyphen, which our text rules ban in
  anything a person reads. It becomes `TONGA WM: APP SURFACES`, drawn at the same place. Only that string in
  `tonga.rs` changes. The bar is part of the screen that `fbcon wm` hashes, so the expected hash in `fbcon-time`
  (`WM_HASH_ARM`, `WM_HASH_PC`, today `3962d25c879f188c`) changes once, to the value a the cloud runner run of the new code
  prints on each kernel. The run that sets it must also show the clear check and the clip check unchanged, so only
  the bar's pixels can have moved.
- In the PC screenshots at 1920x1080 the mouse pointer sits in the middle of the screen, over the Files list. The
  screenshot driver `scripts/shell-demo.py` gains `SHOT_MOUSE_REL=dx,dy`: before the screendump it moves the mouse
  by that much through QMP (`input-send-event`, relative), and the desk clamps the pointer to the screen.
  `scripts/desk-shots.sh` sets it for the PC scenes so that the pointer ends in the bottom right corner, where no
  window is. No kernel change; no suite uses the variable, so no suite changes.

Proof: the cloud runner `fbcon-time` on the new code (the new hash, the other `fbwm` lines as before), md5 and `llvm-size`
like for like, the screenshots taken again in the cloud and looked at, and one os2 run. The ARM image moves by
the one string.

Done (12 Nov, the cloud runner afriui-bar-job on 578f8757): the screen hash after `fbcon wm` moved from `3962d25c879f188c` to
`d6da56a7c9f713a4`, the same on ARM and on PC, and `fbcon-time` now expects the new value on both. The same run
shows the clear check (31200 of 31200 zero) and the clip check (1200 of 1200 inside, 100 of 100 corner, 0 from
empty fills, 0 from far text, rest unchanged) as before, so only the bar's pixels moved. The new label is 4
characters shorter and still starts at W - 232, so its right edge sits 32 px further left than before. That is
intended: the label keeps its left edge, as the other bar text does, and nothing is drawn to its right. The
pointer step moves the mouse 4000,4000 and the desk clamps it to the bottom right corner, off every window; the
portrait scene adds a second step of 0,-200 so the pointer cannot land on the bar there.

## 23. Faster text: draw_string and draw_char (12 Nov 2026) [law 0]

`fb::draw_string` and `fb::draw_char` draw the bar text in Tonga (every `win_present`), the store, the Nia web
page, the boot splash and the PC console back buffer. They call `fill_rect` once for every lit pixel and, in
`draw_string`, read each glyph's 64 bits twice (once to measure its ink, once to draw). The probe in commits
85b4f11f and 79a96c70 (`fbcon blit`, new `[fbtext]` lines) timed the old code on a cloud runner, fastest of 8:

| call | ARM | PC |
|---|---|---|
| `draw_string`, A to Z at scale 1 | 852 us | 1043 us |
| `draw_string`, A to Z at scale 2 | 1396 us | 1471 us |
| `draw_char` x 26 at scale 1 | 593 us | 741 us |

The same run on the old code: screen hash `146c9edc133e9276` after sample text at scales 1 to 4 (both kernels), end
x 1003 (the sum of what the `draw_string` calls return), 90 pixels lit in the bottom right corner by text hanging
off it and the rest of the screen unchanged.

Target: each of the three under 300 us on both kernels, at opt-level 0, with the same pixels and the same return
values. (Not reached; see "Result" below. The budget is 500 us.)

**What changes (only `framebuffer.rs`):**
- The ink of a glyph comes from one OR of its 8 rows: the left edge is the OR's leading zeros, the right edge is 7
  minus its trailing zeros. Same values as the old double loop; a blank glyph still moves on 4 * scale.
- Each glyph row is drawn as runs: every run of lit columns is one `fill_rect` of run * scale by scale, not one call
  per pixel. `fill_rect` still does all clipping, so the pixels written are exactly the old ones.
- Positions use `saturating_add` and `saturating_mul`, so a start near `usize::MAX` stops at the screen edge
  instead of overflowing (a panic in a debug build). Today every caller passes small kernel constants, so this is a
  guard, not a fix of a live bug. The return value saturates the same way.
- Nothing else changes: the callers, their arguments and the font stay as they are.

**How it is proved:**
- `fbcon-time` grades the `[fbtext]` check line exactly: hash `146c9edc133e9276`, end x 1003, 90 in the corner, rest
  unchanged, far text 0. The far text case (`draw_string` and `draw_char` from `usize::MAX - 3` in x and in y) runs
  only once the fix turns `SATURATING_TEXT` on; the old code would panic there. Grader selftest cases for a wrong
  hash, a wrong end x, a wrong corner count, a changed rest, far text lit and a missing line.
- The three times graded under `TEXT_BUDGET_US` (default 500) in the fix commit.
- the cloud runner: `fbcon-time` before and after, then `fbcon-time`, `desk`, `pc-desk` and the Tonga screen hash
  (`d6da56a7c9f713a4`, the `fbcon wm` line) unchanged; ARM md5 and `llvm-size` like for like; one os2 run.
- The ARM image moves (framebuffer.rs).

**Result (12 and 13 Nov, the cloud runner, fastest of 8, ARM / PC):**

| version | A to Z, scale 1 | A to Z, scale 2 | draw_char x 26 |
|---|---|---|---|
| old code | 852 / 1043 us | 1396 / 1471 us | 593 / 741 us |
| one `fill_rect` per run (d879cd8e) | 331 / 327 us | 765 / 611 us | 299 / 291 us |
| runs written straight in when on screen (a3ae8454) | 269 / 273 us | 358 / 363 us | 237 / 237 us |
| while loops and a row copy per run (tried, not kept) | 256 / 234 us | 406 / 343 us | 275 / 256 us |

All three versions gave the old code's text check exactly (hash `146c9edc133e9276`, end x 1003, 90 in the corner,
rest unchanged) and far text 0, and the Tonga hash stayed `d6da56a7c9f713a4`. What is left is a fixed cost of about 9
us per glyph (the glyph lookup and eight row calls in a debug build under emulation); the third version did not
move it, so the branch keeps the second, which is simpler. A change from one `fill_rect` per run to direct writes
is a change from the plan as first sent: when a glyph row is wholly on the screen, its runs are written straight
into the framebuffer; when not, each run is still one clipping `fill_rect`. The budget is 500 us, under every old
time, so a slide back fails the suite.
## 24. Cell windows for apps (13 Nov 2026) [law 0]

The native apps engineer's editor, Andika, is a Nia app that needs a desk window made of character cells. The
shared notes call this "D1"; here it is "cell windows", because D1 is already the name of the write-combining work
in AFRIUI_HANDOVER.md. Agreed with the native apps engineer on 12 Nov: a cell is 2 bytes, a character and an
attribute (foreground role in the low 4 bits, background role in the high 4 bits, background 0 = the window body);
16 colour roles, not RGB; characters 32 to 126, 0x80 the box for a letter the font lacks, 0x81 to 0x8F kept for UI
glyphs; calls to put a run of cells, set the cursor and read the grid size; the desk redraws only the rows a put
touched; keys from 0x100 up with SHIFT 0x1000, CTRL 0x2000, ALT 0x4000; a close request with a 3 s wait.

**What the code has today** (read on master f9e9d1ba):
- No app draws on the desk. The desk has four built-in window kinds (`App` in desk.rs, `NAPPS = 4`), each drawn by
  kernel code; Nia apps and EL0 apps draw only in Tonga, a separate full screen.
- A Nia app runs to completion inside one shell command. The desk draws only from `idle()`, which runs while the
  shell waits for a byte, so while an app runs nothing on the desk moves.
- Nia has no event queue: a 16-byte console ring, `getkey` and `pollkey`. No resize or close events.
- On the PC, printable keys go to the serial ring (`uart::inject`) and only window shortcuts reach the desk;
  ARM has serial input only.
- The Terminal window is already a grid in all but name: 6 x 13 pixel cells, glyphs from the Sovereign face.

So the work comes in four stages, each its own branch, plan, proof and merge.

**Stage 1, the cell window on the desk (desk.rs and paint.rs only; this branch):**
- A fifth window kind, `App::Cell`, one cell window at a time in this stage. Its grid lives in the desk: at most
  120 columns by 50 rows of 2-byte cells (12,000 bytes in .bss; the ARM image moves).
- Cells of 6 x 13 pixels at UI scale 1, as the Terminal; glyphs from the Sovereign face. 0x80 and every byte outside
  32 to 126 draw as a hollow box the size of a capital; 0x81 to 0x8F draw as the box until they get glyphs.
- A table of 16 roles in paint.rs: text, dim, keyword, string, comment, number, punct, gutter, gutter-current,
  selection, status, status-dim, cursor-line, error, ok, accent. Values from the desk's own colours and the
  colours sent on 12 Nov (keyword gold #F2C14E, string malachite #2E9E6A, comment #B0A391, number ochre #D08A2E,
  punct terracotta #C4562B); background role 0 is the Terminal's body colour. The contrast check is stage 4.
- The grid size follows the window: columns = (body width - 16) / 6, rows = (body height - 12) / 13, capped at 120
  by 50. On a resize the cells keep their place from the top left; what no longer fits is not drawn. The new size
  is kept for the resize event of stage 2.
- `put(col, row, cells)` writes a run of cells along one row (it does not wrap), clipped to the grid, and damages
  the rectangle of that one row inside the window body. `cursor(col, row, shape)` with shape 0 hidden, 1 bar, 2
  block, 3 underline, drawn in gold by the desk; moving it damages the old cell and the new one.
- Close is immediate in stage 1 (no app owns the window yet).
- Shell verbs for tests: `desk cell open`, `desk cell put <col> <row> <attr hex> <text>`, `desk cell cursor <col>
  <row> <shape>`, `desk cell size`, `desk cell demo` (an editor-like page using all 16 roles, the box and the
  cursor). They go through the same functions an app will call.

**Proof for stage 1:**
- A new suite `desk-cell`, on both kernels: the size line for the default window; after `demo` and a few puts, a
  hash of the grid bytes equal to the expected one; `desk verify` 0 rows differ (the partial frames match a full
  redraw); a put on one row copies only that row's band to the screen (the `desk perf` rows count); a put past the
  grid edge is clipped and changes nothing outside; bytes outside 32 to 126 draw the box. Grader selftest cases.
- the cloud runner: `desk-cell`, `desk`, `pc-desk`, `fbcon-time`; screenshots of the demo at 1280x800 and 1920x1080, looked
  at; ARM md5 and `llvm-size` like for like; one os2 run.

**Stage 2, the Nia calls (nia.rs, the apps developer's file; the ABI goes to the architect first):**
- `cell_open(min_cols, min_rows) -> win`, `cell_put(win, col, row, ptr, len) -> cells written`, `cell_cursor(win,
  col, row, shape)`, `cell_size(win) -> cols << 14 | rows`, `cell_event(timeout_ms) -> event`. Pointers checked
  with `mem_range`, lengths capped at the grid width, under the draw capability.
- Events as one i32: kind in bits 28 to 31 (1 key, 2 resize, 3 close request), the rest the key code with its
  modifier bits, or columns << 14 | rows for a resize (the same packing as `cell_size`, so one decoder reads both).
  0 means no event before the timeout. `cell_event(0)` returns at once; `cell_event(-1)` waits with no limit.
- A queue of at least 32 events per cell window, so fast typing or a paste is not lost while the app redraws.
- The window closes when the app's run returns or traps; there is no `cell_close` call.
- Andika on wasm opens a 64 x 24 grid (`cell_open(64, 24)`; the whole app shares one 64 KiB page); a bigger window
  shows the body colour past it.
- `cell_event` runs the desk's `idle()` while it waits, so the desk keeps drawing and taking input while the app
  runs. That is the one new link between the two.

**Stage 3, keys and close:** while a cell window has the focus, the PC keyboard drivers (ps2.rs, xhci.rs) post every
key to the desk with the codes above instead of the serial ring, and the desk passes them to the app. Plain keys keep
the serial line's values: printable bytes as themselves, Enter 13, Tab 9, Backspace 8, Esc 27, Ctrl and a letter 1
to 26 (Ctrl+S is 19); modifier bits add to those (Ctrl+Shift+S is 19 | SHIFT, Ctrl+Tab is 9 | CTRL, Ctrl+PgDn is
0x107 | CTRL). The largest code, 0x11B with all three modifiers, is 0x711B. Alt+F4 and the close box send a close
request, and pressing them again sends another; the app answers with its own prompt (Andika asks to save). If the
window is still open 3 s after a request, the title bar offers "Close anyway", which closes only when the person
clicks it, never on its own. On ARM the serial bytes map to the same codes.

**Stage 4:** an Andika tile in the Circle, "open with Andika" later, and the design review of the 16 colours
for contrast.

The native apps engineer checked this section on 13 Nov: the roles are in Andika's order (text 0 to accent 15), the
key codes fit, and 120 x 50 is fine; the queue, the wait values, the plain key values, the close rule, the window
end and the `<< 14` packing above come from that check.

Questions for the architect: 12 KB more .bss for the grid; a fifth `App` kind rather than a separate path (it
reuses the window manager, damage and verify as they are); and the stage 2 event encoding.

**Stage 1 done (13 Nov, the cloud runner on 3e5a00fd), with the architect's conditions:**
- `NWIN = 5` for window state and `NTILES = 4` for the Spine, the Circle, the keyboard walk and Super+1..4: no Cell
  tile anywhere yet, and `desk` and `pc-desk` pass unchanged.
- The grid is a fixed static array (12,000 bytes); `put` is bounded by `cols - col`, so a column or row of
  `usize::MAX` writes 0 cells; the body subtractions saturate, so the smallest window (40 x 30) gives a 3 x 0 grid.
- The suite `desk-cell` passes on both kernels: a 65 x 24 grid at the home size, the demo's grid hash
  `1c6f918fc2716b05` and the hash after the puts `7a8d0bf5c19f47b4` (the same on ARM and PC), a put redraws 13 rows
  (one row band), a cursor move 26 (its two cells), the clip at the grid's edge, 0x80, 0x01, 0xC3 and 0x7F drawing
  the same box and a letter not, and `desk verify` 0 rows differ four times, after partial frames, including after a
  resize and in the 3 x 0 grid.
- One change from the plan: the suite counts the rows the desk redrew, not the canvas rows copied to the screen,
  because the ARM desk draws straight into its framebuffer and makes no copy to count. The count is the same on
  both kernels.


## 25. Cell windows, stage 2: the Nia calls (13 Nov 2026) [law 0]

Stage 1 is on master (dd31b40a). This stage lets a Nia app own the cell window: open it, draw cells, set the
cursor, read the size and wait for events, with the four notes from the architect's verdict on section 24 (no
re-entry, a forced close the interpreter obeys, calls after close, a lock-free key queue). nia.rs is the apps
developer's file, so the split of the work is agreed with them before any code.

**What the code has today** (read on master dd31b40a):
- `run_app_in` (nia.rs) runs an app's `run` to its end inside one shell command, on the kernel stack. Nothing
  stops a second call to it while one app runs.
- `idle()` (desk.rs) runs only from the shell's `cpu_getc` wait, so the desk is frozen while an app runs. `idle()`
  itself starts no app and no shell command today: Circle tiles open the four built-in windows, and Terminal keys
  go to the serial ring, which the blocked shell does not read.
- Every branch back to a `loop` label calls `loop_poll()`, which reads the console once per 4096 branches
  (`console_key`): Ctrl+C traps, other bytes go to a 16-byte key ring. `pollkey` and `sleep_ms` read the console
  through the same function; `getkey` spins on `uart::getc`.
- Interrupts post desk input to `EVQ` (head and tail atomics, 28-bit payloads); `idle()` drains it in mainline.
- Host ids 0 to 16 are taken; a module may import 16 functions.

**The ABI (one copy, `kernel/src/afriui/cell_abi.rs`, used by nia.rs and desk.rs):**
- Five imports from `env`, all under the draw capability (`CAP_DRAW`, which sideloaded apps keep):
  - `cell_open(min_cols, min_rows) -> win`. Opens the cell window at its home size, or grows it to fit the
    minimum when the screen allows, and gives it the focus. Returns 1 (one cell window per app in this stage).
    If a test window from `desk cell open` is showing, the app takes it over and the grid is cleared.
  - `cell_put(win, col, row, ptr, len) -> cells written`. `len` is in cells and capped at 120 before it is
    doubled to bytes, then `mem_range(ptr, 2 * len)`; the desk's `cell_put` clips as in stage 1.
  - `cell_cursor(win, col, row, shape) -> 0`. Shapes 0 to 3 as in stage 1; any other shape is refused (-5).
  - `cell_size(win) -> cols << 14 | rows`.
  - `cell_event(timeout_ms) -> event`. 0 returns at once; any negative value waits with no limit (treated as -1,
    never cast to a long u64); a positive value waits up to that many milliseconds. Returns 0 when the time runs
    out with no event.
- Events are positive i32s: kind in bits 28 to 30, payload in bits 0 to 27, bit 31 always clear. Kinds: 1 key (the
  code with its modifier bits, as section 24), 2 resize (`cols << 14 | rows`, the `cell_size` packing), 3 close
  request, 4 closed (the window is gone). 5 to 7 are kept.
- Errors are negative, as elsewhere in Nia: -1 bad window (not the handle `cell_open` gave), -2 no window (closed,
  or never opened), -3 already open, -4 no desk (the desk is not showing, so there is nowhere to draw), -5 bad
  argument.
- The constants and the two pack functions (`event(kind, payload)` and `grid(cols, rows)`) live in `cell_abi.rs`
  only. nia.rs and desk.rs both import them, so there are not two copies to drift.

**The desk side (desk.rs, paint.rs; mine):**
- The cell window gets an owner: none (the stage 1 test window) or the running app. It is closed and its queue
  emptied when the app's run returns or traps (`run_app_in` calls `desk::cell_app_ended()` on both paths).
- **The event queue:** 32 slots of `AtomicU32` with head and tail atomics, like `EVQ`. When it is full the newest
  event is dropped and counted (`desk cell size` prints the count), never the oldest overwritten. In this stage
  every producer runs in mainline: `idle()` posts resize and close events and the serial keys, and in stage 3 the
  ps2 and xhci interrupt paths will keep posting to `EVQ` (already interrupt-safe), from which `idle()` moves keys
  into the cell queue. The queue is still built on atomics, so an interrupt producer could post to it directly.
- **Resize:** a drag or `desk cell resize` that changes the grid posts one resize event with the new size;
  consecutive resizes in one `idle()` pass collapse into the last.
- **Close:** the close box and Alt+F4 post a close request (pressing again posts another). The window stays until
  the app's run ends, and only that answers a request: an app that takes the request from its queue and carries on
  has not answered it. If the window is still open 3 s after the first request, the title bar shows "Close
  anyway". Only a click on it closes the window: the grid goes, the queue is emptied, and the forced-close flag is
  set (below).
- **Keeping the desk alive while an app runs:** `cell_event` runs `idle()` and then `wfi` in its wait loop, as the
  shell's `cpu_getc` does (the 100 Hz timer wakes it). An app stuck in a loop never calls `cell_event`, so the
  interpreter also calls `desk::app_tick()` from `loop_poll` (once per 4096 backward branches): it runs `idle()`
  at most once per 10 ms, and only while the app owns a cell window. That is how the "Close anyway" click is seen
  at all when the app is stuck.
- **Test verb:** `desk cell plan <step>;<step>...` stores up to 8 steps, which `idle()` applies one per pass while
  an app owns the window (the shell is blocked then, so a suite cannot type desk verbs): `resize W H`, `close` (a
  close request, as the close box), `wait N` (passes N ms), `force` (the click on "Close anyway"; refused before the
  3 s), `run NAME` (tries to start another Nia app from inside the desk, for the re-entry test), `verify` (`desk
  verify`). Each step prints one line.

**The Nia side (nia.rs; the apps developer's file):**
- Host ids 17 to 21 for the five calls, in `host_cap` and the import-name match, all needing `CAP_DRAW`. Each calls
  the desk through `cell_abi.rs` and returns its result unchanged.
- **No re-entry:** an `APP_RUNNING` atomic, set by `run_app_in` for the whole run and cleared on every way out.
  `run_app_in` refuses at once when it is set ("[nia] REFUSED 'x': an app is already running") and touches
  nothing: not the key ring, not linear memory, not the tables on the running app's stack. Because every path that
  starts an app (the shell's `run`, appget, and later a Circle tile) goes through `run_app_in`, the one check covers
  them all. `crate::nia::app_running()` is public so stage 4's Andika tile can grey itself. The test: plan step `run
  celltest.wasm` from inside `cell_event` prints the refusal, and the running app then ends cleanly with its grid
  hash unchanged.
- **Forced close:** `loop_poll` reads the desk's forced-close flag on every backward branch (one relaxed atomic
  load, before the 4096 count) and traps with "closed by the person (Close anyway)". `getkey` and `sleep_ms` check
  it in their waits too, so no host call can hold the app past it. Endless recursion is already ended by the
  16-level depth limit.
- **Calls after close:** between the click and the trap the app may still make host calls. Every `cell_*` call then
  returns -2, and `cell_event` returns kind 4 (closed) at once, whatever the timeout. A `cell_event` waiting when
  the click comes returns kind 4, so an app whose loop ends on it returns cleanly and does not trap; one that loops
  on traps at its next backward branch.
- **Serial keys while an app owns a cell window:** `console_key` passes each byte to the cell queue as a key event
  instead of the 16-byte ring, so `getkey` and `pollkey` see nothing while the window is open. Ctrl+C is a key (3)
  for the app, as section 24 says for every Ctrl+letter, and does not stop it. On the serial line Ctrl+\ (0x1C) is
  the close request, and a Ctrl+\ 3 s or more after an unanswered request is the "Close anyway" click, since a
  serial terminal has no title bar; it is still the person's own act, never a timer. This is the stage 2 key path on
  both kernels; stage 3 adds the PC keyboard with modifier bits.
- **Stack:** `cell_event` and `app_tick` run the desk's drawing on the app's kernel stack. The suite measures the
  stack left with `idle()` called from 16 call levels deep, as `wasmfuzz` does for the interpreter, and the plan
  fails if it is under 16 KiB.

**A test app:** `celltest.wasm` in nia-apps (the apps developer's tree), asking only for `draw`: it opens 64 x 24,
puts a page using all 16 roles, then loops on `cell_event(-1)` and prints each event. A key `s` puts it in an
endless loop with no host calls (for the forced close); on kind 4 it calls `cell_put`, `cell_size` and `cell_event(0)`
once each, prints the three results and returns; on a close request it returns.

**Proof:**
- A suite `desk-cell-app` on both kernels, with grader selftest cases: the size line; the grid hash after the app's
  page (expected the same on ARM and PC); serial keys arriving as kind 1 with the right codes, Ctrl+C as 3 and the
  app still running; `desk verify` 0 rows differ while the app waits; a resize step posting one kind 2 event with
  the new size; a close request ending the run cleanly and the window gone; `force` before 3 s refused and after it
  accepted; the forced close of the spinning app trapping within 100 ms of the click; the after-close results -2,
  -2 and kind 4; the nested `run` refused; `cell_event(-5)` waiting like -1 (it returns on the next key, not at
  once); 33 keys sent while the app sleeps giving 32 events and a drop count of 1; the stack line.
- the cloud runner: `desk-cell-app`, `desk-cell`, `desk`, `pc-desk`, `fbcon-time`, and the Nia suites that touch `loop_poll`
  and `console_key` (`wasmfuzz` and the key and loop suites the apps developer names), so Ctrl+C still stops an app
  with no cell window. Screenshots of the app's page and of "Close anyway" on both kernels, looked at. ARM md5 and
  `llvm-size` like for like (the image moves). One os2 run.

**Questions for the architect:**
1. The interpreter calling `desk::app_tick()` from `loop_poll`, so the desk draws (at most every 10 ms) from inside a
   stuck app. It is the only way the "Close anyway" click can be seen, but it puts desk drawing under the
   interpreter's frames.
2. Ctrl+C as an app key while a cell window is open, with Ctrl+\ as the serial close request and close anyway.
3. Errors -1 to -5 and the closed kind 4 as above.

The native apps engineer checked this section on 13 Nov: Ctrl+C as key 3 is what Andika wants (it is Andika's Copy),
Ctrl+\ is free, Andika's loop ends on kind 4 and on any error -1 to -5, and on the serial line it reads arrows as the
raw `ESC [ A` bytes, one key event each, calling `cell_event(50)` while a sequence is half read so a lone Esc works.

The apps developer checked this section on 13 Nov and takes the nia.rs pieces and `celltest` (with its `tools/nia.py`
stubs, `draw` permissions file and tests.json entry) as commits on this branch once the desk side is in. What
changes from the text above:
- **Host ids:** the five calls keep 17 to 21; the apps developer's `win_text_run` (plan A1) moves to 22. Whichever
  lands second rebases.
- **The re-entry check sits first in `run_file` too**, before `vfs::read_full`, the integrity lines and
  `tonga::wm_reset()`, so a refused nested `run` prints only its refusal and never resets the running app's windows.
  `run_app_in` keeps its own check for the built-in demos.
- **`getkey` and `pollkey` while the app owns a cell window:** `getkey` returns -2 at once (no window for that call,
  so the app sees it should use `cell_event`), and `pollkey` returns -1. `getkey`'s wait with no cell window now goes
  through `console_key`, the forced-close check and `app_tick`, as `sleep_ms` does.
- **`sleep_ms`** calls `app_tick` beside its forced-close check, so the desk does not freeze for up to 10 s.
- **The 33 keys:** both serial lines are fed by their receive interrupts into 256-byte rings (the ARM PL011 and the
  PC's COM1 after `enable_rx_irq`), so 33 bytes sent at once all reach the ring and the one drop is the cell queue's.
- **Order:** the apps developer's `claude/nia-ctrl` (loop label arity and `else` in a loop, next to `loop_poll`) lands
  first, then these nia.rs pieces on top. If plan A1 lands first, the 16 KiB stack line is measured after it.

**Stage 2 done (13 Nov, `claude/nia-cell` at 50396a8a on master 79dd1171), with the architect's conditions:**
- Desk side (afri-desktop): `cell_abi.rs` is the one copy of the ABI; the app's cell window with its 32-slot atomic
  queue (the newest event dropped and counted when full); close requests from the close box, Alt+F4 and Ctrl+\ on
  the serial line; "Close anyway" 3 s after the first request; `app_tick` so a stuck app cannot freeze the desk; the
  app's file name and an "unsigned" mark drawn by the desk in the title; the `desk cell plan` verb for the suite.
- Nia side (the apps developer): the five calls on host ids 17 to 21, the re-entry check first in `run_file` and in
  `run_app_in`, the forced flag read before the 4096-branch count, `getkey` -2 and `pollkey` -1 while the app owns a
  cell window, `sleep_ms` and `getkey` calling `app_tick`; the test app `celltest`, made by
  `scripts/lib/celltest-wasm.py`.
- One fix from the first real run: taking a close request from the queue no longer answers it. Only the end of the
  app's run does, so an app that ignores requests still gets "Close anyway" 3 s after the first one (the text above
  says so now).
- The suite `desk-cell-app` passes on both kernels (the cloud runner and os2): the full key and event list, 1 event dropped of
  33, the nested `run` refused with the grid hash `52feb47e95e1ac19` the same before and after (and the same on ARM
  and PC), two Ctrl+\ only requests and the third the force, the app stopping 9 ms (ARM) and 13 ms (PC) after Close
  anyway, the cell stack line at depth 16 with 854,000 bytes left on ARM and 1,901,080 on PC (well above 16 KiB).
  The screenshots show the title `celltest.wasm` with the unsigned mark, and the red Close anyway, on both kernels.
- the cloud runner on 50396a8a: ARM 25 of 25 (desk-cell-app, desk-cell, desk, fbcon-time, the wasm set, smoke, heap, appget,
  stackwater), PC 4 of 4. os2 run 43459814448: desk-cell-app, desk-cell, desk, pc-desk, fbcon-time, wasm-fuzz,
  smoke, 7 of 7 PASS, PC md5 `3d54f5611557b321bfff78404fbbc788`.
- ARM image, the cloud runner like for like on one machine and path: master 79dd1171 `2f9cafb1dc836f52e44b06df2bab14d8`
  (text 2185124, data 1840, bss 30787120) to 50396a8a `7c3318bfdde89f2df8765e51153120e2` (text 2207944, data 1848,
  bss 30787736); a rebuild repeats it.

## 26. The premium look, step 1: smooth text that scales (13 Nov 2026) [law 0]

The owner said yes on 13 Nov (by phone, through the lead) to a premium look, in five steps in this order: smooth
fonts that scale, rounded corners with soft shadows and see-through panels, scaling for high-resolution screens,
smooth motion, and one design system. Each step is measured, keeps this week's speed, goes to the architect as a
plan first, and is judged by before and after screenshots from the cloud. This is step 1. Cell windows stage 2
(section 25) waits for the architect meanwhile; whichever plan comes back first is built first.

**What the code has today** (read on master dd31b40a):
- The desk's text is the Sovereign face (section 16): our own bitmap font, 95 ASCII glyphs at most 5 pixels wide on
  12 rows, one bit a dot. `paint.rs` draws it with `text` (proportional), `text_w` and `mono` (6-pixel cells for the
  Terminal and cell windows). Larger text is the same dots doubled (`scale` 2), so at 1920x1080 every letter is
  made of 2 x 2 blocks. Bytes outside 32 to 126 draw as "?" or the box.
- Every pixel is either ink or not: no edge is smoothed, and nothing in paint.rs reads a pixel back to blend it.
- The text console, Tonga, the web view and the store use the older 8 x 8 font in framebuffer.rs (`draw_string`,
  made fast this week, section 23). They are not part of this step.
- The ARM desk draws straight into the ramfb framebuffer (no second buffer, step P2); the PC desk draws into a
  canvas in RAM and copies what changed to the screen.
- The kernel has no font files, no outline code and no floating point in its drawing. ADR 0007 allows third-party
  crates only for cryptography, so the rasterizer has to be our own code.

**The plan:**
- **Outlines, not dots.** Each glyph is kept as its outline: closed curves of straight lines and quadratic curves
  (the TrueType kind), in font units, as `i16` points in a compact table in `kernel/src/afriui/outline_data.rs`.
  The table is made once, offline, by a new tool `tools/face/outline_src.py` (fontTools, run in the cloud, never on
  this PC) and committed. The kernel parses no font files at run time, so a font is never something an app or a
  disk can hand it.
- **Our own rasterizer** (`kernel/src/afriui/vector.rs`, about 300 lines): the curves are split into short lines,
  each line adds its signed area to a coverage buffer, and one running sum turns that into 256 levels of coverage
  per pixel. All in integer fixed point (no floats), so the same glyph gives the same bytes on ARM and PC and the
  grid hashes stay comparable across the two kernels.
- **Sharp at small sizes without hinting:** for each size the vertical scale is bent slightly so that the baseline,
  the x-height and the cap height each land on a whole pixel. Horizontal strokes (the bars of e, t, H and the
  baseline) then sit in one pixel row and look crisp at 13 pixels; the sides of letters keep their smoothing.
- **Light text on the dark desk:** coverage goes through a small gamma table (one per text colour brightness) so
  that light-on-charcoal text does not look thin, the usual problem with smoothed text on a dark ground.
- **A glyph cache:** a glyph is rasterized once per size and kept, as coverage bytes, in a fixed 256 KiB static
  arena (.bss, no heap), keyed by glyph, pixel size and the pen's quarter-pixel position. When the arena is full it
  is emptied and refilled. Drawing a cached glyph is a copy with blending: coverage 0 is skipped, 255 is a plain
  write, and only the edge pixels read the pixel below and mix.
- **Sizes that scale:** text is asked for by pixel size, not by doubling. The desk's body text is about 13 pixels
  at UI scale 1 (capitals 9 pixels, as today) and twice that at scale 2, with all its detail. Any size in between
  works, which step 3 (high-resolution scaling) will use for 1.25x and 1.5x.
- **UTF-8 and African letters:** `text` decodes UTF-8. Besides ASCII the table carries the letters of the languages
  the OS is meant for: Akan and Ewe (ɛ Ɛ ɔ Ɔ ŋ Ŋ ʋ ƒ), Yoruba and Igbo (ẹ ọ ṣ ị ụ with their tone marks), Hausa (ɓ ɗ
  ƙ ƴ), Swahili and Zulu (plain Latin), and Latin-1 for French and Portuguese names. A character not in the table
  draws as the box. About 300 glyphs.
- **Which text changes in step 1:** the desk's proportional text everywhere on both kernels: window titles, the
  Spine, the Circle, Settings, About, Files, notices and the bar. The Terminal and cell windows keep the bitmap face
  in their 6 x 13 cells for now: a monospace outline font at 6 pixels a column is too small to read well, and
  changing the cell size changes every grid. They move at step 3 with the larger cells that scaling brings. The
  text console, Tonga, web and store follow later, each with its own plan.
- **A switch for the shots:** `desk font vector` and `desk font bitmap` choose the face at run time, so the before
  and after screenshots come from one build and one scene, and a problem found later can be set aside with one
  command. Vector is the default.

**The font itself (a decision for the architect, and for the owner by screenshot):**
- My recommendation: **Noto Sans** (Google's open font family, made to cover every writing system; SIL Open Font
  Licence 1.1, which lets us embed and ship it, including changed copies under another name). It already has
  every African letter above, drawn by type designers, which we could not match by hand in a reasonable time. The
  outlines are taken from one pinned release file, checked by its SHA-256 in the tool, and the licence goes in
  THIRD_PARTY.md under a new Fonts section. Fonts are data, not crates, so ADR 0007 is not touched, but the
  architect should say whether OFL-1.1 joins the allowed licences.
- The other way: draw the Sovereign face itself as outlines. It keeps a face that is only ours, but drawing about
  300 glyphs, with the African letters, to a quality that holds up at 26 pixels is weeks of design work. It could
  come later as a display face for the brand (the lock screen, the About title) on top of Noto Sans.

**What it costs, measured before it ships:**
- **Text speed:** the `[fbtext]` probe gets two more lines for the desk's text: the A-to-Z line and a 40-character
  window title, bitmap against vector, at 13 and 26 pixels, cache cold (first draw) and warm. The rule: warm vector
  text stays within the 500 us text budget of section 23 on both kernels; the cold cost is printed and only paid
  once per glyph and size.
- **Frames:** `desk perf` for a full frame and for a title-bar redraw at 1280x800 and 1920x1080, before and after.
  The rule: a full frame no more than 15% slower than today, and partial frames (one window's damage) about the
  same as today.
- **Size:** the outline table (about 60 KB of .rodata), the rasterizer code, and the 256 KiB arena in .bss. ARM md5
  and `llvm-size` like for like; the ARM image moves, which this plan asks the architect to agree to.
- **ARM reads:** blending reads the pixels it mixes. On the ARM desk those are framebuffer pixels. In QEMU the
  ramfb is plain RAM, but on a real phone panel reading the scan-out buffer is slow. Only edge pixels are read
  (about a third of a glyph's pixels at 13 pixels), and P3's back buffer (afriui-phone-plan.md) removes the read
  from the scan-out buffer entirely. Recorded here as a known cost, measured on QEMU.

**Proof:**
- A suite `desk-font` on both kernels with grader selftest cases: a hash of the rasterized coverage for a fixed
  word at 13 and 26 pixels (the same on ARM and PC); a glyph drawn at x and at x + a quarter pixel differing only in
  its edge pixels; the box for a character not in the table; UTF-8 decoding of `ɛ ɔ ŋ ẹ ọ ṣ` with a bad byte
  sequence drawing one box, never reading past the string; the cache filling, emptying and drawing the same pixels
  after; `desk verify` 0 rows differ after partial frames with vector text; text clipped at a window edge drawing
  nothing outside it.
- `desk`, `pc-desk`, `desk-cell` and `fbcon-time` pass, with their screen hashes updated where the text changed (the
  pixel-row checks that do not involve text stay as they are).
- the cloud runner on both kernels; screenshots, bitmap then vector, of the same scenes at 1280x800 and 1920x1080 (the
  desktop, Settings, the Circle, notices, the phone layout), looked at closely, and sent to the lead for the owner.
  One os2 run.

**Questions for the architect:**
1. Noto Sans under OFL-1.1, from one pinned file, the outlines committed as a generated table, and a Fonts section
   in THIRD_PARTY.md.
2. Our own integer rasterizer and a 256 KiB static glyph arena, so the ARM image grows (about 60 KB .rodata and the
   code, plus the .bss).
3. Step 1 limited to the desk's proportional text, with the Terminal and cell windows keeping the bitmap face until
   step 3.
4. The budgets: warm text within 500 us, a full frame no more than 15% slower, partial frames about the same.

**Step 1 built (13 Nov, `claude/afriui-fonts` at a60a791e on master 2fc6134b), with the architect's conditions:**
- **The font:** Noto Sans v2.015 Regular (unhinted TrueType, sha256 `f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8`, pinned in
  `tools/face/outline_src.py`), 381 characters in 393 glyphs, 5136 points, 369 components (an accented letter is its
  base and its mark), about 35 KB of tables in `kernel/src/afriui/outline_data.rs`. The tool ran on a cloud runner; no
  character asked for is missing. The file keeps the OFL (its header says so), the OFL text is `fonts/OFL.txt`, the
  release has no Reserved Font Name, and the OS calls the face "AfriSovera Sans". THIRD_PARTY.md has a Fonts section
  and ADR 0005 a line: OFL-1.1 for fonts only.
- **The rasterizer** (`kernel/src/afriui/vector.rs`): integer, 1/256 pixel, FreeType-style cover and area cells one
  row at a time (so only a row of cells and 2048 lines are kept, not a whole bitmap); quadratic curves cut into at
  most 16 lines; the x-height and cap height bent onto whole pixels; quarter-pixel placement; sizes clamped at the one
  entry point to cap heights 4 to 45 (about 6 to 63 pixels an em), the cell row sized for 128. The 256 KiB arena and a
  1024-slot table empty themselves when full, before a new glyph is written. Lookup is a checked read for ASCII and a
  binary search otherwise; UTF-8 decoding never reads past the string and gives one box for a bad sequence.
- **Drawing:** `text` and `text_w` in paint.rs draw the outline face; the bitmap face stays behind `desk font bitmap`.
  Both canvases got a `blend` that mixes coverage through a gamma table (light text on the dark desk a little
  heavier, dark text on a chip a little lighter); 0 is skipped and 255 written plainly, only edges read and mix. Text
  is clipped as `rect` clips, the cut corner included.
- **Changes from the plan:** the shell's line editor takes ASCII only, so `desk font sample` posts a notice with
  African letters from inside the kernel for the screenshots; typing them waits for keyboard work. And the outline
  face is a little wider, which pushed the cell window's "unsigned" mark under Close anyway: the title now gives the
  mark its room first and cuts a long file name with "..".
- **Suite `desk-font`** (grader selftest 24 of 24) on both kernels, the PC at 1920x1080: the coverage hash
  `e650b7175d399587` (cap 9) and `f792bfcbb21451de` (cap 18), the same on ARM and PC; half a pixel keeps the ink
  within 2%; U+E000 draws the box; UTF-8 with a cut sequence gives 1 bad of 12 and stops at the end; the cache empties
  itself and draws the same pixels after; cap 1000 is clamped to 45 with no line dropped; nothing drawn outside a
  window or its cut corner; `desk verify` 0 rows three times; the frame budgets below.
- **Measured** (the cloud runner QEMU, the last run, the debug kernel):

| | ARM 640x480 | PC 1920x1080 (960x540 at 2x) |
|---|---|---|
| A to Z, cap 9: bitmap / outline warm | 789 / 419 us | 1246 / 454 us |
| 40-character title, cap 9: bitmap / outline warm | 754 / 507 us | 1150 / 575 us |
| the same at cap 18: bitmap / outline warm | 1158 / 978 us | 1337 / 1032 us |
| full frame: bitmap / outline | 26.4 / 23.8 ms (90%) | 48.4 / 42.7 ms (88%) |
| partial frame 300x24: bitmap / outline | 2462 / 2462 us | 3243 / 3215 us |
| first frame after an empty cache | 52.9 ms | 77.0 ms (budget 100 ms) |

  The outline face is faster than the bitmap face it replaces, because the bitmap face is one fill per run of dots.
  The 500 us text budget holds for the A-to-Z line; the 40-character title at cap 9 is 394 to 628 us across four
  runs (the machines vary), against 599 to 1284 us for the bitmap face on the same runs.
- the cloud runner on a60a791e: desk-font, desk, pc-desk, desk-cell, desk-cell-app, fbcon-time, 6 of 6. os2 43491064699: the
  same and smoke, 7 of 7, PC md5 `1c914664056d8c75bbadb5f8a44278f7`.
- ARM image, the cloud runner like for like on one machine: master 2fc6134b `2454bfa1f729ff04b118b3de23b3113b` (.text 1752608,
  .rodata 462400, .data 2064, .bss 30787736) to a60a791e `d18910a782c5863295b3bb0b036b4217` (.text 1809224, .rodata
  508656, .data 2072, .bss 31099032): .text +56,616, .rodata +46,256, .bss +311,296 (the arena, the lines, the slots
  and the cell row); a rebuild repeats it.
- Screenshots, before (master dd31b40a) and after, the same scenes, in the OS notes folder `premium_fonts/before` and
  `premium_fonts/after`, plus the African letters (`arm-letters`, `pc-letters`) and the cell window title
  (`cell-*-anyway`, `cell-*-app`).

**Before (13 Nov, the cloud runner on master dd31b40a, `scripts/desk-shots.sh`):** nine screenshots of the bitmap face, kept
for the after set. `desk perf` there (QEMU without acceleration, so slow, but the same machine the after run uses):
1920x1080 at 2x, a full frame 112 ms and a partial frame 65 ms; 1360x768 at 1x, a full frame 118 ms and a partial
frame 120 ms. `desk verify` 0 rows differ on both.

**Step 1 follow-ups done (13 Nov night, `claude/afriui-fonts-followups` on f35ddcf4), the architect's three and the owner's
look:**
1. **The narrowest title:** an app-owned cell window is never narrower than its lintel needs (the icon, a name cut
   to "..", the unsigned mark, Close anyway and the buttons; `lintel_min_w` works it out from the text widths), in
   every resize path and when the app opens it. `desk cell titletest NAME|off` (test only) shows an unsigned app's
   title with Close anyway and no app; `desk cell titlecheck` reports where the parts went. The desk-font suite
   resizes that window to 40 pixels and grades the mark whole, nothing over Close anyway, the width at its minimum.
2. **The title budget:** warm text now prints the worst of 20 beside the best. The 40-character title stays a little
   over 500 us at its worst (712 to 735 us on the last run); step 2's run cache is the fix.
3. **Mainline only:** no interrupt draws text. The PS/2 and xHCI handlers only post keys, and the PC screen tick
   returns at once while the desk shows. `vector::glyph` now also refuses to be entered twice (a panic).
- **ƙ and ƴ** (the owner's look): small-size forms with longer hooks, made by the tool (`SMALL` in outline_data.rs)
  and used up to a cap height of 12.
- the cloud runner 6 of 6 (desk-font with 30 grader cases); os2 43505846818 7 of 7, PC md5 `98812ecc6623d7bd55f08ebd9d2c3a01`.
  ARM like for like: f35ddcf4 `d18910a782c5863295b3bb0b036b4217` to `63f7bdf4b40536dc1cac7c77ea1af06b` (.text
  +4,096, .rodata +1,688, .bss +32). Shots: OS notes `premium_fonts/followups`.

## 27. The desk in the user's language (13 Nov 2026) [law 0]

Owner's mandate of 13 Nov: AfriSovera speaks the user's language. The full plan, with what the code had before and
the glyph table per language, is LANGUAGES_PLAN.md in the OS notes folder; this section records what is built.

**Stage 1, the catalogue (`afriui/lang.rs`):** every string a person reads on the desk has an id (`Msg`) and an
English text. The drawing code asks `lang::t(id)` or `Line::f(id, args)`; sentences are whole with numbered gaps
(`{0} core(s), {1} MiB memory`), so a language can put a number where its grammar wants. About finds its uptime row
by index, not by its label. `Line` holds 128 bytes and notices are cut at a character's start. What stays English on
purpose: `[desk]` serial lines and shell output (tests grade them), command words, file names, and the names
AfriSovera, Mansa, Tonga and Nia. The English screens do not change.

**Stage 2, the switch:**
- Settings > Display starts with a Language row: a chip with the language's own name (a click moves to the next)
  and an ochre "draft" mark while any of its strings is a machine draft. Settings is 22 px taller (home 380 x 322).
- `afrisovera.conf` keeps `lang=<code>` (2 to 8 lowercase letters; anything else reads as English) through the
  existing allocation-free `persist`; the desk reads it at the first `gui` and says `[desk] lang: tw (from
  afrisovera.conf)`. With no disk the choice lasts until power off.
- Every non-English string is `Draft` or `Checked`; a language's empty string shows the English and is counted.
  Twi is the first language beside English, all its strings drafts until a fluent speaker checks them.
- Room for longer words: About's value column starts after its widest label (never left of x + 100), Files
  right-aligns a Size word wider than English's. `desk lang fit` measures every string of every language against
  its room at the home sizes, with the widest numbers it can show.
- Shell verbs: `desk lang`, `desk lang <code>`, `desk lang check` (same gaps as the English, no combining marks, no
  banned dashes), `desk lang fit`. Suite `desk-lang` on both kernels, with a two-boot reboot on one disk image.

**Later (with the desktop designer):** combining marks placed over their letter (Yoruba tones on ẹ and ọ), then
Yoruba; the Terminal and cell windows on the outline face (premium step 3) before the shell's words are translated.

**The architect's conditions (13 and 14 Nov), as built:**
- **Safety words are never a draft:** `lang::SAFETY` lists "unsigned", "Close anyway" and the draft mark (camera,
  permission, password and update words join it as they arrive). A language shows its own words for these only when
  they are `Checked`; until then the desk draws the English, and `desk lang` counts them as English. `desk lang
  check` fails if a safety word would be drawn from a draft.
- **The filler cannot be steered by its arguments:** `desk lang selftest` checks a file name holding `{0}`, a gap
  past the arguments, lone and unmatched braces, two-digit braces, the largest number, an argument longer than the
  line (it stops at 128 bytes), and the safety rule in every language.
- **Widths follow the language:** `lintel_min_w` and the Close anyway button measure the language's words (the
  button never narrower than its English 84 px, so English is unchanged); `desk lang fit` checks the narrowest app
  title in every language against the cell window's home width, and desk-lang runs `desk cell titlecheck` in Twi.


## 28. The Terminal scrolls back, a pager, and output to a file (14 Nov 2026, urgent) [law 0]

The owner's first test on the HP, by its PS/2 keyboard and touchpad: `hwscan` printed 72 devices in the desk's
Terminal window and the top ran off with no way back ("The terminal is terrible, no scrolling or way to see the
other list"). Ahead of premium step 2 (section 27, on its branch).

**What the code has today** (master 196b024a):
- The text console already scrolls back on the PC (`pc/conback.rs`, HP field item F7, 9 Nov): 512 lines,
  Shift+PgUp and Shift+PgDn half a screen, the mouse wheel and a touchpad's scroll three lines a notch. But it scrolls
  only the console. With the desk showing, `ps2.rs` and `xhci.rs` still send Shift+PgUp, Shift+PgDn and the wheel to
  it, and it leaves the desk alone, so nothing moves.
- The desk's Terminal window keeps its own copy of the console text (`term_feed` in desk.rs): **48 lines** of 96
  bytes, drawn newest at the bottom (`terminal_app` in paint.rs), with no offset. A 72-line report loses its top 24
  lines for good.
- `help` pages itself on the PC (`pc/helpview.rs`): its lines are caught at `uart::_print` and shown a page at a
  time. Nothing else pages.

**The plan** (both kernels, unless said):
1. **The Terminal keeps 2,048 lines** (TL 48 to 2048; 96 bytes each plus a length: about 194 KiB of .bss, static,
   no heap) and gets a view offset: how many lines up from the bottom it shows.
   - **Shift+PgUp and Shift+PgDn** move the view half the window's rows. **The wheel and a touchpad's scroll** move
     it three lines a notch while the pointer is over the Terminal window. With the desk hidden both keep driving the
     console's own scrollback, as today.
   - The PS/2 and USB interrupt paths only add to an atomic count of lines to move (`term_scroll`). The desk applies
     it in mainline at its next pass and redraws the Terminal's body (one damaged box), so no interrupt draws.
   - **Any other key returns to the bottom**: a typed key with the desk showing sets an atomic "back to the bottom"
     flag. New output while scrolled back keeps the view where it is: the offset grows with each new line, up to the
     lines kept. So a long report can be read while a command is still printing.
   - **A mark while scrolled back:** a chip at the Terminal's top right, "N lines back, any key returns", drawn by the
     desk in its gold-on-night style.
   - `desk term up|down|bottom|status` for serial and the suite. `status` prints the offset, the lines kept and the
     first line shown.
2. **A pager: `more <command>` and `<command> | more`.**
   - The command runs as usual, but what it prints is caught at `uart::_print` (as `help`'s pages are) into a 32 KiB
     static buffer instead of reaching the screen. A longer output keeps its first 32 KiB, and says so at the end.
   - Then the pager shows it a screenful at a time: the Terminal window's rows with the desk showing, else the
     console's 48 lines. Under each screenful the line "More: Space next page, Enter next line, q quit" (not the old "more" between two dashes: no double
     hyphens in what a person reads). It waits for
     a key the way the shell waits for one: serial, PS/2 or USB, whichever the person types on.
   - Commands do not page by themselves: the suites read their raw output, and the scrollback above now covers a
     report that runs long. `hwscan | more` is one word away.
3. **Output to a file: `<command> > name`.** The same catch, then `vfs::write_full` (a file holds up to 16 KiB; a
   longer output is cut there, with a line saying so). `cat name` or `more cat name` reads it back.

**Costs:**
- About 194 KiB more .bss for the Terminal's lines, and the 32 KiB catch buffer. The lead set a few hundred KB as
  fine.
- Drawing: unchanged. The Terminal still draws only its visible rows, from an offset into the ring.
- The ARM image moves (desk.rs, uart.rs, shell.rs for the `more` and `>` forms); like for like md5 and llvm-size in
  the evidence.

**Proof:**
- A suite `term-scroll` on both kernels.
  - The PC uses QEMU's PS/2 keyboard and mouse, as the HP does. Through QMP: Shift+PgUp twice and the wheel up three
    notches over the Terminal, then a letter. The ARM kernel uses `desk term up` on serial.
  - Over `help` (600 lines) in the Terminal it grades the first line shown after each move, the mark's text, and the
    return to the bottom.
  - `more help`: the first screenful ends with the "More:" line; Space shows the next; q stops it, with the
    prompt back.
  - `help | more` does the same. `help > helpout` gives a 16 KiB file whose first line is help's.
  - `desk verify` 0 rows differ, while scrolled and after.
- `desk`, `pc-desk`, `desk-cell`, `desk-cell-app`, `desk-font`, `fbcon-time`, `pc-ps2mouse` and the console's own
  scrollback suite pass.
- Screenshots: the Terminal scrolled back with its mark, and the pager's "More:" line, to the lead for the
  owner. One os2 run.

**Questions for the architect:**
1. The Terminal's ring at 2,048 lines (194 KiB .bss) on both kernels.
2. Interrupt paths only count lines to move and set a flag; the desk moves the view in mainline.
3. The pager's catch at `uart::_print` beside `help`'s, a 32 KiB buffer, and the wait for a key after the command
   has finished (never inside the print lock).
4. No command pages by itself.

**Built (14 Nov, `claude/afriui-scrollback`, rebased on master 5b177294):**
- The Terminal keeps 2,048 lines. Shift+PgUp and Shift+PgDn move half its rows; the wheel and a touchpad's scroll
  move three lines a notch over the Terminal (or with it focused); a key typed on the PC's own keyboard
  (`uart::inject`) brings it back to the bottom. Serial typing does not, so the suite and the phone keep their place,
  and `desk term bottom` does it there. New output while scrolled back keeps the view on the same text.
- With the desk hidden, `console_page` and `console_wheel` drive the console's own scrollback as before; with it
  showing they go to the Terminal (pc/uart.rs), so ps2.rs and xhci.rs call them always.
- The mark: a gold chip "N lines back, any key returns" (lang.rs TermBack and TermBackOne, English only; the
  languages engineer adds Twi and the fit check).
- `more <cmd>`, `<cmd> | more` and `<cmd> > name` catch output at `uart::_print` on both kernels (32 KiB), page it with
  the line "More: Space next page, Enter next line, q quit", or save up to 16 KiB. The pager's own lines go past
  `help`'s page filter on the PC, which had held the prompt back on the first run.
- Found in the owner's photo on the way: the console drew Backspace as a box (framebuffer.rs con_putc), so history
  recall left "hwscan: .. .. .." on the input line. Backspace now steps back a cell on the console and in the PC's
  scrollback copy (conback.rs).
- Suite `term-scroll` (grader selftest 19 of 19), both kernels, the PC through QEMU's PS/2 keyboard and mouse: one
  page up is R/2, the wheel 3 lines a notch, Enter back to 0, `more help` stops after two pages and `help | more`
  after one, `help > helpout` saves 16,384 bytes, `desk verify` 0 rows while scrolled back.
- the cloud runner on the rebased branch: term-scroll, desk, pc-desk, desk-cell, desk-cell-app, desk-font, desk-lang,
  fbcon-time, pc-ps2mouse, 9 of 9. Also fixed: desk-font exited 0 on a failure (the runner showed NO-VERDICT). Its
  cold-frame budget missed once (120 ms) on a slow machine; an A/B on one machine gave the branch 62.5 ms and the base
  61.2 ms.
- ARM image, like for like against the branch point 5b177294: `8345a1039eecd59af7e083e1eb00cfe7` to
  `66fc353d7a8815714e91549ed67c84c1` (.text +12,632, .rodata +2,256, .bss +194,024, the Terminal's ring); a rebuild
  repeats it.


## 30. The premium look, step 2: corners, soft shadows and see-through panels (13 Nov 2026) [law 0]

Step 2 of the five the owner said yes to on 13 Nov: "rounded corners, soft shadows, see-through panels". Measured,
this week's speed kept, a plan to the architect first, before and after screenshots from the cloud. Step 1 (section
26) is merged (f35ddcf4).

**What the code has today** (master f35ddcf4):
- **Shape:** every container and control has corners cut at 45 degrees (section 3, the system's signature: a window
  is a carved tag with its top left and bottom right cut, controls cut all four, the Circle an octagon). `cut`,
  `cut_frame` and `chip` in paint.rs draw them with one fill a row; every edge is either ink or not, so the cuts and
  the one-pixel frames show stair steps.
- **Shadows:** a hard relief shadow, the window's own shape filled in near-black 4 pixels down and right (windows,
  the Circle, the notifications list, toasts). `win_box` already reaches 4 pixels past a window for it.
- **Panels:** every fill is opaque. The only mixing is text's (step 1: `framebuffer::blend_into`, which reads the
  pixel below).
- Partial frames (section 15) redraw each damaged box back to front (wallpaper, windows in z order, the Spine,
  toast, notifications, the Circle), so anything mixed with what lies below finds the right pixels below.

**A decision for the owner (yes or no, by screenshot): rounded, or the cut corner made smooth?** Section 3 chose cut
corners on purpose and calls rounding the thing the system does not do; the owner's yes of 13 Nov says "rounded
corners". This step builds both shapes with smooth edges behind one switch, and the default is the owner's answer:
- **(a) Round:** quarter circles everywhere a cut is today (8 px on windows, 2 to 6 on controls, the Circle a rounded
  square). The look most people will call premium; the carved-tag outline goes.
- **(b) Smooth cut (my recommendation):** the same 45-degree cuts, which keep AfriSovera's own outline, with the
  stair steps gone (anti-aliased edges), plus the soft shadows and see-through panels below. Premium and still ours.
The screenshots show the same scenes in both.

**The plan:**
- **One shape for every corner.** `shape(x, y, w, h, corners, fill)` with a radius per corner and a style, Cut or
  Round; `cut`, `chip` and `cut_frame` become calls to it, so the switch changes every corner at once. The straight
  middle stays one fill a row (as fast as today). Only the corner pixels mix: each has a coverage from 0 to 255, and
  the fill is mixed with the pixel below through `blend_into` (the same integer mixing as text, the same on ARM and
  PC).
- **Corner coverage from a table, made once.** For a Round corner of radius r, the coverage of each of its r x r
  pixels is counted from 16 x 16 sub-samples against the circle (integers, no square roots, no floats), at first use,
  into a static table for radii 1 to 24 (about 5 KB). A Cut corner's diagonal pixels are half covered (128) and the
  rest full or empty, so it needs no table. Frames (the one-pixel outlines) take the ring between two shapes.
- **Soft shadows.** The hard 4-pixel copy becomes a shadow that fades: 4 px down, spread 10 px, darkest at 45%
  black under the window's edge and falling to 0 along a fixed 11-step profile (a table, not a blur), with the
  corners shaped by the same corner tables at radius + spread. Only the ring outside the window is drawn (the window
  covers the rest). The damage box of a window (`win_box`) and of each panel grows by the shadow's reach (spread +
  offset), so moving a window clears its old shadow; `desk verify` proves it. Touch mode keeps no shadow, as today.
- **See-through panels.** The Spine, the notifications list, the Circle and toasts are filled at 86% over what lies
  below (a constant per panel, about 220 of 255), so the wallpaper and windows show faintly through. A tinted mix
  only: no blur in step 2 (a blur reads and writes every pixel several times, too slow at opt-level 0 on the ARM
  framebuffer; it can be a later step once a back buffer exists, P3). Windows stay opaque: a window is where work is
  read.
- **The text run cache (the architect's follow-up 2 of step 1).** Whole strings drawn often (window titles, Settings
  labels, the Spine's clock) are kept as one coverage block, keyed by a hash of the bytes, the cap height and the
  pen's quarter pixel, in a 64 KiB arena of their own that empties itself as the glyph cache does. A title then
  costs one `blend_into` instead of a lookup and a blend per glyph. Target: the 40-character title within 500 us at
  its worst of 20.
- **Switches for the shots and as a fallback:** `desk look cut|round` (the corners), `desk look shadow hard|soft`,
  `desk look glass on|off`; `desk look` prints the three. Before and after shots come from one build and one scene.

**What it costs, measured before it ships (the cloud runner, both kernels, `desk perf` and a new probe):**
- **Frames:** a full frame and a partial frame (a window's title, a toast) at 640x480, 1360x768 and 1920x1080, with
  each switch off and on. The rule: with everything on, a full frame no more than 15% slower than master f35ddcf4,
  and a partial frame no more than 15% slower.
- **Mixed pixels on ARM:** corners, shadows and see-through panels read the pixels they mix, which on the ARM desk
  are framebuffer pixels (fine in QEMU, slow on a phone panel until P3's back buffer). Counted per frame by the
  probe, so the cost is known.
- **Size:** the corner table (about 5 KB of .bss), the shadow profile (11 bytes) and the 64 KiB run arena; ARM md5
  and `llvm-size` like for like.

**Proof:**
- A suite `desk-look` on both kernels with grader selftest cases: a hash of the Round corner table for radii 8 and 16
  (the same on ARM and PC); a Cut corner's diagonal at 128; a see-through panel over a known pattern gives the
  expected mixed pixel; a shadow's profile at its edge and at its reach; `desk verify` 0 rows after partial frames
  with every switch on, and after moving a window (no stale shadow); the frame budgets; the run cache's 40-character
  title at its worst of 20.
- `desk`, `pc-desk`, `desk-cell`, `desk-cell-app`, `desk-font` and `fbcon-time` pass, their screen hashes updated
  where the look changed.
- the cloud runner on both kernels; screenshots of the same scenes in (a) and (b), at 1280x800 and 1920x1080, before and after,
  to the lead for the owner; one os2 run.

**Questions for the architect:**
1. One shape primitive for all corners, edges mixed with the pixel below through `blend_into`, the corner table made
   at first use into .bss.
2. Damage boxes grown by the shadow's reach (14 px with the offset), and `desk verify` as the proof that no shadow
   is left behind.
3. See-through panels without blur in step 2; windows stay opaque.
4. The text run cache in this step, with its own 64 KiB arena.
5. The budgets: full and partial frames each no more than 15% slower than f35ddcf4 with everything on.

**Built (14 Nov, `claude/afriui-look`, rebased on master 8fa5775f), with the architect's conditions:**
- **One shape for every corner** (`kernel/src/afriui/look.rs`): `cut`, `chip` and `cut_frame` go through `shape` and
  `frame`, in three styles behind `desk look hard|cut|round`. Hard is today's drawing exactly (the before shots); Cut
  is the default until the owner chooses (the architect's Q1 note); Round is the other choice. Only edge pixels mix.
  The Round coverage table is a `const fn` into .rodata (radii 1 to 24, 8 x 8 points a pixel), as asked; a bigger
  radius (the Circle) works from the row's crossing, kept for the row. Section 3's rule is rewritten when the owner
  chooses, in the same commit as the default.
- **Soft shadows:** a fixed profile (47% black at the edge, gone 7 px out, 4 px down), drawn only where the shape
  does not cover. The straight runs are block fills (black through a two-pixels-a-step darkening path), and only the
  corner squares are worked out per pixel, from a build-time distance table. The focused window and the panels cast
  the full shadow; unfocused windows a 3 px one. Damage boxes grow by the reach, clamped (`shadow_box`, `panel_box`),
  and `desk verify` is 0 rows after a move, a window dragged half off, a close, a resize and a toast coming and going.
- **See-through panels:** the notifications list, the Circle and toasts at 86%. Two changes from the plan: the Spine
  stays opaque (the wallpaper is not drawn under it, so a see-through Spine mixed again on every partial frame; the
  first build's `desk verify` found it), and touch mode has no shadow and no see-through, as planned.
- **The marks a person relies on are never mixed:** when a see-through panel lies over the cell window's title, its
  name, "unsigned" and Close anyway are drawn again over it, opaque (`marks_on_top`), unless a window above hides them.
- **The text run cache:** whole strings up to 120 bytes as one coverage block in a 64 KiB arena; a hit compares the
  hash, the cap height, the length and every byte; the arena empties before a new run is written. The probe forces
  every hash to collide and checks each string still draws as itself. Shaped strings (the scripts engineer's) skip it.
- Also, with the rebase: the architect's step 1 follow-ups. `desk cell titletest` is refused while an app owns the
  window and turned off by `app_cell_open`, and outline_data.rs's header (through the tool, regenerated in the cloud)
  names the two modified small forms.
- **Measured** (the cloud runner QEMU, debug, `desk look probe`, against the same build with everything off):

| all on | ARM 640x480 (pointer) | PC 1360x768 | PC 1920x1080 |
|---|---|---|---|
| full frame, smooth cut | 118% | 107% | 107% |
| full frame, round | 123% | 107% | 117% |
| partial frame, smooth cut | 112% | 104% | 104% |
| 40-character title, worst of 20 | 549 us | 390 us | 458 us |

  The PC is within every budget in smooth cut. ARM in pointer mode (QEMU's test setup) is 3 points over for a full
  frame and 49 us over for the worst title; the phone runs in touch mode, where step 2 is the corners only (about 104%).
  The cold first frame moves with the machine (an A/B on one machine gave the same figure with and without these
  changes, 14 Nov), so it is reported, not graded here.
- the cloud runner: desk, pc-desk, desk-cell, desk-cell-app, desk-font, desk-lang, term-scroll, fbcon-time, 8 of 8. os2 43565249940:
  desk, pc-desk, desk-cell, desk-font, desk-lang, term-scroll, smoke, 7 of 7, PC md5 `983a85172ed67e9704d778104b05e7e1`.
- ARM image, like for like against 8fa5775f: `42f428449e79aaed7f47da2fb8afa0cc` to `6674e87a068e4314c5c5500361ffc4d3`
  (.text +43,636, .rodata +14,728, .bss +86,016: the run arena and its slots); a rebuild repeats it.
- Shots, smooth cut and round, the same scenes: OS notes `premium_look/smooth-cut` and `premium_look/round`, and
  `cut_vs_round_circle.png` for the owner's choice.
## 32. Cell windows, stage 4: an Andika tile in the Circle, and the colour review (14 Nov 2026) [law 0]

Stage 4 of section 24: "an Andika tile in the Circle, 'open with Andika' later, and the design review of the 16
colours for contrast". Andika ships bundled (`andika.wasm` in `nia::BUNDLED`, A7) and runs in a cell window (stage 2),
but today it starts only by typing `nia andika.wasm`.

**What the code has today** (master 30238304):
- The Circle shows the four system apps (Files, Terminal, Settings, About) at its compass points on a ring of 16
  beads. `NTILES = 4` serves the Spine, the Circle, the keyboard walk and Super+1..4 alike.
- An app runs in the shell's mainline: `nia NAME` calls `run_file`, which returns when the app ends. The desk keeps
  drawing meanwhile (`app_tick`), and `crate::nia::app_running()` says whether one runs (section 25 left it public
  "so stage 4's Andika tile can grey itself").
- The cell colours (`CELL_ROLES`, Andika's order 0 to 15), on the window body (role 0 behind, `TERM_BG`), the
  cursor line (role 12) and the selection (role 9), measured as WCAG contrast:

| role | on the body | on the cursor line | on the selection |
|---|---|---|---|
| 0 text | 12.6 | 11.5 | 8.5 |
| 1 dim | 4.9 | 4.5 | **3.3** |
| 2 keyword | 11.7 | 10.7 | 7.9 |
| 3 string | 5.8 | 5.3 | **3.9** |
| 4 comment | 7.9 | 7.2 | 5.3 |
| 5 number | 6.9 | 6.3 | 4.6 |
| 6 punct | **4.4** | **4.0** | **3.0** |
| 7 gutter | **3.3** | **3.0** | **2.2** |
| 8 gutter-current | 7.4 | 6.7 | 4.9 |
| 10 status | 16.4 | 15.0 | 11.0 |
| 11 status-dim | 7.9 | 7.2 | 5.3 |
| 13 error | 4.9 | **4.4** | **3.3** |
| 14 ok | 8.6 | 7.9 | 5.8 |
| 15 accent | 7.2 | 6.5 | 4.8 |

  Bold is under 4.5:1, the usual floor for reading text. Punctuation and the gutter's line numbers are under it
  everywhere, and five roles are under it on a selection.

**The plan:**
1. **The Circle holds the system apps and the bundled apps meant for it.** A short list, `CIRCLE_APPS`, beside
   `nia::BUNDLED` names which bundled apps get a tile: Andika now, the media player later. The Spine and Super+1..4
   keep the four system apps (`NTILES` unchanged), so the Spine's layout and its suites do not move.
   - The Circle places all its tiles evenly round the ring, starting at the top: four as today, five with Andika.
     The keyboard walk goes round them in that order.
   - Andika's tile is drawn by the desk from rectangles, like the others: a cell grid of three short lines with a
     gold cursor block, on the indigo ground of the editor's cloth. Its label is "Andika", its own name in every
     language (the languages engineer adds the id).
2. **A tile starts the app the way typing would.** A click (or Enter on the walk) sets a pending command,
   `nia andika.wasm`. The shell's line reader, waiting in `cpu_getc`, takes it as if typed: the Terminal shows
   `afrisovera> nia andika.wasm` and the app runs in mainline, as `nia` always does. No second way to start an app,
   and nothing runs from inside the desk's own pass.
   - The Circle closes on the click, as a click on a system tile does.
   - While an app runs (`app_running()`), the tile is drawn dim and a click on it shows a toast, "Another app is
     running", instead of queuing anything. One app at a time, as section 25 rules.
   - A bundled app is signed, so its title shows `andika.wasm` with no "unsigned" mark.
3. **The colour review, with the native apps engineer** (the roles are Andika's): every text role reads at 4.5:1 or
   more on the body, the cursor line and the selection.
   - The dark selection moves darker and the dim roles lighter, keeping their hues, until the table passes. The
     proposed values come to the native apps engineer for a look before they ship, since they change his editor's
     colours.
   - `desk cell roles` prints the 16 colours, and the suite computes each ratio from them (in the grader, not the
     kernel), so a later change that drops a role under 4.5:1 turns the suite red.
4. "Open with Andika" (from Files) stays later, as section 24 says.

**Costs:** a few hundred bytes of code and one small list; the colours are constants. The ARM image moves (desk.rs,
paint.rs); like for like md5 and llvm-size.

**Proof:**
- A suite `desk-andika` on both kernels: the Circle lists five tiles with Andika fifth (`desk list`), the keyboard
  walk reaches it, a click starts `nia andika.wasm` (the line appears and Andika's cell window opens with its title
  and no unsigned mark), a second click while it runs gives the toast and starts nothing, and closing Andika
  (its own quit key, agreed with the native apps engineer) leaves the tile bright again.
- The contrast table in the grader: every text role at 4.5:1 or more on all three grounds.
- `desk`, `pc-desk`, `desk-cell`, `desk-cell-app`, `desk-font`, `desk-lang` pass; screenshots of the Circle with
  five tiles, Andika running, and Andika's page before and after the colour change, to the lead for the owner.

**Questions for the architect:**
1. The Circle's tiles beyond `NTILES` (bundled apps from a short list), with the Spine and Super keys unchanged.
2. A tile starting an app through the shell's line reader, as typed; a dim tile and a toast while an app runs.
3. The 4.5:1 floor for every text role on all three grounds, graded by the suite.

**Built (14 Nov, `claude/afriui-andika` on master ac39e3bf), with the architect's rules:**
- **The tiles.** `CIRCLE_APPS` in desk.rs is compiled in: (the bundled name, its label, its line). Only an app that
  passed the boot check gets a tile (`nia::bundled_ok`). The Circle places five tiles as a pentagon from the top
  (Files, Terminal, Settings, About, Andika); the Spine, `NTILES` and Super+1..4 are unchanged. Andika's tile is the
  editor's indigo with three short lines and a gold cursor block; its label is its own name in every language.
- **The line goes only to the prompt.** shell.rs `prompt_getc` is the top-level prompt's key wait, and the only
  reader of `desk::launch_take`. It is idle (`shell::at_prompt`) only while nothing is typed; a key typed after a
  click drops the tile's line. `cpu_getc` (the pager, appget's y, every command's key wait) never reads it. The line
  is the constant `nia andika.wasm`, echoed so the Terminal shows it after the prompt.
- **Busy.** While the shell runs anything (a command, the pager, an app), the app tile is drawn dim, and a click or
  Enter on the walk closes the Circle and shows the toast "The Terminal is busy; try again soon." with nothing
  queued. The Circle redraws when the shell becomes free, so the tile brightens by itself.
- **Test hooks:** `desk circle` (the tiles in walk order and whether app tiles are dim), `desk later <ms> tile <id>`
  (a tap when the shell next waits) and the cell plan step `tile <id>`.
- **The colours** (the native apps engineer chose to keep the selection, and the bigger lifts): dim A3988B, string
  33AE75, punct DC815D, gutter A59889, error E8796F. Every text role now reads at 4.6:1 or more on the body, the
  cursor line and the selection; the title's unsigned mark is 6.9 and Close anyway 5.2. `desk cell roles` prints
  them and desk-cell grades the ratios (selftest 28). The after shots showed Andika drawing its bars on text roles
  (the tab bar on gutter, dim on status); the native apps engineer moves every bar of Andika, Hati and Ngoma onto the
  cursor line (12), pairs already in the grade (status 15.0, status-dim 7.2, error 6.3, accent 6.5), and sends the
  new app bytes through the apps developer.
- **Tests** (os2): desk-andika on both kernels, with every refusal (Enter on the walk during a command, a tap while
  Andika runs, a tap while `more help` waits at More:, on ARM a tap while appget waits for its y; the pager got only
  its q and appget only its n), two starts from the tile, each signed and closed; desk, pc-desk, desk-cell,
  desk-cell-app, desk-lang, term-scroll, pc-deskmotion, desk-font and bundled-big green (43622669890 on the tile,
  43623425702 with the colours). desk and pc-deskmotion tap Settings where five tiles put it (400,286).
- Shots in the OS notes, `andika_tile/`: the Circle with five tiles, the 16 roles before and after, Andika with text.
