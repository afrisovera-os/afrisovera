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
monument. No easing curves that need floating point. A Settings switch turns all motion off; tests run with it off.

## 8. Speed budget and how we keep it

- Flat fills only, clipped once per rectangle; cut corners cost one subtraction per row.
- The mud-cloth wallpaper is at most about 1,300 small fills a frame.
- Later slice: redraw only the part of the screen that changed (damage rectangles), so moving the pointer or a clock
  tick does not repaint the whole screen.
- The frame stays at 640x480 logical pixels for now; the PC scales it to the screen (desktop.md section 7). Every
  layout number comes from the screen size and the Spine width, so a bigger logical screen later re-lays out.

## 9. Phone later

The phone keeps the 8 Nov desktop for now: the ARM kernel image must stay byte-identical until the architect agrees.
The PC builds the new desktop from `kernel/src/sovereign/desk.rs`, a copy of `kernel/src/desk.rs` with the new look;
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
- **Model and paint apart** (architect's review): `kernel/src/sovereign/desk.rs` keeps the model (state, the input
  ring, windows and focus, hit-testing geometry, the `desk` commands); `kernel/src/sovereign/paint.rs` holds the
  theme (palette, cloths, cut-corner primitives, text, icons) and every drawing function. Paint reads the model;
  the model never draws. When the phone moves over, it keeps this model and gets a phone paint (or the same one with
  the touch profile), instead of a third copy of the desktop.
