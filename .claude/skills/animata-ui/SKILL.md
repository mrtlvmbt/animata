---
name: animata-ui
description: >
  Operating manual for the animata in-app GUI (crates/animata/src/ui) — the egui "naturalist's
  dashboard" HUD over the macroquad 3D world. Load at the START of any HUD work: panels/flyouts,
  vitals/transport/rail/minimap/toast, creature inspector, theme tokens, fonts/icons, hand-painted
  widgets, world-anchored overlays, mockup pixel-match. Holds the egui-0.31 + egui_macroquad gotchas
  (integer geometry, premultiplied alpha, alpha weakening, frame ordering), the
  UiState/SimMetrics/UiActions/HudCache seam, the verify-in-app / mockup-diff procedures. Read before
  editing UI code, not after the panel looks wrong.
triggers:
  - "HUD"
  - "egui"
  - "UI panel / flyout"
  - "creature inspector"
  - "minimap / transport / vitals"
  - "theme tokens"
  - "macroquad overlay"
  - "mockup pixel-match"
  - "animata-ui"
---

# animata-ui — the HUD operating manual

The in-app GUI is an **egui overlay composited over the macroquad 3D world** — dark glass panels, ONE
warm amber accent, IBM Plex type ("naturalist's dashboard"). egui has no native letter-spacing, SVG,
rounded clipping, backdrop blur, or mesh gradients, and `egui_macroquad` distorts translucent fills, so
the HUD is hand-painted against exact mockup numbers. Most mistakes here are alpha/geometry surprises or
frame-ordering lag. This skill is the spine: read it, then act. The dev-bridge control channel is
documented in `DEV_BRIDGE.md` (repo root); the PR / review / verify-in-app rules are §7–§8 here +
`CLAUDE.md`. (The HUD-redesign and creature-inspector feature state is dated/volatile — it lives in the
code + `~/.claude/plans/`, not a durable doc.)

Stack: `egui = "0.31"`, `egui-macroquad = "0.17.3"`, `egui-phosphor = "0.9"` (Regular), `macroquad 0.4`.
**No rustfmt** — match surrounding style by hand; the ONLY gate is
`cargo clippy -p animata --all-targets -- -D warnings` (add `--features dev` when dev code changed).
**Never run `cargo fmt`** (massive churn). UI lives entirely in `crates/animata` (the bin); the sim crate
(`animata-sim`) knows nothing of egui — see the `animata-sim` skill for that side.

## 0. The rules that dominate everything

- **`Color32` is PREMULTIPLIED.** Writing a translucent tint as `from_rgba_premultiplied(fullRGB, lowA)`
  is INVALID (RGB ≫ alpha) and renders near-opaque (a "0.10 white" track came out solid white). Always
  go through `theme::straight(r,g,b,a)` — a `const fn` that premultiplies straight alpha. `from_rgba_
  unmultiplied` isn't const, hence `straight`.
- **egui 0.31 geometry is INTEGER.** `Margin`/`CornerRadius`/`Shadow` fields are `i8`/`u8`. Write integers
  in `theme.rs`, not floats. (Stroke widths + painter coords are still `f32`.)
- **`egui_macroquad` WEAKENS dark translucent fills.** A `.72`-alpha glass panel comes out lighter than
  the number says, and lighter still over bright terrain than over water. Panel alphas in `theme.rs` are
  EMPIRICALLY calibrated, not the raw mockup numbers — don't "correct" them to match CSS. Verify the
  rendered tint in-app, not by arithmetic.
- **`TextureHandle` must outlive the frame's paint.** Any texture (minimap, rounded-bar mask) must be
  cached in `HudCache` (owned by `main`, passed `&mut` into `draw_hud`) — a handle dropped within the
  frame paints nothing. Bound cache growth (see `HudCache::bars`, cleared past 64 entries).
- **One amber accent for CHROME.** `ACCENT`/`ACCENT_TEXT`/`ACCENT_LINE`/`ACCENT_FILL` = selection/active/
  focus only. Colour that carries DATA (trophic dot, vitals bars, strata segments, field ramps) uses the
  `DATA_*`/`GOOD_GREEN`/`STRATA_*` tokens — never introduce a second chrome accent.

## 1. Architecture (where things live) — `crates/animata/src/ui/`

- **`theme.rs`** — design tokens + helpers. Colours via `straight()`; type via `mono(sz)`/`sans(sz)`;
  letter-spacing via `paint_tracked`/`tracking_em`/`total_tracked_width` (egui has NO native tracking —
  caps are laid out glyph-by-glyph). `themed_frame(FrameKind)` is the ONE source of per-panel padding/
  radius/fill/shadow (`Vitals`/`Rail`/`Transport`/`Flyout`/`Inspector`). `install_fonts` registers IBM
  Plex Sans+Mono and Phosphor (Phosphor is added to BOTH the Proportional AND the Monospace family — our
  icons render in Monospace, else tofu). `install_style` makes widget backgrounds transparent so panels
  read as glass. Call `install_fonts`/`install_style` ONCE.
- **`mod.rs`** — the data seam (see §2): `UiState`, `SimMetrics`, `UiActions`, `HudCache`, `Panel`, plus
  `legend_text`/`ramp_color` for field maps. Re-exports `draw_hud` and the inspector types.
- **`hud.rs`** — the nine screen-anchored Areas (vitals top-left, transport bottom-left, rail + one
  flyout bottom-right, minimap top-right, toast top-centre, hide-hint) and the shared widgets. `draw_hud`
  is the entry point; it returns `UiActions`.
- **`inspector.rs`** — the creature inspector: a selection-driven panel (left, under vitals) + the
  in-world crosshair/conspecific markers. Outside the "one panel at a time" rule (see §6).
- **`minimap.rs`** — the iso-diamond minimap texture build (kept iso by design, NOT flat like the mockup).
- **`loader.rs`** — the full-screen modal worldgen/load overlay (drawn INSTEAD of the HUD).

## 2. The data seam — how state flows (do not bypass it)

`draw_hud(ctx, st: &mut UiState, m: &SimMetrics, cache: &mut HudCache, terrain, now: f32) -> UiActions`.

- **`UiState`** — `Copy` struct of plain toggles (show_info, debug_view, water_on, mask, outline,
  open_panel, selected). Widgets mutate it DIRECTLY (an egui checkbox writes its `&mut bool`); the same
  fields are flipped by hotkeys in `main.rs`, so widget and key share one source of truth. Keep it `Copy`
  — don't add `String`/`Vec` to it.
- **`SimMetrics`** — a READ-ONLY snapshot built fresh each frame in `main.rs` (so the `ui` module stays
  free of sim-getter knowledge). Anything the panels display that needs a sim getter (population stats,
  the inspector's `CreatureView`, screen-projected marker positions) is computed in `main.rs` and handed
  in here. This is the pattern; follow it (`LifeStats`, `CreatureView` are the worked examples).
- **`UiActions`** — non-trivial intents flowing BACK to `main` (toggle_pause, set_time_scale, save, load)
  plus `wants_pointer` (= `ctx.is_pointer_over_area()`) used to gate world mouse input on `!wants_pointer`
  so a click on glass never reaches the world.
- **`HudCache`** — persistent GPU resources (minimap `TextureHandle` keyed by `(seed,view,tick-bucket)`;
  `bars` map of 1-D colour textures). Owned by `main`, lives across frames.

## 3. Frame ordering — the #1 subtle trap

The macroquad loop does: **(a) build the egui UI** (`egui_macroquad::ui(|ctx| draw_hud(...))`) at the
TOP of the frame → **(b) input/hotkeys/camera update** → **(c) render the 3D world**, where
`vp = cam.camera().matrix()` and creatures are projected → **(d) `egui_macroquad::draw()`** composites the
HUD on top → `next_frame()`. Consequences:

- The egui UI is BUILT before this frame's camera/`vp` exists. So any **world-anchored overlay** (the
  inspector crosshair, conspecific rings) must be projected in `main.rs` BEFORE the egui pass using
  `cam.camera().matrix()` — which is the PREVIOUS frame's camera. Net: a ≤1-frame lag on world-anchored
  HUD elements. Acceptable and documented; don't try to "fix" it by reordering the egui pass.
- Screen-anchored panels have no lag (they're absolute). Only things that track a world point do.
- **DPI:** `ctx.set_pixels_per_point(dpi_scale())` each frame, so 1 logical pt = `dpi_scale` px (2 on
  Retina). Screenshots are physical px (e.g. 2200×1520 for a 1100×760 logical window). Account for the
  scale when measuring a capture.
- The modal **loader replaces the HUD** (drawn instead of `draw_hud`, input gated on `loader_active`) —
  the egui_macroquad scrim isn't fully opaque, so drawing the HUD under it would bleed through.

## 4. Hand-painted widgets — the techniques (egui can't do these natively)

- **Letter-spacing (caps tracking):** `theme::paint_tracked(ui, pos, align, text, font, color, tracking)`
  lays out glyph-by-glyph (`ui.ctx().fonts(|f| f.glyph_width(..))`). `caps_tracked` is the in-flow helper.
- **Vector icons / glyphs:** there is NO SVG. Predefine vertices in a 24-px viewBox and map into the rect
  (`hud::vb`/`vbr`); draw with `Shape::line`/`circle_stroke`/`convex_polygon`/`rect_filled`. Icons don't
  recolour themselves — pass the colour for idle/hover/active. (Rail icons, play/pause glyph, radio/
  checkbox markers, the inspector close `×` are all this.)
- **Rounded multi-colour bars (gradient legends, strata, genome):** `hud::rounded_bar` paints a
  `RectShape::filled(rect, corner_radius, WHITE).with_texture(tex, uv)` — the rounded GEOMETRY masks the
  corners (a true mask), the 1-D colour texture fills it. The texture is cached in `HudCache::bars` by a
  hash of its colours. This replaced earlier end-cap hacks (egui clamps a corner radius to capwidth/2 →
  half-size bars). Single-colour progress bars use `hud::bar_sized(ui, frac, col, h, r)`.
- **Custom rows that stay clickable:** allocate with `Sense::click()`, return/inspect the `Response`,
  apply on `.clicked()`. (There is no focus/tab nav in an egui overlay — every toggle also has a hotkey.)
- **No backdrop blur, no mesh gradients.** Skip blur; emulate a gradient band as a loop of thin rects
  with decreasing alpha if ever needed.
- **Missing glyphs render as tofu.** `→` (U+2192) is ABSENT from the vendored IBM Plex subset — use a
  Phosphor glyph (`egui_phosphor::regular::ARROW_RIGHT`) or ASCII. The static guard is
  `tools/hud_compare/tofu_check.py` (scans UI string literals against the font cmaps + Phosphor PUA).

## 5. Shared widgets & frames (reuse, don't re-invent)

In `hud.rs` (raise to `pub(crate)` if a new module needs them, as the inspector did): `kv` (label…value
row), `hairline`, `bar`/`bar_sized`, `caps_tracked`, `rounded_bar`, `legend_bar`, the `vb`/`vbr` viewBox
mappers, `secondary_button`. New panel chrome → add a `FrameKind` variant to `themed_frame` rather than
cloning a `Frame` and patching `inner_margin` (single source of truth). When stacking many rows in a
panel, **zero `ui.spacing_mut().item_spacing.y`** and use explicit `add_space()` — egui adds item-spacing
between EVERY vertical item, which silently inflates a pixel-spec'd panel's height (this bit the inspector:
~6 px × rows of creep). Clamp a panel's width with `set_width` AND `set_max_width`; pin a corner button
with a `right_to_left` layout so it can't grow the panel.

## 6. The "one panel at a time" rule and its exception

The rail is the ONLY entry to detail flyouts, and only ONE flyout is open at a time (`UiState.open_panel`).
The **creature inspector is the exception**: it's selection-driven (click a creature in the world), keyed
by a STABLE creature `id` in `UiState.selected` (the `creatures` Vec reorders each tick — never key by
index), and it COEXISTS with any flyout. Picking is screen-space and runs as its OWN input branch in
`main.rs` — a hit toggles the selection and suppresses the pan-grab; a miss starts the pan (one
`MouseButton::Left` can't do both). Gate picking on `show_info`. A dead selection clears itself. Markers
draw on a background `layer_painter` (under panels, non-interactive → world stays clickable through them).

## 7. Verifying — never eyeball from reasoning

- **Run it:** `cargo run -p animata --features dev`. Drive the HUD over the dev bridge for scripted
  screenshots — the port is **PER-BRANCH**, read it from `.animata-dev-port`, never assume 8127
  (full protocol: `DEV_BRIDGE.md`). Methods: `animata/set_panel{panel,debug,show_info}`, `animata/select{id|nearest}`
  (drives the inspector), `animata/set_timescale{paused}`, `animata/render{water}`,
  `animata/screenshot{path,window}`. Read the PNG back and LOOK.
- **Pixel-match a mockup:** the reference mockups are the `Animata GUI*.html` files. Render one in
  headless Chrome (`--headless=new --force-device-scale-factor=2 --window-size=W,H --force-color-profile=
  srgb`), crop the region, and put it side-by-side with the app capture (scale both to one height). The
  `srgb` profile matters — Chrome's default P3 colour management adds a uniform ~10-MAE brightness shift.
  `tools/hud_compare/` automates the older HUD diff (composites the static `template.html` over the app's
  own paused, water-off world so the diff isolates HUD rendering); extend it or do a manual side-by-side
  for new panels.
- **Gate:** `cargo clippy -p animata --all-targets -- -D warnings` (add `--features dev` if dev code
  changed) must be clean. Don't `cargo fmt`.
- **The render bin is NOT in cloud CI** (it links macroquad → needs a GPU/display; CI covers
  `animata-sim` only — see the `.github/workflows/tests.yml` header). So for UI/render changes the clippy gate above AND
  the in-app visual verification stay **local** — `ci-report.sh` green does not vet the bin.

## 8. Workflow gates (non-negotiable)

- **Big HUD feature / redesign → plan-consensus FIRST** (`/plan-consensus`) before code; land the plan in
  `~/.claude/plans/`. (The inspector and the pixel-perfect redesign both went through it.)
- **Land on main ONLY via a GitHub PR.** Create the branch in a SEPARATE Bash
  call (a guard hook blocks committing on main even in a `checkout -b && commit` compound); confirm
  `git rev-parse --abbrev-ref HEAD`; then commit. Rebase onto fresh `origin/main`, re-run clippy, push,
  PR, merge. Commit messages end with the `Co-Authored-By` trailer; PR bodies end with the Claude Code
  line. Pure docs/UI-polish PRs don't need subsystem-reviewer (state why); behavioural changes do.
- **`.claude-dev-kit/**` is READ-ONLY** — never edit it locally.

## 9. The standard loop for a HUD change

1. Read this skill + the relevant memory. Identify: screen-anchored or world-anchored? new texture?
2. Big feature → plan → plan-consensus → implement; else implement smallest-first.
3. Put any sim-derived data into `SimMetrics` (built in `main.rs`); keep `ui` free of sim getters.
4. Reuse `theme`/`hud` helpers; add a `FrameKind` for new chrome; cache any texture in `HudCache`.
5. `cargo clippy -p animata --all-targets [--features dev] -- -D warnings` — green.
6. Run it, screenshot over the dev bridge, LOOK; pixel-match the mockup side-by-side; run `tofu_check.py`
   if you added any non-ASCII string.
7. Branch (separate Bash call) → commit → PR → merge → sync main → update the relevant memory.
