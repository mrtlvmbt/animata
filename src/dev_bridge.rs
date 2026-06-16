//! Dev bridge — a localhost JSON-RPC server for autonomous verification.
//!
//! Compiled only under `--features dev`. A background thread runs a tiny HTTP
//! server on `127.0.0.1:8127`; each request is parsed into a [`Cmd`], pushed onto
//! a shared queue, and the HTTP handler blocks on a one-shot reply that the main
//! loop produces on its next frame (so commands never run mid-step and the GL
//! context / world stay on the main thread). See `DEV_BRIDGE.md`.
//!
//! ```sh
//! curl -s 127.0.0.1:8127 \
//!   -d '{"jsonrpc":"2.0","id":1,"method":"animata/status","params":null}'
//! ```

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use macroquad::math::Vec2;
use serde_json::{json, Value};

use crate::world::World;
use crate::ColorMode;

/// A control/read command, decoded from a JSON-RPC request.
pub enum Cmd {
    Status,
    Inspect { id: Option<u64>, at: Option<Vec2> },
    Histogram,
    SetPause(bool),
    SetSpeed(u32),
    Step(u32),
    Reset { seed: Option<u64> },
    SetView { scale: Option<f32>, cx: Option<f32>, cy: Option<f32> },
    SetColor(ColorMode),
    Select { id: Option<u64>, at: Option<Vec2> },
    SetParam { name: String, value: f64 },
    Save(String),
    Load(String),
    Screenshot(String),
}

/// A queued request: the command plus the channel to answer it on.
pub struct Req {
    pub cmd: Cmd,
    pub reply: Sender<Value>,
}

pub type Queue = Arc<Mutex<VecDeque<Req>>>;

/// Start the bridge: bind the server and spawn its thread. Returns the shared
/// queue the main loop drains with [`take`]. A bind failure is logged and a dead
/// (never-filled) queue is returned, so the app still runs without the bridge.
pub fn spawn(port: u16) -> Queue {
    let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
    let server = match tiny_http::Server::http(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[dev_bridge] could not bind 127.0.0.1:{port}: {e}");
            return queue;
        }
    };
    eprintln!("[dev_bridge] listening on http://127.0.0.1:{port}");
    let q = queue.clone();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let id = v.get("id").cloned().unwrap_or(Value::Null);
            let method = v.get("method").and_then(Value::as_str).unwrap_or("");
            let params = v.get("params").cloned().unwrap_or(Value::Null);

            let resp = match parse_cmd(method, &params) {
                Ok(cmd) => {
                    let (tx, rx) = channel();
                    q.lock().unwrap().push_back(Req { cmd, reply: tx });
                    match rx.recv_timeout(Duration::from_secs(3)) {
                        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        Err(_) => rpc_err(id, -32000, "timeout: main loop did not answer"),
                    }
                }
                Err(msg) => rpc_err(id, -32601, &msg),
            };
            let header = tiny_http::Header::from_bytes(
                &b"Content-Type"[..],
                &b"application/json"[..],
            )
            .unwrap();
            let _ = request.respond(
                tiny_http::Response::from_string(resp.to_string()).with_header(header),
            );
        }
    });
    queue
}

/// Drain all pending requests for the main loop to service. Each `Req` carries
/// its own reply channel, so the loop can answer inline (mutating its locals).
pub fn take(queue: &Queue) -> Vec<Req> {
    queue.lock().unwrap().drain(..).collect()
}

fn rpc_err(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Map a JSON-RPC method + params into a [`Cmd`], or an error string.
fn parse_cmd(method: &str, p: &Value) -> Result<Cmd, String> {
    let f = |k: &str| p.get(k).and_then(Value::as_f64);
    let u = |k: &str| p.get(k).and_then(Value::as_u64);
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let at = || match (f("x"), f("y")) {
        (Some(x), Some(y)) => Some(Vec2::new(x as f32, y as f32)),
        _ => None,
    };
    match method {
        "animata/status" => Ok(Cmd::Status),
        "animata/histogram" => Ok(Cmd::Histogram),
        "animata/inspect" => Ok(Cmd::Inspect { id: u("id"), at: at() }),
        "animata/set_pause" => Ok(Cmd::SetPause(
            p.get("paused").and_then(Value::as_bool).ok_or("paused must be bool")?,
        )),
        "animata/set_speed" => Ok(Cmd::SetSpeed(u("steps").ok_or("steps must be uint")? as u32)),
        "animata/step" => Ok(Cmd::Step(u("n").unwrap_or(1) as u32)),
        "animata/reset" => Ok(Cmd::Reset { seed: u("seed") }),
        "animata/set_view" => Ok(Cmd::SetView {
            scale: f("scale").map(|v| v as f32),
            cx: f("cx").map(|v| v as f32),
            cy: f("cy").map(|v| v as f32),
        }),
        "animata/set_color" => {
            let mode = match s("mode").ok_or("mode must be string")?.as_str() {
                "diet" => ColorMode::Diet,
                "lineage" => ColorMode::Lineage,
                "species" => ColorMode::Species,
                other => return Err(format!("unknown color mode: {other}")),
            };
            Ok(Cmd::SetColor(mode))
        }
        "animata/select" => Ok(Cmd::Select { id: u("id"), at: at() }),
        "animata/set_param" => Ok(Cmd::SetParam {
            name: s("name").ok_or("name must be string")?,
            value: f("value").ok_or("value must be number")?,
        }),
        "animata/save" => Ok(Cmd::Save(s("path").unwrap_or_else(|| "animata_save.txt".into()))),
        "animata/load" => Ok(Cmd::Load(s("path").unwrap_or_else(|| "animata_save.txt".into()))),
        "animata/screenshot" => Ok(Cmd::Screenshot(s("path").unwrap_or_else(|| "shot.png".into()))),
        other => Err(format!("unknown method: {other}")),
    }
}

/// JSON of the world's latest stats snapshot + run controls — the assert surface.
pub fn status_json(world: &World, paused: bool, speed: u32, scale: f32, center: Vec2) -> Value {
    let s = world.stats.latest();
    json!({
        "tick": world.tick,
        "paused": paused,
        "speed": speed,
        "drought": world.in_drought(),
        "view": { "scale": scale, "cx": center.x, "cy": center.y },
        "population": s.population,
        "herbivores": s.herbivores,
        "predators": s.predators,
        "species": s.species,
        "lineages": s.lineages,
        "max_generation": s.max_generation,
        "avg_speed": s.avg_speed,
        "avg_sense": s.avg_sense,
        "avg_radius": s.avg_radius,
        "avg_carnivory": s.avg_carnivory,
        "avg_ornament": s.avg_ornament,
        "diversity": s.diversity,
        "niche_spread": s.niche_spread,
        "avg_memory": s.avg_memory,
        "avg_segments": s.avg_segments,
        "appendaged_frac": s.appendaged_frac,
        "frac_underground": s.frac_underground,
        "frac_air": s.frac_air,
        "avg_hidden": s.avg_hidden,
        "frac_finned": s.frac_finned,
    })
}

/// JSON of one creature (selected by id, by world-point, or the first), for
/// `life/inspect` — body plan, brain shape, and live state.
pub fn inspect_json(world: &World, id: Option<u64>, at: Option<Vec2>) -> Value {
    let cr = if let Some(id) = id {
        world.creatures.iter().find(|c| c.id == id)
    } else if let Some(p) = at {
        world
            .creatures
            .iter()
            .min_by(|a, b| {
                (a.pos - p).length_squared().total_cmp(&(b.pos - p).length_squared())
            })
    } else {
        world.creatures.first()
    };
    let Some(c) = cr else {
        return json!({ "found": false });
    };
    let segs: Vec<Value> = c
        .pheno
        .segments
        .iter()
        .map(|s| {
            json!({
                "length": s.length,
                "width": s.width,
                "appendage": format!("{:?}", s.appendage),
                "flexibility": s.flexibility,
            })
        })
        .collect();
    json!({
        "found": true,
        "id": c.id,
        "pos": { "x": c.pos.x, "y": c.pos.y },
        "layer": c.layer,
        "energy": c.energy,
        "age": c.age,
        "generation": c.generation,
        "lineage": c.lineage,
        "species_id": c.species_id,
        "carnivory": c.carnivory(),
        "radius": c.pheno.radius,
        "max_speed": c.pheno.max_speed,
        "primary_layer": c.pheno.primary_layer(),
        "n_hidden": c.pheno.n_hidden,
        "synapse_count": c.pheno.synapses.len(),
        "segments": segs,
    })
}

/// Population distributions for richer asserts: counts per layer, per appendage
/// kind, and segment-count / hidden-width spreads.
pub fn histogram_json(world: &World) -> Value {
    use crate::genome::Appendage;
    let mut layers = [0u32; 3];
    let mut app = [0u32; 5]; // none, fin, wing, leg, burrow
    let mut seg_counts = [0u32; 9]; // 0..=8
    let mut hidden = [0u32; 17]; // 0..=16
    for c in &world.creatures {
        layers[(c.layer as usize).min(2)] += 1;
        seg_counts[c.pheno.segments.len().min(8)] += 1;
        hidden[c.pheno.n_hidden.min(16)] += 1;
        let mut any = false;
        for s in &c.pheno.segments {
            match s.appendage {
                Appendage::Fin => app[1] += 1,
                Appendage::Wing => app[2] += 1,
                Appendage::Leg => app[3] += 1,
                Appendage::Burrow => app[4] += 1,
                Appendage::None => {}
            }
            if s.appendage != Appendage::None {
                any = true;
            }
        }
        if !any {
            app[0] += 1;
        }
    }
    json!({
        "population": world.creatures.len(),
        "layer": { "underground": layers[0], "surface": layers[1], "air": layers[2] },
        "appendage": { "none_bodies": app[0], "fin": app[1], "wing": app[2], "leg": app[3], "burrow": app[4] },
        "segment_counts": seg_counts,
        "hidden_widths": hidden,
    })
}
