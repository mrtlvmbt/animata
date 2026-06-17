# Dev bridge — autonomous verification for the voxel viewer

A runtime control/inspection channel so an agent (or a CI script) can **drive the
live app, read its state, and capture screenshots over `curl`** — verifying the
render without a human watching the display.

> **Status:** restored from the archived a-life build (tag `sim-v1`) and adapted to
> the voxel viewer. Compiled only under `--features dev` (`src/dev_bridge.rs`). A
> background thread runs a tiny HTTP server on `127.0.0.1:8127`; each request is
> parsed into a `Cmd`, queued, and answered by the **main loop** on its next frame
> (so nothing touches the GL context off-thread).

## Run

```sh
cargo run --features dev          # opens the window + binds 127.0.0.1:8127
source tools/dev/bridge.sh        # curl recipes (bstatus, bview, breseed, bshot…)
```

## Methods (`animata/*`)

| Method | Params | Effect / result |
|---|---|---|
| `animata/status` | — | `{ fps, frame_ms, seed, view:{cx,cz,zoom,yaw}, map:{cols,rows,vox_m,map_scale,meshes} }` |
| `animata/set_view` | `{cx?,cz?,zoom?,yaw?}` | move/zoom/rotate the iso camera (each field optional) → `{ok}` |
| `animata/reseed` | `{seed?}` | regenerate the world (omit `seed` → next) → `{seed}` |
| `animata/screenshot` | `{path}` | capture the current frame to a PNG (serviced post-draw) → `{saved}` |

Raw call:

```sh
curl -s 127.0.0.1:8127 \
  -d '{"jsonrpc":"2.0","id":1,"method":"animata/status","params":null}'
```

## How it works

- **Transport:** bg HTTP thread (`tiny_http`) ↔ main loop via a shared queue +
  per-request one-shot reply channel; the HTTP handler blocks up to 3 s for the
  main loop's answer (`timeout: main loop did not answer` if it never draws).
- **Screenshots are deferred:** the `Screenshot` command stashes its reply; after
  the frame is fully drawn the main loop runs `get_screen_data().export_png(path)`
  and answers. Restrict paths to the repo dir.
- **Sandbox gotcha:** macroquad only services frames (hence the bridge) while the
  window is **foregrounded** — an unfocused/headless window stalls the loop and
  every call times out. Capture locally with the window in focus.

## Maintaining it

This bridge is kept **in sync with the viewer** as it grows: when a new piece of
state is worth asserting (e.g. chunk/cull counts, water stats) or a new control is
added, extend `Cmd` + `parse_cmd` in `src/dev_bridge.rs`, service it in `main()`,
and add a recipe to `tools/dev/bridge.sh`. Keep `animata/status` the canonical
numeric assert surface.
