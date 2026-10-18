//! Lets other apps on this computer control Voicenux, like a plugin for OpenDeck or a Stream Deck,
//! or a script. One port on 127.0.0.1 speaks two ways:
//!
//! - WebSocket at `ws://127.0.0.1:41967`: send `{"id": 1, "action": "voice", "arg": "robot"}`,
//!   get `{"id": 1, "ok": true, "result": …}` back, and events like `{"event": "state", …}`
//!   whenever something changes.
//! - HTTP: `GET /api/state`, `/api/voices`, `/api/sounds`, `/api/icon/NAME`, and
//!   `POST /api/ACTION/ARG`, like `POST /api/sound/airhorn`.
//!
//! It is off until it is turned on in Settings. Only programs on this computer reach it, and web
//! pages are turned away by their origin, so a page on the internet can not play sounds into your
//! microphone. API.md lists every action.

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gtk::{gio, glib};
use serde_json::{Value, json};
use tungstenite::Message;

use crate::i18n::t_dyn;
use crate::state::{Change, St};

pub const PORT: u16 = 41967;
/// Goes up when a change breaks clients.
const VERSION: u32 = 1;

/// A question from a client, answered on the main thread, where the state lives.
struct Request {
    action: String,
    arg: Option<String>,
    reply: mpsc::Sender<Result<Value, String>>,
}

/// The WebSocket clients, each with the outbox for its events.
type Clients = Arc<Mutex<Vec<(u64, mpsc::Sender<String>)>>>;

struct Server {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Server {
    fn shut(self) {
        self.stop.store(true, Ordering::Relaxed);
        // Quick: the thread looks at the flag ten times a second. The port is free after it.
        let _ = self.thread.join();
    }
}

/// Runs the API while it is on in Settings, and tells the clients about every change.
pub fn start(st: &St) {
    let (tx, rx) = async_channel::unbounded::<Request>();
    let clients: Clients = Arc::default();
    glib::spawn_future_local({
        let st = st.clone();
        async move {
            while let Ok(req) = rx.recv().await {
                let _ = req.reply.send(handle(&st, &req.action, req.arg.as_deref()));
            }
        }
    });

    let server: Rc<RefCell<Option<Server>>> = Rc::default();
    let sync = Rc::new({
        let (st, server, clients) = (st.clone(), server.clone(), clients.clone());
        move || {
            let want = st.config.borrow().api;
            if want == server.borrow().is_some() {
                return;
            }
            if let Some(s) = server.take() {
                s.shut();
            }
            let error = if want {
                match serve(tx.clone(), clients.clone()) {
                    Ok(s) => {
                        server.replace(Some(s));
                        None
                    }
                    Err(e) => {
                        log::warn!("the API could not start: {e}");
                        Some(e.to_string())
                    }
                }
            } else {
                None
            };
            *st.api.borrow_mut() = crate::state::ApiStatus { running: server.borrow().is_some(), error, clients: 0 };
            st.notify(Change::Api);
        }
    });
    sync();

    // What the clients saw last, so they only hear of real changes. Forgotten without clients:
    // a new client asks for all of it anyway.
    let last: Rc<RefCell<[Option<String>; 4]>> = Rc::default();
    let send = {
        let (clients, last) = (clients.clone(), last.clone());
        move |slot: usize, event: &str, data: Value| {
            let text = data.to_string();
            let mut last = last.borrow_mut();
            if last[slot].as_deref() == Some(text.as_str()) {
                return;
            }
            last[slot] = Some(text);
            let message = json!({ "event": event, "data": data }).to_string();
            clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, out)| out.send(message.clone()).is_ok());
        }
    };
    let send = Rc::new(send);
    st.subscribe({
        let (st, clients, send) = (st.clone(), clients.clone(), send.clone());
        move |c| {
            if matches!(c, Change::Settings) {
                sync();
            }
            if clients.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                return;
            }
            match c {
                Change::Voices | Change::VoiceEdited(_) | Change::Favorites => send(1, "voices", voices(&st)),
                Change::Sounds => send(2, "sounds", sounds(&st)),
                _ => {}
            }
            if !matches!(c, Change::Notice(_) | Change::Api | Change::Sounds | Change::SoundRemoved { .. }) {
                send(0, "state", state(&st));
            }
        }
    });

    // Sounds start and stop without a change: look ten times a second, while someone listens.
    let count = Cell::new(0);
    let st = st.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let n = clients.lock().unwrap_or_else(|e| e.into_inner()).len();
        if n != count.replace(n) {
            st.api.borrow_mut().clients = n;
            st.notify(Change::Api);
        }
        if n == 0 {
            *last.borrow_mut() = Default::default();
        } else {
            send(3, "playing", json!(st.playing_sounds()));
        }
        glib::ControlFlow::Continue
    });
}

/// Opens the port and serves clients on their own threads.
fn serve(tx: async_channel::Sender<Request>, clients: Clients) -> std::io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", PORT))?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread = std::thread::spawn({
        let stop = stop.clone();
        move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (tx, clients, stop) = (tx.clone(), clients.clone(), stop.clone());
                        std::thread::spawn(move || {
                            if let Err(e) = client(stream, &tx, &clients, &stop) {
                                log::debug!("API client: {e}");
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                    Err(e) => {
                        log::warn!("API: {e}");
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }
    });
    Ok(Server { stop, thread })
}

/// One connection: a WebSocket, or a single HTTP request.
fn client(mut stream: TcpStream, tx: &async_channel::Sender<Request>, clients: &Clients, stop: &AtomicBool) -> anyhow::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    // Look at the head without taking it: a WebSocket handshake reads it again.
    let mut buf = vec![0u8; 8192];
    let began = Instant::now();
    let n = loop {
        let n = stream.peek(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") || n == buf.len() {
            break n;
        }
        anyhow::ensure!(began.elapsed() < Duration::from_secs(5), "no complete request");
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut headers = [httparse::EMPTY_HEADER; 48];
    let mut req = httparse::Request::new(&mut headers);
    let httparse::Status::Complete(head_len) = req.parse(&buf[..n])? else { anyhow::bail!("request too long") };
    let header = |name: &str| {
        req.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).and_then(|h| std::str::from_utf8(h.value).ok())
    };
    let ok = allowed(header("host"), header("origin"));
    let upgrade = header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
    let body_len = header("content-length").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
    let (method, path) = (req.method.unwrap_or("GET").to_string(), req.path.unwrap_or("/").to_string());
    if !ok {
        return respond(&mut stream, 403, "application/json", br#"{"ok":false,"error":"only apps on this computer may use the API"}"#);
    }
    if upgrade {
        return websocket(stream, tx, clients, stop);
    }
    anyhow::ensure!(body_len <= 64 * 1024, "body too long");
    let mut head = vec![0u8; head_len];
    stream.read_exact(&mut head)?;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body)?;
    http(&mut stream, &method, &path, &body, tx)
}

/// Web pages may not use the API. Programs send no origin; plugin views of OpenDeck or a Stream
/// Deck run on this computer. And the address must be ours, so a page can not reach us under a
/// name of its own (DNS rebinding).
fn allowed(host: Option<&str>, origin: Option<&str>) -> bool {
    fn local(host: &str) -> bool {
        let name = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => host.split(':').next().unwrap_or(""),
        };
        matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1" | "tauri.localhost")
    }
    let host_ok = host.is_none_or(local);
    let origin_ok = origin.is_none_or(|o| {
        let o = o.trim();
        if o.starts_with("file://") || o.starts_with("tauri://") {
            return true;
        }
        match o.strip_prefix("http://").or_else(|| o.strip_prefix("https://")) {
            Some(rest) => local(rest.split('/').next().unwrap_or("")),
            None => false,
        }
    });
    host_ok && origin_ok
}

fn websocket(stream: TcpStream, tx: &async_channel::Sender<Request>, clients: &Clients, stop: &AtomicBool) -> anyhow::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut ws = tungstenite::accept(stream).map_err(|e| anyhow::anyhow!("handshake: {e}"))?;
    // Short, so events go out between questions.
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(50)))?;
    let (out, inbox) = mpsc::channel::<String>();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let hello = json!({ "event": "hello", "data": { "app": "voicenux", "version": env!("CARGO_PKG_VERSION"), "api": VERSION } });
    ws.send(Message::text(hello.to_string()))?;
    for (event, action) in [("state", "state"), ("voices", "voices"), ("sounds", "sounds")] {
        if let Ok(data) = ask(tx, action, None) {
            ws.send(Message::text(json!({ "event": event, "data": data }).to_string()))?;
        }
    }
    clients.lock().unwrap_or_else(|e| e.into_inner()).push((id, out));
    let result = (|| -> anyhow::Result<()> {
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = ws.close(None);
                let _ = ws.flush();
                return Ok(());
            }
            match ws.read() {
                Ok(Message::Text(text)) => ws.send(Message::text(command(tx, text.as_str())))?,
                Ok(Message::Close(_)) => return Ok(()),
                // Pings get their pong by themselves.
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
            while let Ok(event) = inbox.try_recv() {
                ws.send(Message::text(event))?;
            }
            let _ = ws.flush();
        }
    })();
    clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, _)| *i != id);
    result
}

/// A WebSocket message: `{"id": …, "action": "…", "arg": …}`. The answer carries the same id.
fn command(tx: &async_channel::Sender<Request>, text: &str) -> String {
    let msg: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let arg = match msg.get("arg") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Bool(b)) => Some(if *b { "on" } else { "off" }.to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let result = match msg.get("action").and_then(Value::as_str) {
        None => Err("a message needs an \"action\"".to_string()),
        Some("icon") => match arg.as_deref().and_then(png) {
            Some(bytes) => Ok(json!({ "png": base64(&bytes) })),
            None => Err("no such icon".to_string()),
        },
        Some(action) => ask(tx, action, arg),
    };
    match result {
        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    }
    .to_string()
}

fn http(stream: &mut TcpStream, method: &str, path: &str, body: &[u8], tx: &async_channel::Sender<Request>) -> anyhow::Result<()> {
    let path = path.split('?').next().unwrap_or("");
    let parts: Vec<String> = path.split('/').filter(|p| !p.is_empty()).map(unescape).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let result = match (method, parts.as_slice()) {
        ("GET", ["api"] | ["api", "state"]) => ask(tx, "state", None),
        ("GET", ["api", list @ ("voices" | "sounds")]) => ask(tx, list, None),
        ("GET", ["api", "icon", name]) => {
            return match png(name.trim_end_matches(".png")) {
                Some(bytes) => respond(stream, 200, "image/png", &bytes),
                None => respond(stream, 404, "application/json", br#"{"ok":false,"error":"no such icon"}"#),
            };
        }
        // Side effects only on POST: web pages can not send those to us unseen.
        ("POST", ["api", action]) => {
            let text = String::from_utf8_lossy(body);
            let arg = match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(o)) => o.get("arg").map(|a| a.as_str().map_or_else(|| a.to_string(), String::from)),
                _ => Some(text.trim().to_string()).filter(|t| !t.is_empty()),
            };
            ask(tx, action, arg)
        }
        ("POST", ["api", action, arg]) => ask(tx, action, Some(arg.to_string())),
        _ => return respond(stream, 404, "application/json", br#"{"ok":false,"error":"see API.md for the paths"}"#),
    };
    let (status, json) = match result {
        Ok(result) => (200, json!({ "ok": true, "result": result })),
        Err(error) => (400, json!({ "ok": false, "error": error })),
    };
    respond(stream, status, "application/json", json.to_string().as_bytes())
}

fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        _ => "Not Found",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

/// Asks the main thread.
fn ask(tx: &async_channel::Sender<Request>, action: &str, arg: Option<String>) -> Result<Value, String> {
    let (reply, answer) = mpsc::channel();
    tx.send_blocking(Request { action: action.to_string(), arg, reply }).map_err(|_| "Voicenux is closing".to_string())?;
    answer.recv_timeout(Duration::from_secs(5)).map_err(|_| "no answer".to_string())?
}

/// Does an action on the main thread. Actions that change something answer with the new state.
fn handle(st: &St, action: &str, arg: Option<&str>) -> Result<Value, String> {
    let need = || arg.map(str::trim).filter(|a| !a.is_empty()).ok_or_else(|| format!("{action} needs an \"arg\""));
    match action {
        "state" => return Ok(state(st)),
        "voices" => return Ok(voices(st)),
        "sounds" => return Ok(sounds(st)),
        "voice" => {
            let id = find_voice(st, need()?)?;
            st.set_voice(&id);
        }
        "random-voice" => st.random_voice(),
        "next-voice" => st.step_voice(true),
        "previous-voice" => st.step_voice(false),
        // Read the switch first: a borrow in the argument would last through the call.
        "voice-changer" => {
            let on = switch(arg, st.config.borrow().voice_on)?;
            st.set_voice_on(on);
        }
        "hear-myself" => {
            let on = switch(arg, st.config.borrow().hear_myself)?;
            st.set_hear_myself(on);
        }
        "background" => {
            let on = switch(arg, st.config.borrow().background)?;
            st.set_background(on);
        }
        "mute" => st.set_muted(switch(arg, st.muted.get())?),
        "favorite" => {
            let id = find_voice(st, need()?)?;
            st.toggle_favorite(&id);
        }
        "sound" => {
            let id = find_sound(st, need()?)?;
            st.play_sound(&id);
        }
        "stop-sound" => {
            let id = find_sound(st, need()?)?;
            st.stop_sound(&id);
        }
        "stop-sounds" => st.stop_sounds(),
        "sound-volume" => {
            let v: f32 = need()?.trim_end_matches('%').trim().parse().map_err(|_| "sound-volume needs 0 to 100".to_string())?;
            st.update(|c| c.sound_volume = (v / 100.0).clamp(0.0, 1.0));
        }
        "hear-sounds" => {
            let on = switch(arg, st.config.borrow().hear_sounds)?;
            st.update(|c| c.hear_sounds = on);
        }
        _ => return Err(format!("unknown action \"{action}\", see API.md")),
    }
    Ok(state(st))
}

/// "on", "off" or "toggle" (also without an argument).
fn switch(arg: Option<&str>, now: bool) -> Result<bool, String> {
    match arg.map(|a| a.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "toggle") => Ok(!now),
        Some("on" | "true" | "1") => Ok(true),
        Some("off" | "false" | "0") => Ok(false),
        Some(other) => Err(format!("\"{other}\" is not on, off or toggle")),
    }
}

/// A voice by id or by name, in any language.
fn find_voice(st: &St, name: &str) -> Result<String, String> {
    let name = name.to_lowercase();
    st.voices
        .borrow()
        .iter()
        .find(|v| v.id == name || v.name.to_lowercase() == name || t_dyn(&v.name).to_lowercase() == name)
        .map(|v| v.id.clone())
        .ok_or_else(|| format!("no voice called \"{name}\""))
}

fn find_sound(st: &St, name: &str) -> Result<String, String> {
    let name = name.to_lowercase();
    st.library
        .borrow()
        .sound
        .iter()
        .find(|s| s.id == name || s.name.to_lowercase() == name || t_dyn(&s.name).to_lowercase() == name)
        .map(|s| s.id.clone())
        .ok_or_else(|| format!("no sound called \"{name}\""))
}

fn state(st: &St) -> Value {
    let c = st.config.borrow();
    let v = st.current();
    json!({
        "voice": { "id": v.id, "name": t_dyn(&v.name), "icon": v.icon, "category": v.category },
        "voice_changer": c.voice_on,
        "hear_myself": c.hear_myself,
        "background": c.background,
        "mute": st.muted.get(),
        "sound_volume": (c.sound_volume * 100.0).round() as i64,
        "hear_sounds": c.hear_sounds,
        "playing": st.playing_sounds(),
        "apps": st.users.borrow().clone(),
    })
}

fn voices(st: &St) -> Value {
    let list: Vec<Value> = st
        .voices
        .borrow()
        .iter()
        .map(|v| {
            json!({
                "id": v.id,
                "name": t_dyn(&v.name),
                "icon": v.icon,
                "category": v.category,
                "favorite": st.is_favorite(&v.id),
                "custom": !v.builtin,
            })
        })
        .collect();
    json!(list)
}

fn sounds(st: &St) -> Value {
    let list: Vec<Value> = st
        .library
        .borrow()
        .sound
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "name": t_dyn(&s.name),
                "icon": s.icon,
                "mode": st.sound_mode(s),
                "volume": (s.volume * 100.0).round() as i64,
            })
        })
        .collect();
    json!(list)
}

/// A picture of a voice or sound, as PNG. Only plain names, so nothing outside the pictures.
fn png(name: &str) -> Option<Vec<u8>> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return None;
    }
    ["icons", "emoji"].iter().find_map(|dir| {
        let path = format!("{}/{dir}/{name}.png", crate::ui::RESOURCE);
        gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).ok().map(|b| b.to_vec())
    })
}

/// "%20" back to a space, for names in a path.
fn unescape(part: &str) -> String {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_callers() {
        assert!(allowed(Some("127.0.0.1:41967"), None));
        assert!(allowed(Some("localhost:41967"), Some("http://localhost:57118")));
        assert!(allowed(None, Some("tauri://localhost")));
        assert!(allowed(Some("[::1]:41967"), Some("file://")));
        assert!(!allowed(Some("127.0.0.1:41967"), Some("https://evil.example")));
        assert!(!allowed(Some("127.0.0.1:41967"), Some("null")));
        assert!(!allowed(Some("evil.example:41967"), None));
        assert!(!allowed(Some("127.0.0.1:41967"), Some("http://localhost.evil.example")));
    }

    #[test]
    fn switches() {
        assert_eq!(switch(None, true), Ok(false));
        assert_eq!(switch(Some("on"), true), Ok(true));
        assert_eq!(switch(Some("Off"), true), Ok(false));
        assert!(switch(Some("maybe"), true).is_err());
    }

    #[test]
    fn small_helpers() {
        assert_eq!(unescape("deep%20voice"), "deep voice");
        assert_eq!(unescape("a+b"), "a b");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(png("../secret"), None);
    }
}
