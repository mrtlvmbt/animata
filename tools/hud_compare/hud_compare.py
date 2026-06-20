#!/usr/bin/env python3
"""HUD ↔ mockup pixel comparison.

For each UI state it:
  1. drives the running app over the dev-bridge (open the flyout / pick a debug view / hide UI),
  2. grabs the real HUD frame (window:true, 2200×1520) and the world-only background
     (window:false, 1100×760 — no HUD),
  3. renders the static mockup reproduction (template.html) in headless Chrome over that SAME
     background, at the same 2200×1520, so corner-anchored panels line up,
  4. diffs the two: per-region MAE + %-changed + an amplified heat-map, and a side-by-side strip.

Compositing the template over the app's own world isolates the *HUD* rendering difference
(egui blend vs CSS) instead of the unavoidable world mismatch. backdrop-blur is intentionally off
in the template (egui cannot blur), so diffs are actionable.

The sim is paused so consecutive frames are identical; the background is the real window
back-buffer with the HUD hidden (same render path → matches the world under the panels). The
bottom-left hide-hint chip that appears when the HUD is hidden is inpainted away (copied from the
strip just above it) so it doesn't pollute the transport region.

Usage:
    python3 hud_compare.py                 # all states → /tmp/hud_cmp/report.md
    python3 hud_compare.py --states base view
"""
import argparse, json, os, subprocess, sys, time, urllib.request
from pathlib import Path
from PIL import Image
import numpy as np

HERE = Path(__file__).resolve().parent
TEMPLATE = HERE / "template.html"
OUT = Path("/tmp/hud_cmp")
BRIDGE = "http://127.0.0.1:8127"
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
W, H = 1100, 760          # logical
SCALE = 2                 # backbuffer = 2200×1520

# state -> (panel, debug/field, hide)
STATES = {
    "base":  ("none",  "none", False),
    "world": ("world", "none", False),
    "view":  ("view",  "temp", False),   # view panel + temperature field recolor
    "pop":   ("pop",   "none", False),
    "perf":  ("perf",  "none", False),
    "hide":  ("none",  "none", True),
    "toast": ("none",  "none", False),   # special: triggers a Saved toast (see main loop)
}

# Regions of interest in LOGICAL px (x, y, w, h). `note` flags expected-divergent / text-noisy.
REGIONS = {
    "vitals":    (8, 8, 360, 74,   "text: day/time/pop are live"),
    "minimap":   (892, 8, 200, 150, "EXCLUDED interior: app keeps iso-diamond by design"),
    "transport": (8, 670, 360, 82, ""),
    "rail":      (1020, 466, 74, 282, ""),
    "flyout":    (760, 250, 312, 490, "text: live metrics differ"),
    "hint":      (8, 706, 150, 46, ""),
    "toast":     (470, 10, 160, 46, "top-centre Saved toast"),
}


FINDINGS = """\
Методика выверена: фон под панелями совпадает идеально (sRGB-профиль Chrome + вода выключена +
пауза) — в `hide` пустые регионы vitals/rail дают **MAE 0.0**. Значит весь остаточный MAE — это сам
HUD: тени + текст + реальные диффы.

**Совпало хорошо:**
- transport — play-кнопка (вектор-триангл на тинте), трек+кольцевой бегунок, `1.0×`, `PAUSED` ✓
- rail — иконки clock/layers/circles/bars выровнены по позиции и форме ✓
- hide-hint — `press [I] for UI` ✓ ; toast `Saved` (top-center) ✓
- vitals/world/perf — компоновка и kv-строки ✓
- view/pop — после тюна высота строк radio (26.5) и big-stat (32) ближе к макету

**Остаточные мелочи (низкий ROI, субпиксельно):**
- view-флайаут: лёгкий вертикальный дрейф строк (метрики строкового бокса egui-IBM-Plex vs Chrome
  чуть разные) — визуально незаметно.
- vitals: трекинг caps `DAY`/`POPULATION` ±1px; ширина панели зависит от разрядности population.
- rail: eye — эллипс-аппроксимация vs bezier-линза макета.

**Неустранимо / по дизайну (не баги):**
- Гало теней вокруг панелей — egui box-shadow мягче/шире CSS (доминирующий не-текстовый диф;
  параметры выставлены по числам макета, но рендер-модель иная).
- Минимапа: интерьер исключён — намеренно изо-ромб (решение пользователя).
- Полноэкранный field-recolor wash — только в макете (приложение красит лишь минимапу).
- Backdrop-blur — egui не умеет; в шаблоне отключён.

**Шум сравнения (не дефекты HUD):** живые значения (population/tick/fps/проценты) — поэтому в
строках с текстом ориентируйся на %off, а не на MAE."""


def rpc(method, **params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(BRIDGE, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=8) as r:
        return json.load(r)


def shot(path, window):
    rpc("animata/screenshot", path=str(path), window=window)


HINT = (8, 700, 150, 52)  # logical rect of the hide-hint chip (bottom-left)


def inpaint_hint(png):
    """Erase the hide-hint chip from a HUD-off background by copying the strip just above it."""
    im = Image.open(png).convert("RGB")
    x, y, w, h = [v * SCALE for v in HINT]
    src = im.crop((x, y - h, x + w, y))  # strip directly above
    im.paste(src, (x, y))
    im.save(png)


def render_template(panel, field, bg_png, out_png, toast=None):
    url = (f"file://{TEMPLATE}?panel={panel}&field={field}"
           f"&hide={'1' if panel=='hide' else '0'}&paused=1&bg={bg_png}")
    if toast:
        url += f"&toast={toast}"
    url += "&water=0"  # comparison runs disable the animated water surface
    subprocess.run([
        CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
        "--allow-file-access-from-files", f"--force-device-scale-factor={SCALE}",
        f"--window-size={W},{H}", "--default-background-color=00000000",
        # Render in plain sRGB so the world background PNG round-trips without a P3/colour-mgmt
        # brightness shift (otherwise every panel region carries a uniform ~10 MAE floor).
        "--force-color-profile=srgb", "--disable-color-correct-rendering",
        "--virtual-time-budget=4000", f"--screenshot={out_png}", url,
    ], check=True, capture_output=True)


def crop(img, region):
    x, y, w, h = [v * SCALE for v in region[:4]]
    return img.crop((x, y, x + w, y + h))


def diff_metrics(a, b):
    aa = np.asarray(a.convert("RGB"), dtype=np.int16)
    bb = np.asarray(b.convert("RGB"), dtype=np.int16)
    d = np.abs(aa - bb)
    mae = float(d.mean())
    pct = float((d.max(axis=2) > 28).mean() * 100.0)  # frac of pixels off by >~.11
    heat = np.clip(d.max(axis=2) * 3, 0, 255).astype(np.uint8)
    return mae, pct, Image.fromarray(heat, "L")


def side_by_side(app_c, tpl_c, heat):
    h = app_c.height
    strip = Image.new("RGB", (app_c.width * 3 + 16, h), (20, 20, 20))
    strip.paste(app_c.convert("RGB"), (0, 0))
    strip.paste(tpl_c.convert("RGB"), (app_c.width + 8, 0))
    strip.paste(heat.convert("RGB"), (app_c.width * 2 + 16, 0))
    return strip


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--states", nargs="*", default=list(STATES))
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    rpc("animata/set_timescale", scale=1, paused=True)  # freeze the sim (label stays 1.0×)
    rpc("animata/render", water=False)  # hide the real-time-animated water surface → static world

    rows = []  # (state, region, mae, pct, note, strip_path)
    for st in args.states:
        panel, field, hide = STATES[st]
        app_png = OUT / f"app_{st}.png"
        bg_png = OUT / f"bg_{st}.png"
        # 1a. background: real world with the HUD hidden (same render path, same debug field)
        rpc("animata/set_panel", panel="none", debug=field, show_info=False)
        time.sleep(0.4)
        shot(bg_png, True)
        if not hide:
            inpaint_hint(bg_png)
        # 1b. the actual HUD frame
        tpl_png = OUT / f"tpl_{st}.png"
        if st == "toast":
            rpc("animata/save")  # fires the "Saved" toast
            rpc("animata/set_panel", panel="none", debug="none", show_info=True)
            time.sleep(0.5)      # capture during the toast's full-opacity hold
            shot(app_png, True)
            render_template("none", "none", bg_png, tpl_png, toast="Saved")
        else:
            rpc("animata/set_panel", panel=("none" if hide else panel),
                debug=field, show_info=(not hide))
            time.sleep(0.4)
            shot(app_png, True)
            render_template("hide" if hide else panel, field, bg_png, tpl_png)
        app = Image.open(app_png)
        tpl = Image.open(tpl_png)
        # 3. per-region diff
        for name, region in REGIONS.items():
            if name == "minimap":
                continue  # interior intentionally diverges (iso diamond)
            if st == "toast":
                if name != "toast":
                    continue
            elif name == "toast":
                continue
            if name == "flyout" and (hide or st in ("base", "toast")):
                continue
            if name == "hint" and not hide:
                continue
            ac, tc = crop(app, region), crop(tpl, region)
            mae, pct, heat = diff_metrics(ac, tc)
            sp = OUT / f"strip_{st}_{name}.png"
            side_by_side(ac, tc, heat).save(sp)
            rows.append((st, name, mae, pct, region[4], sp.name))
        # full-frame heat (visual only — world outside panels is irrelevant noise)
        _, _, fh = diff_metrics(app, tpl)
        fh.save(OUT / f"heat_{st}.png")
        panel_maes = [r[2] for r in rows if r[0] == st]
        agg = sum(panel_maes) / len(panel_maes) if panel_maes else 0.0
        print(f"[{st}] panels avg MAE={agg:.1f}  (heat_{st}.png)")

    # report
    rep = ["# HUD ↔ mockup diff report", "",
           "App HUD (window:true, 2200×1520) vs static mockup reproduction composited over the",
           "app's own world background. Lower MAE / %off = closer. backdrop-blur disabled in the",
           "template (egui can't blur). Minimap interior excluded (intentional iso-diamond).", ""]
    rep.append("| state | region | MAE | %off>.11 | note |")
    rep.append("|---|---|---:|---:|---|")
    for st, name, mae, pct, note, _ in rows:
        rep.append(f"| {st} | {name} | {mae:.1f} | {pct:.1f}% | {note} |")
    rep += ["", "## Curated findings (from the strips)", "", FINDINGS, ""]
    rep += ["## Strips (app | template | heat) and full-frame heatmaps", ""]
    for st, name, mae, pct, note, fn in rows:
        rep.append(f"- `{st}/{name}` MAE {mae:.1f} → `{fn}`")
    (OUT / "report.md").write_text("\n".join(rep))
    print(f"\nReport: {OUT/'report.md'}")


if __name__ == "__main__":
    main()
