//! Optional: control an ADI-2 that is connected to an ADI-2-Pi, through the bridge's existing
//! WebSocket API (docs/API.md of ADI-2-Pi, API version 1). Nothing on the Pi needs changing:
//! this is an ordinary native client (no Origin header).
//!
//! When the bridge has an access key set, the key goes along as header `X-ADI2PI-Key` on the
//! WebSocket upgrade. A missing or wrong key closes the socket with 4401: the app then stops
//! and asks for the key (no reconnect loop, the bridge locks out after 10 wrong keys a minute);
//! 4429 (locked out) waits 61 s.
//!
//! A bridge set to "key needed for changes only" accepts a keyless connection with
//! `hello.access: "read"`: values are shown, commands are refused until the key is entered.
//! `{ t: "access" }` changes that for an open connection.
//!
//! The bridge works with the same protocol integers and "addr.idx" keys as the USB engine, so
//! its messages map straight onto the app's model. The bridge's own volume limits apply on top.

use crate::device::In;
use crate::model::{Command, DeviceInfo, Origin, Preset, Problem, Update};
use crate::protocol;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const RETRY: Duration = Duration::from_secs(3);

/// "host", "host:port", "http://host:port/…" → (host, port)
pub fn parse_address(addr: &str) -> Option<(String, u16)> {
    let a = addr.trim();
    let a = a.strip_prefix("http://").or_else(|| a.strip_prefix("ws://")).unwrap_or(a);
    let a = a.split('/').next()?.trim();
    if a.is_empty() {
        return None;
    }
    if let Some(rest) = a.strip_prefix('[') {
        // [IPv6]:port
        let (h, p) = rest.split_once(']')?;
        let port = p.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(80))?;
        return Some((h.to_string(), port));
    }
    match a.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Some((h.to_string(), p.parse().ok()?)),
        _ => Some((a.to_string(), 80)),
    }
}

/// Why a connection ended or couldn't be made.
#[derive(Debug, PartialEq)]
enum Ended {
    /// network problem, bridge restarted …: try again
    Lost(String),
    /// access key missing or wrong (WS close 4401 / HTTP 401): stop until the settings change
    Auth,
    /// too many wrong keys (WS close 4429 / HTTP 429): wait 61 s
    Locked,
}

pub const KEY_HEADER: &str = "X-ADI2PI-Key";

fn connect(host: &str, port: u16, key: &str) -> Result<WebSocket<TcpStream>, Ended> {
    use tungstenite::client::IntoClientRequest;
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|e| Ended::Lost(format!("can't resolve {host} ({e})")))?.collect();
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
            Ok(stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let host_hdr = if a.is_ipv6() { format!("[{host}]") } else { host.to_string() };
                let url = format!("ws://{host_hdr}:{port}/ws");
                let mut req = url.into_client_request().map_err(|e| Ended::Lost(e.to_string()))?;
                if !key.is_empty() {
                    let v = tungstenite::http::HeaderValue::from_str(key).map_err(|_| Ended::Auth)?;
                    req.headers_mut().insert(KEY_HEADER, v);
                }
                let (ws, _) = tungstenite::client(req, stream).map_err(|e| match e {
                    tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) if r.status() == 401 => Ended::Auth,
                    tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) if r.status() == 429 => Ended::Locked,
                    e => Ended::Lost(format!("WebSocket: {e}")),
                })?;
                ws.get_ref().set_nodelay(true).ok();
                return Ok(ws);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(Ended::Lost(last))
}

/// The unit's status message: `unitStatus` (bridge 1.0.0+), `status` on earlier bridges
/// (there it is the hex string; now `status` is only an error's number).
fn unit_status(m: &Value) -> Option<protocol::Status> {
    to_status(&m["unitStatus"]).or_else(|| to_status(&m["status"]))
}

fn to_device(v: &Value, label: &str, read_only: bool) -> DeviceInfo {
    let problem = v.get("problem").and_then(|p| {
        Some(Problem::new(p.get("code")?.as_str().unwrap_or("pi"), p.get("text")?.as_str().unwrap_or("").to_string()))
    });
    DeviceInfo {
        connected: v["connected"].as_bool().unwrap_or(false),
        ready: v["ready"].as_bool().unwrap_or(false),
        answering: v["answering"].as_bool().unwrap_or(false),
        id: v["id"].as_u64().and_then(|x| u8::try_from(x).ok()),
        model: v["model"].as_str().map(Into::into),
        port: Some(label.to_string()),
        problem,
        read_only,
    }
}

fn to_params(v: &Value) -> Vec<(String, i32)> {
    v.as_object()
        .map(|m| m.iter().filter(|(k, _)| crate::model::valid_key(k)).filter_map(|(k, v)| Some((k.clone(), i32::try_from(v.as_i64()?).ok()?))).collect())
        .unwrap_or_default()
}

fn side(v: &Value) -> BTreeMap<u8, i32> {
    v.as_object()
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.parse().ok()?, i32::try_from(v.as_i64()?).ok()?))).collect())
        .unwrap_or_default()
}

fn to_presets(v: &Value) -> Vec<Preset> {
    let mut out = crate::model::empty_presets();
    for p in v.as_array().into_iter().flatten() {
        let n = p["n"].as_u64().unwrap_or(0) as usize;
        if (1..=20).contains(&n) {
            out[n - 1] = Preset {
                n: n as u8,
                name: p["name"].as_str().unwrap_or("").into(),
                empty: p["empty"].as_bool(),
                dual: p["dual"].as_bool().unwrap_or(false),
                l: side(&p["L"]),
                r: side(&p["R"]),
            };
        }
    }
    out
}

fn to_status(hex: &Value) -> Option<protocol::Status> {
    let h = hex.as_str()?;
    let bytes: Vec<u8> = h.split_whitespace().filter_map(|b| u8::from_str_radix(b, 16).ok()).collect();
    protocol::parse_status(&bytes)
}

fn setups(v: &Value) -> Option<Vec<String>> {
    Some(v.as_array()?.iter().map(|s| s.as_str().unwrap_or("").to_string()).collect())
}

fn command_json(c: Command) -> Value {
    let map = |m: BTreeMap<u8, i32>| -> Map<String, Value> { m.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect() };
    match c {
        Command::Set(items) => {
            let p: Map<String, Value> = items.into_iter().map(|(k, v)| (k, json!(v))).collect();
            json!({ "t": "set", "p": p })
        }
        Command::LoadSetup(n) => json!({ "t": "setup", "action": "load", "n": n }),
        Command::SaveSetup(n, name) => match name {
            Some(name) => json!({ "t": "setup", "action": "save", "n": n, "name": name }),
            None => json!({ "t": "setup", "action": "save", "n": n }),
        },
        Command::RenameSetup(n, name) => json!({ "t": "setup", "action": "rename", "n": n, "name": name }),
        Command::LoadFactory => json!({ "t": "setup", "action": "factory" }),
        Command::StorePreset { n, name, l, r, dual } => {
            let mut v = json!({ "t": "storePreset", "n": n, "L": map(l), "R": map(r), "dual": dual });
            if let Some(name) = name {
                v["name"] = json!(name);
            }
            v
        }
        Command::RenamePreset(n, name) => json!({ "t": "renamePreset", "n": n, "name": name }),
        Command::Refresh => json!({ "t": "refresh" }),
    }
}

/// Wait, staying responsive to Stop. false = stop.
fn wait(rx: &Receiver<In>, d: Option<Duration>) -> bool {
    let until = d.map(|d| Instant::now() + d);
    loop {
        if until.is_some_and(|u| Instant::now() >= u) {
            return true;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(In::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
            _ => {}
        }
    }
}

pub fn run(address: String, key: String, hub_rx: Receiver<In>, out: Box<dyn Fn(Update) + Send>) {
    let label = format!("ADI-2-Pi at {address}");
    // commands come through a second channel plus a wake-up byte on a pipe, so a session can
    // wait on the socket and on commands at the same time (poll)
    let (pipe_w, pipe_r) = match UnixStream::pair() {
        Ok(p) => p,
        Err(e) => {
            out(Update::Device(DeviceInfo { problem: Some(Problem::new("pi", format!("pipe: {e}"))), ..Default::default() }));
            return;
        }
    };
    pipe_r.set_nonblocking(true).ok();
    let (ctx, rx) = std::sync::mpsc::channel::<In>();
    std::thread::Builder::new()
        .name("adi2-pi-cmds".into())
        .spawn(move || {
            let mut pipe_w = pipe_w;
            for m in hub_rx.iter() {
                let stop = matches!(m, In::Stop);
                if ctx.send(m).is_err() {
                    return;
                }
                let _ = std::io::Write::write(&mut pipe_w, &[1]);
                if stop {
                    return;
                }
            }
            // the hub dropped its sender
            let _ = ctx.send(In::Stop);
            let _ = std::io::Write::write(&mut pipe_w, &[1]);
        })
        .ok();
    let Some((host, port)) = parse_address(&address) else {
        out(Update::Device(DeviceInfo { problem: Some(Problem::new("pi", format!("“{address}” is not an address"))), ..Default::default() }));
        let _ = rx.recv();
        return;
    };
    let offline = |code: &str, text: String| DeviceInfo { port: Some(label.clone()), problem: Some(Problem::new(code, text)), ..Default::default() };
    out(Update::Device(offline("pi", format!("Connecting to {label}…"))));
    loop {
        let ended = match connect(&host, port, &key) {
            Ok(ws) => session(ws, &label, &rx, &pipe_r, &out),
            Err(e) => e,
        };
        match ended {
            Ended::Auth => {
                let text = if key.is_empty() {
                    format!("ADI-2-Pi at {address} needs an access key. Enter it under Settings → Connection.")
                } else {
                    format!("Access key for ADI-2-Pi at {address} is wrong. Check it under Settings → Connection.")
                };
                out(Update::Device(offline("pi-auth", text)));
                // no retry loop: the bridge locks out after 10 wrong keys; a new key restarts this
                if !wait(&rx, None) {
                    return;
                }
            }
            Ended::Locked => {
                out(Update::Device(offline("pi-locked", format!("ADI-2-Pi at {address}: too many wrong access keys. Trying again in a minute."))));
                if !wait(&rx, Some(Duration::from_secs(61))) {
                    return;
                }
            }
            Ended::Lost(e) if e == "stop" => return,
            Ended::Lost(e) => {
                out(Update::Device(offline("pi", format!("Can't reach ADI-2-Pi at {address}: {e} – trying again…"))));
                if !wait(&rx, Some(RETRY)) {
                    return;
                }
            }
        }
    }
}

/// One connection, until it ends. Waits with poll(2) on the socket and on a pipe that the
/// forwarder thread writes to when a command arrives, so commands go out at once and the
/// thread sleeps otherwise.
fn session(mut ws: WebSocket<TcpStream>, label: &str, rx: &Receiver<In>, pipe_r: &UnixStream, out: &(dyn Fn(Update) + Send)) -> Ended {
    use std::io::Read as _;
    use std::os::unix::io::AsRawFd;

    out(Update::Reset);
    // commands queued for a connection that ended are not replayed on this one; a Stop that
    // arrived while connecting ends this session before it starts
    while let Ok(m) = rx.try_recv() {
        if matches!(m, In::Stop) {
            let _ = ws.close(None);
            return Ended::Lost("stop".into());
        }
    }
    let sock_fd = ws.get_ref().as_raw_fd();
    ws.get_ref().set_read_timeout(Some(Duration::from_secs(2))).ok();
    ws.get_ref().set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut pipe_r = pipe_r;
    {
        let mut alive = Instant::now();
        let mut hello_seen = false;
        let mut auth_err = false;
        let mut read_only = false;
        let mut device = DeviceInfo::default();
        // the first messages can come with the handshake response and sit in the reader's buffer
        let mut first = true;
        let result = 'session: loop {
            let mut fds = [
                libc::pollfd { fd: sock_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: pipe_r.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            ];
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, if first { 0 } else { 1000 }) };
            if n < 0 && std::io::Error::last_os_error().kind() != ErrorKind::Interrupted {
                break Ended::Lost("poll failed".into());
            }
            if fds[1].revents != 0 {
                let mut buf = [0u8; 64];
                while matches!(pipe_r.read(&mut buf), Ok(n) if n > 0) {}
                let mut stop = false;
                while let Ok(m) = rx.try_recv() {
                    match m {
                        In::Cmd(c) => {
                            if let Err(e) = ws.send(Message::text(command_json(c).to_string())) {
                                break 'session Ended::Lost(format!("send failed: {e}"));
                            }
                        }
                        In::Stop => stop = true,
                        In::Link(_) => {}
                    }
                }
                if stop {
                    let _ = ws.close(None);
                    break Ended::Lost("stop".into());
                }
            }
            if fds[0].revents == 0 && !first {
                // the bridge pings every 25 s
                if alive.elapsed() > Duration::from_secs(60) {
                    break Ended::Lost("no answer for 60 s".into());
                }
                continue;
            }
            // Read everything there is: several messages can arrive in one TCP read, and what
            // the WebSocket reader has buffered doesn't wake poll() again.
            first = false;
            ws.get_ref().set_nonblocking(true).ok();
            loop {
                match ws.read() {
                    Ok(Message::Text(t)) => {
                        alive = Instant::now();
                        let Ok(m) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                        let kind = m["t"].as_str().unwrap_or("");
                        match kind {
                            "hello" | "device" => {
                                if kind == "hello" {
                                    hello_seen = true;
                                    read_only = m["access"] == "read";
                                }
                                device = to_device(&m["device"], label, read_only);
                                out(Update::Device(device.clone()));
                            }
                            "access" => {
                                read_only = m["access"] == "read";
                                device.read_only = read_only;
                                out(Update::Device(device.clone()));
                            }
                            "state" => {
                                device = to_device(&m["device"], label, read_only);
                                out(Update::Device(device.clone()));
                                out(Update::Params(to_params(&m["p"]), Origin::Device));
                                if let Some(s) = setups(&m["setups"]) {
                                    out(Update::Setups(s));
                                }
                                out(Update::Presets(to_presets(&m["eqPresets"])));
                                if let Some(s) = unit_status(&m) {
                                    out(Update::Status(s));
                                }
                            }
                            "p" => out(Update::Params(to_params(&m["p"]), Origin::Device)),
                            "setups" => {
                                if let Some(s) = setups(&m["setups"]) {
                                    out(Update::Setups(s));
                                }
                            }
                            "eqpresets" => out(Update::Presets(to_presets(&m["eqPresets"]))),
                            "status" => {
                                if let Some(s) = unit_status(&m) {
                                    out(Update::Status(s));
                                }
                            }
                            // before the hello: the key was refused and a close follows
                            "err" if m["status"] == 401 && !hello_seen => auth_err = true,
                            "err" if m["status"] == 429 => break 'session Ended::Locked,
                            // a refused set: the model took it already, so read the state again
                            "err" if m["status"] == 400 || m["status"] == 409 || (m["status"] == 401 && read_only) => {
                                let _ = ws.send(Message::text(json!({ "t": "get" }).to_string()));
                            }
                            _ => {}
                        }
                    }
                    Ok(Message::Ping(_)) => {
                        alive = Instant::now();
                        // the pong is queued by read(); send it now, not with the next read or write
                        // (the bridge drops a connection that answers no ping within 50 s)
                        let _ = ws.flush(); // (non-blocking here; what doesn't go out now is flushed below)
                    }
                    Ok(Message::Pong(_)) => alive = Instant::now(),
                    Ok(Message::Close(frame)) => {
                        let code = frame.map(|f| u16::from(f.code)).unwrap_or(0);
                        break 'session match code {
                            4401 => Ended::Auth,
                            4429 => Ended::Locked,
                            _ if auth_err => Ended::Auth,
                            _ => Ended::Lost("connection closed".into()),
                        };
                    }
                    Ok(_) => {}
                    // nothing more buffered or on the socket
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => break,
                    Err(_) if auth_err => break 'session Ended::Auth,
                    Err(e) => break 'session Ended::Lost(e.to_string()),
                }
            }
            ws.get_ref().set_nonblocking(false).ok();
            let _ = ws.flush();
        };
        result
    }
}

/// An ADI-2-Pi found on the network.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Found {
    pub name: String,
    /// what to connect to: "<host>.local[:port]", or the IPv4 address
    pub address: String,
    pub ip: Option<String>,
}

/// Look for ADI-2-Pi bridges (mDNS `_adi2._tcp`) for about `wait`.
pub fn discover(wait: Duration) -> Result<Vec<Found>, String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let rx = mdns.browse("_adi2._tcp.local.").map_err(|e| e.to_string())?;
    let until = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let host = info.get_hostname().trim_end_matches('.').to_string();
                let port = info.get_port();
                let address = if port == 80 { host.clone() } else { format!("{host}:{port}") };
                let ip = info.get_addresses_v4().into_iter().next().map(|a| if port == 80 { a.to_string() } else { format!("{a}:{port}") });
                let name = info.get_fullname().split("._adi2").next().unwrap_or(&host).replace("\\032", " ");
                if !found.iter().any(|f| f.address == address) {
                    found.push(Found { name, address, ip });
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = mdns.shutdown();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses() {
        assert_eq!(parse_address("adi2-pi.local"), Some(("adi2-pi.local".into(), 80)));
        assert_eq!(parse_address("http://192.168.1.5:8080/"), Some(("192.168.1.5".into(), 8080)));
        assert_eq!(parse_address("[fe80::1]:81"), Some(("fe80::1".into(), 81)));
        assert_eq!(parse_address(""), None);
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    #[test]
    fn unit_status_and_older_bridges() {
        let hex = "01 00 00 00 00 00 00 00 00 02 02 0A 00 00";
        // bridge 1.0.0+: unitStatus; status is only an error's number
        let m = json!({ "t": "state", "unitStatus": hex, "status": 401 });
        assert_eq!(unit_status(&m).unwrap().mode, 2);
        // earlier bridges: status was the hex string
        let m = json!({ "t": "state", "status": hex });
        assert_eq!(unit_status(&m).unwrap().hex, hex);
        assert!(unit_status(&json!({ "t": "err", "status": 401 })).is_none());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::model::Update;
    use std::sync::{Arc, Mutex};

    /// Needs the stand-in bridge: `node fake-bridge.js 8098 s3cret`.
    #[test]
    #[ignore]
    fn commands_reach_the_bridge_at_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            run("127.0.0.1:8098".into(), "s3cret".into(), rx, Box::new(move |u| match u {
                Update::Params(p, _) => s2.lock().unwrap().push(format!("{p:?}")),
                Update::Device(d) => s2.lock().unwrap().push(format!("device ro={}", d.read_only)),
                _ => {}
            }))
        });
        std::thread::sleep(Duration::from_millis(800));
        let t0 = Instant::now();
        tx.send(In::Cmd(Command::Set(vec![("3.12".into(), -123)]))).unwrap();
        let mut got = false;
        while t0.elapsed() < Duration::from_secs(2) {
            if seen.lock().unwrap().iter().any(|s| s.contains("-123")) {
                got = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(got, "set echoed back by the bridge");
        assert!(t0.elapsed() < Duration::from_millis(100), "round trip {:?}", t0.elapsed());
        assert!(seen.lock().unwrap().iter().any(|s| s == "device ro=false"));
        let _ = tx.send(In::Stop);
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::model::Update;
    use std::sync::{Arc, Mutex};

    /// Needs the stand-in bridge in view mode: `node fake-bridge.js 8097 s3cret view`.
    #[test]
    #[ignore]
    fn keyless_connection_is_read_only() {
        let (tx, rx) = std::sync::mpsc::channel();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            run("127.0.0.1:8097".into(), String::new(), rx, Box::new(move |u| match u {
                Update::Params(p, _) => s2.lock().unwrap().push(format!("params {}", p.len())),
                Update::Device(d) => s2.lock().unwrap().push(format!("device connected={} ro={}", d.connected, d.read_only)),
                _ => {}
            }))
        });
        // (hello and state arrive with the handshake response; they must not wait for a ping)
        std::thread::sleep(Duration::from_millis(800));
        let v = seen.lock().unwrap().clone();
        assert!(v.iter().any(|s| s == "device connected=true ro=true"), "{v:?}");
        assert!(v.iter().any(|s| s.starts_with("params")), "values arrive read-only: {v:?}");
        let _ = tx.send(In::Stop);
    }

    /// The bridge pings every 25 s and drops a connection without a pong within 50 s; the
    /// session must stay up through that. Needs the view-mode bridge on 8097. Takes 60 s.
    #[test]
    #[ignore]
    fn pings_are_answered() {
        let (tx, rx) = std::sync::mpsc::channel();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            run("127.0.0.1:8097".into(), String::new(), rx, Box::new(move |u| {
                if let Update::Device(d) = u {
                    s2.lock().unwrap().push(format!("connected={}", d.connected));
                }
            }))
        });
        std::thread::sleep(Duration::from_secs(60));
        let v = seen.lock().unwrap().clone();
        // ("connected=false" before the first connect is the "Connecting…" state)
        let up = v.iter().position(|s| s == "connected=true").expect("connected");
        assert!(!v[up..].iter().any(|s| s == "connected=false"), "dropped: {v:?}");
        let _ = tx.send(In::Stop);
    }
}
