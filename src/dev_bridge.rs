//! Dev bridge — a localhost JSON-RPC server for autonomous verification.
//!
//! Compiled only under `--features dev`. A background thread runs a tiny HTTP
//! server on `127.0.0.1:8127`; each request is parsed into a [`Cmd`], pushed onto a
//! shared queue, and the HTTP handler blocks on a one-shot reply that the main loop
//! produces on its next frame (so commands never touch the GL context / app state
//! off the main thread). See `DEV_BRIDGE.md`.
//!
//! Restored from the archived a-life build (tag `sim-v1`) and adapted to the voxel
//! viewer: the transport/threading/deferred-screenshot machinery is unchanged; the
//! command set now drives the camera, the world seed and screenshots.
//!
//! ```sh
//! curl -s 127.0.0.1:8127 \
//!   -d '{"jsonrpc":"2.0","id":1,"method":"animata/status","params":null}'
//! ```

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// A control/read command, decoded from a JSON-RPC request.
pub enum Cmd {
    /// Camera + world + timing snapshot (the assert surface).
    Status,
    /// Move/zoom/rotate the iso camera (any field optional).
    SetView {
        cx: Option<f32>,
        cz: Option<f32>,
        zoom: Option<f32>,
        yaw: Option<f32>,
    },
    /// Regenerate the world; `seed` omitted → next seed.
    Reseed { seed: Option<u64> },
    /// Drive the sim clock: set the time scale and/or pause state (either field optional).
    SetClock { scale: Option<f32>, paused: Option<bool> },
    /// Toggle render flags (diagnostic): `water` draws the translucent surface, `topo` is the
    /// height-debug view (water hidden). Either field optional.
    Render { water: Option<bool>, topo: Option<bool> },
    /// Capture the current frame to a PNG (serviced post-draw on the main loop).
    Screenshot(String),
}

/// A queued request: the command plus the channel to answer it on.
pub struct Req {
    pub cmd: Cmd,
    pub reply: Sender<Value>,
}

pub type Queue = Arc<Mutex<VecDeque<Req>>>;

/// Start the bridge: bind the server and spawn its thread. Returns the shared queue
/// the main loop drains with [`take`]. A bind failure is logged and a dead queue is
/// returned, so the app still runs without the bridge.
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
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = request
                .respond(tiny_http::Response::from_string(resp.to_string()).with_header(header));
        }
    });
    queue
}

/// Drain all pending requests for the main loop to service. Each `Req` carries its
/// own reply channel, so the loop answers inline (mutating its locals).
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
    let b = |k: &str| p.get(k).and_then(Value::as_bool);
    match method {
        "animata/status" => Ok(Cmd::Status),
        "animata/set_view" => Ok(Cmd::SetView {
            cx: f("cx").map(|v| v as f32),
            cz: f("cz").map(|v| v as f32),
            zoom: f("zoom").map(|v| v as f32),
            yaw: f("yaw").map(|v| v as f32),
        }),
        "animata/reseed" => Ok(Cmd::Reseed { seed: u("seed") }),
        "animata/set_timescale" => Ok(Cmd::SetClock {
            scale: f("scale").map(|v| v as f32),
            paused: b("paused"),
        }),
        "animata/render" => Ok(Cmd::Render { water: b("water"), topo: b("topo") }),
        "animata/screenshot" => Ok(Cmd::Screenshot(s("path").unwrap_or_else(|| "shot.png".into()))),
        other => Err(format!("unknown method: {other}")),
    }
}
