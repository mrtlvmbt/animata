# hud_compare — HUD ↔ mockup pixel diff

Quantitative check of how closely the in-app egui HUD matches the `Animata GUI` mockup.

## How it works
For each UI state it drives the running app over the dev-bridge, then:
1. grabs the world with the HUD hidden (`window:true`, `show_info:false`) as the **background** —
   same render path, so the world under the panels matches exactly (sim is paused so frames are
   stable; the bottom-left hide-hint chip is inpainted out);
2. renders `template.html` (a static, deck-runtime-free reproduction of the mockup) in headless
   Chrome **over that same background**, at the same 2200×1520;
3. diffs the two per region → MAE, %-pixels-off, an amplified heat-map, and an
   `app | template | heat` strip.

Compositing the template over the app's own world isolates the **HUD rendering** difference
(egui blend vs CSS) from the unavoidable world mismatch. backdrop-blur is disabled in the template
(egui can't blur) so diffs are actionable.

## Run
```sh
# build + launch the app with the dev bridge (separate terminal), wait for world-gen:
cargo run --release -p animata --features dev
# then:
python3 tools/hud_compare/hud_compare.py                 # all states
python3 tools/hud_compare/hud_compare.py --states view pop
```
Outputs land in `/tmp/hud_cmp/`: `report.md` (metrics table + curated findings), `strip_<state>_<region>.png`,
`heat_<state>.png`, `app_*/tpl_*/bg_*`.

## tofu_check.py — missing-glyph guard
Static scan (no app needed) that flags characters in HUD string literals that no loaded font can
render — they show as tofu boxes (□), e.g. `→` (U+2192) is absent from the vendored IBM Plex subset.
Checks against the IBM Plex Sans/Mono cmaps + the Phosphor PUA range; exits non-zero on a hit.
```sh
python3 tools/hud_compare/tofu_check.py
```
Fix a hit with a Phosphor glyph (`ph::ARROW_RIGHT`) or an ASCII form.

## Requirements
Google Chrome (`/Applications/Google Chrome.app`), `python3` with `pillow` + `numpy`, the app
running with `--features dev` (dev-bridge on 127.0.0.1:8127, incl. the `animata/set_panel` method).

## Caveats
- Live values (population/tick/fps/percentages) differ from the static template → expected text noise.
- Minimap interior is excluded (the app intentionally keeps the iso-diamond; the mockup is flat).
- The whole-screen field-recolor wash is mockup-only (the app recolors just the minimap) and is left
  off in the template so it doesn't confound the per-panel diff.
- Panel drop-shadows differ (egui box-shadow ≠ CSS) — a cosmetic halo around every panel edge.
