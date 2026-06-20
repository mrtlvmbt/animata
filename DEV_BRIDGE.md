# Dev bridge — autonomous verification for the voxel viewer

A runtime control/inspection channel so an agent (or a CI script) can **drive the
live app, read its state, and capture screenshots over `curl`** — verifying the
render without a human watching the display.

> **Status:** restored from the archived a-life build (tag `sim-v1`) and adapted to
> the voxel viewer. Compiled only under `--features dev` (`src/dev_bridge.rs`). A
> background thread runs a tiny HTTP server on `127.0.0.1:<port>`; each request is
> parsed into a `Cmd`, queued, and answered by the **main loop** on its next frame
> (so nothing touches the GL context off-thread).

## Port (per-branch, not fixed)

The bridge no longer binds a fixed `8127` — so several branch checkouts / agent
sessions can run in parallel without colliding. The port is picked by `port()`:

1. `ANIMATA_DEV_PORT` env var if set (explicit override), else
2. a STABLE port derived from the current git branch (FNV-1a → `49152..=65535`,
   same branch ⇒ same port), else
3. `8127` (not a git checkout).

On bind the chosen port is written to **`.animata-dev-port`** in the cwd
(gitignored, one per worktree). **Always read the port from that file — never
assume 8127:**

```sh
PORT=$(cat .animata-dev-port)
curl -s 127.0.0.1:$PORT -d '{"jsonrpc":"2.0","id":1,"method":"animata/status","params":null}'
```

## Run

```sh
cargo run -p animata --features dev   # opens the window + binds 127.0.0.1:$(cat .animata-dev-port)
ANIMATA_DEV_PORT=51234 cargo run -p animata --features dev   # force a port
source tools/dev/bridge.sh            # curl recipes (bstatus, bview, breseed, bshot…)
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
curl -s 127.0.0.1:$(cat .animata-dev-port) \
  -d '{"jsonrpc":"2.0","id":1,"method":"animata/status","params":null}'
```

## How it works

- **Transport:** bg HTTP thread (`tiny_http`) ↔ main loop via a shared queue +
  per-request one-shot reply channel; the HTTP handler blocks up to 3 s for the
  main loop's answer (`timeout: main loop did not answer` if it never draws).
- **Screenshots read an offscreen target:** the scene is rendered into a
  `RenderTarget` each frame and blitted to the window. The `Screenshot` command
  stashes its reply; post-draw the main loop reads that target's texture
  (`get_texture_data`, rows flipped back) and `export_png(path)`. Reading the
  finished pixels *before* the window present decouples capture from the throttled
  front buffer — so it works even when the window isn't focused (GRAV-style
  framebuffer read), not just `get_screen_data` of a foregrounded window. Restrict
  paths to the repo dir.
- **Remaining limit:** capture still needs the loop to *tick*. A merely unfocused
  (but visible) window keeps ticking → capture works. A fully occluded/minimized
  window can have its frames frozen by the OS → the loop stalls and calls time out.

## Maintaining it

This bridge is kept **in sync with the viewer** as it grows: when a new piece of
state is worth asserting (e.g. chunk/cull counts, water stats) or a new control is
added, extend `Cmd` + `parse_cmd` in `src/dev_bridge.rs`, service it in `main()`,
and add a recipe to `tools/dev/bridge.sh`. Keep `animata/status` the canonical
numeric assert surface.
