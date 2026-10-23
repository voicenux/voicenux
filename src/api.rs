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
//! microphone. And only apps you allowed: the first time an app connects, Voicenux asks you. It
//! finds the program on the other end in /proc, so apps need no password. API.md lists every
//! action.

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gtk::{gio, glib};
use serde_json::{Value, json};
use tungstenite::Message;

use crate::config::ApiApp;
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::PlayMode;
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

/// A connection that wants in, answered on the main thread: may this app use the API?
struct Access {
    peer: Peer,
    reply: mpsc::Sender<bool>,
}

/// Where requests and questions go to the main thread.
#[derive(Clone)]
struct Main {
    requests: async_channel::Sender<Request>,
    access: async_channel::Sender<Access>,
}

/// The program on the other end of a connection.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    /// What the app is remembered by: the program, and the script for interpreters like Python.
    /// Empty when the program could not be found, like for apps of another user.
    pub key: String,
    /// A short name for people, like "curl" or "python3 bot.py". Empty with the key.
    pub name: String,
}

thread_local! {
    /// Apps Voicenux is asking you about, with the connections that wait for the answer.
    static ASKING: RefCell<Vec<(Peer, Vec<mpsc::Sender<bool>>)>> = const { RefCell::new(Vec::new()) };
    /// Answers that last until Voicenux quits, or the API is turned off: apps you turned away,
    /// and unknown apps.
    static ANSWERED: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
}

/// The WebSocket clients: each with the key of its app and the outbox for its events.
type Clients = Arc<Mutex<Vec<(u64, String, mpsc::Sender<String>)>>>;

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
    let (access, asked) = async_channel::unbounded::<Access>();
    let tx = Main { requests: tx, access };
    let clients: Clients = Arc::default();
    glib::spawn_future_local({
        let st = st.clone();
        async move {
            while let Ok(req) = rx.recv().await {
                let _ = req.reply.send(handle(&st, &req.action, req.arg.as_deref()));
            }
        }
    });
    glib::spawn_future_local({
        let st = st.clone();
        async move {
            while let Ok(a) = asked.recv().await {
                may(&st, a.peer, a.reply);
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
            // Turning it off and on again asks anew about the apps you turned away.
            ANSWERED.with_borrow_mut(Vec::clear);
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
            clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, _, out)| out.send(message.clone()).is_ok());
        }
    };
    let send = Rc::new(send);
    st.subscribe({
        let (st, clients, send) = (st.clone(), clients.clone(), send.clone());
        move |c| {
            if matches!(c, Change::Settings) {
                sync();
                // An app you took back leaves: dropping its outbox closes its socket.
                clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, key, _)| welcome(&st, key));
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
fn serve(tx: Main, clients: Clients) -> std::io::Result<Server> {
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
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
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
fn client(mut stream: TcpStream, main: &Main, clients: &Clients, stop: &AtomicBool) -> anyhow::Result<()> {
    let tx = &main.requests;
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
        return respond(
            &mut stream,
            403,
            "application/json",
            br#"{"ok":false,"error":"only apps on this computer may use the API"}"#,
        );
    }
    // Pictures are for everyone, like the views of plugins that show them.
    let picture = method == "GET" && path.starts_with("/api/icon/");
    let key = if picture { Some(String::new()) } else { let_in(&stream, main) };
    let Some(key) = key else {
        return respond(
            &mut stream,
            403,
            "application/json",
            br#"{"ok":false,"error":"not allowed: allow this app in Voicenux when it asks"}"#,
        );
    };
    if upgrade {
        return websocket(stream, tx, clients, stop, key);
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

/// Whether the app on the other end may use the API. Waits while Voicenux asks you. Gives the key
/// of the app when it may.
fn let_in(stream: &TcpStream, main: &Main) -> Option<String> {
    let peer = peer(stream).unwrap_or_else(|| Peer { key: String::new(), name: String::new() });
    let key = peer.key.clone();
    let (reply, answer) = mpsc::channel();
    main.access.send_blocking(Access { peer, reply }).ok()?;
    // Time to find the window and decide. A plugin that gives up sooner comes back anyway.
    answer.recv_timeout(Duration::from_secs(120)).unwrap_or(false).then_some(key)
}

/// Whether an app may use the API without asking: you allowed it, or asking is off.
fn welcome(st: &St, key: &str) -> bool {
    let c = st.config.borrow();
    !c.api_ask
        || ours(key)
        || (!key.is_empty() && c.api_apps.iter().any(|a| a.key == key))
        || ANSWERED.with_borrow(|a| a.contains(&(key.to_string(), true)))
}

/// Answers whether an app may use the API: at once when you said so before, or after asking you.
fn may(st: &St, peer: Peer, reply: mpsc::Sender<bool>) {
    if welcome(st, &peer.key) {
        let _ = reply.send(true);
        return;
    }
    if ANSWERED.with_borrow(|a| a.contains(&(peer.key.clone(), false))) {
        let _ = reply.send(false);
        return;
    }
    // One question per app, however often it tries.
    let first = ASKING.with_borrow_mut(|asking| match asking.iter_mut().find(|(p, _)| p.key == peer.key) {
        Some((_, waiting)) => {
            waiting.push(reply);
            false
        }
        None => {
            asking.push((peer.clone(), vec![reply]));
            true
        }
    });
    if first {
        st.notify(Change::ApiAsk(peer));
    }
}

/// Your answer about an app from `Change::ApiAsk`, None when the question went away unanswered:
/// then it comes again next time. Allowed apps are kept in the settings. An unknown app is
/// allowed or turned away until Voicenux quits, like any app you turn away.
pub fn answer(st: &St, peer: &Peer, allow: Option<bool>) {
    let waiting = ASKING.with_borrow_mut(|asking| {
        asking.iter().position(|(p, _)| p.key == peer.key).map(|i| asking.remove(i).1).unwrap_or_default()
    });
    match allow {
        Some(true) if !peer.key.is_empty() => {
            st.update(|c| c.api_apps.push(ApiApp { key: peer.key.clone(), name: peer.name.clone() }))
        }
        Some(ok) => ANSWERED.with_borrow_mut(|a| a.push((peer.key.clone(), ok))),
        None => {}
    }
    for w in waiting {
        let _ = w.send(allow == Some(true));
    }
}

/// The Voicenux plugin for OpenDeck and Stream Deck gets in without asking.
fn ours(key: &str) -> bool {
    key.contains("/de.felitendo.voicenux.sdPlugin/")
}

/// Finds the program on the other end of a connection from this computer: the socket in
/// /proc/net/tcp, then the process that holds it. Programs of other users, and all of them when
/// Voicenux runs in a sandbox, stay unknown.
fn peer(stream: &TcpStream) -> Option<Peer> {
    let (from, to) = (stream.peer_addr().ok()?.port(), stream.local_addr().ok()?.port());
    let inode = ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .find_map(|f| socket_inode(&std::fs::read_to_string(f).ok()?, from, to))?;
    let pid = owner(&format!("socket:[{inode}]"))?;
    describe(Path::new(&format!("/proc/{pid}")))
}

/// The inode of the socket from port `from` to port `to`, in a table like /proc/net/tcp.
fn socket_inode(table: &str, from: u16, to: u16) -> Option<u64> {
    let port = |address: &str| address.rsplit(':').next().and_then(|h| u16::from_str_radix(h, 16).ok());
    table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() > 9 && port(f[1]) == Some(from) && port(f[2]) == Some(to) { f[9].parse().ok() } else { None }
    })
}

/// The process of this user that holds `socket`, like "socket:[1234]".
fn owner(socket: &str) -> Option<u32> {
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|dir| {
        let pid: u32 = dir.file_name().to_str()?.parse().ok()?;
        if dir.metadata().ok()?.uid() != uid {
            return None;
        }
        let mut fds = std::fs::read_dir(dir.path().join("fd")).ok()?.flatten();
        fds.any(|fd| std::fs::read_link(fd.path()).is_ok_and(|l| l.as_os_str() == socket)).then_some(pid)
    })
}

/// Programs that run scripts: the script tells apart what they run.
const INTERPRETERS: &[&str] =
    &["python", "node", "bun", "deno", "perl", "ruby", "lua", "php", "java", "sh", "bash", "zsh", "fish", "dash"];

/// Names a process, from its folder in /proc.
fn describe(proc: &Path) -> Option<Peer> {
    let exe = std::fs::read_link(proc.join("exe")).ok()?;
    let exe = exe.to_string_lossy().trim_end_matches(" (deleted)").to_string();
    let program = Path::new(&exe).file_name()?.to_string_lossy().to_string();
    // A plugin of OpenDeck or a Stream Deck: the name from its manifest.
    if let Some(dir) = Path::new(&exe).ancestors().find(|d| d.extension().is_some_and(|e| e == "sdPlugin")) {
        let manifest: Value =
            std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str(&t).ok())?;
        let name = manifest["Name"].as_str().map_or(program.clone(), String::from);
        return Some(Peer { key: exe, name: fill(t("Plugin {}"), &[&name]) });
    }
    let bare = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if INTERPRETERS.contains(&bare) {
        let args = std::fs::read(proc.join("cmdline")).unwrap_or_default();
        let script =
            args.split(|b| *b == 0).skip(1).map(String::from_utf8_lossy).find(|a| !a.is_empty() && !a.starts_with('-'));
        if let Some(script) = script {
            let cwd = std::fs::read_link(proc.join("cwd")).unwrap_or_default();
            let path = cwd.join(script.as_ref());
            let file = path.file_name().map_or(script.to_string(), |f| f.to_string_lossy().to_string());
            return Some(Peer { key: format!("{exe} {}", path.display()), name: format!("{program} {file}") });
        }
    }
    Some(Peer { key: exe, name: program })
}

fn websocket(
    stream: TcpStream,
    tx: &async_channel::Sender<Request>,
    clients: &Clients,
    stop: &AtomicBool,
    key: String,
) -> anyhow::Result<()> {
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
    clients.lock().unwrap_or_else(|e| e.into_inner()).push((id, key, out));
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
            loop {
                match inbox.try_recv() {
                    Ok(event) => ws.send(Message::text(event))?,
                    Err(mpsc::TryRecvError::Empty) => break,
                    // You took the app back in Settings.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let _ = ws.close(None);
                        let _ = ws.flush();
                        return Ok(());
                    }
                }
            }
            let _ = ws.flush();
        }
    })();
    clients.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, _, _)| *i != id);
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

fn http(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    body: &[u8],
    tx: &async_channel::Sender<Request>,
) -> anyhow::Result<()> {
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
    tx.send_blocking(Request { action: action.to_string(), arg, reply })
        .map_err(|_| "Voicenux is closing".to_string())?;
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
        "random-voice" => {
            st.random_voice(false);
        }
        "random-favorite" => {
            if !st.random_voice(true) {
                return Err("no other favorite to pick, star two voices or more".into());
            }
        }
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
        // The same, but in this play mode, whatever the soundboard is set to.
        "sound-restart" | "sound-toggle" | "sound-overlap" => {
            let id = find_sound(st, need()?)?;
            let mode = PlayMode::from_id(action.trim_start_matches("sound-")).unwrap_or_default();
            st.play_sound_as(&id, mode);
        }
        "stop-sound" => {
            let id = find_sound(st, need()?)?;
            st.stop_sound(&id);
        }
        "stop-sounds" => st.stop_sounds(),
        "sound-volume" => {
            let v: f32 =
                need()?.trim_end_matches('%').trim().parse().map_err(|_| "sound-volume needs 0 to 100".to_string())?;
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
                "mode": st.sound_mode(),
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
        match (
            bytes[i],
            bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()),
        ) {
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
    fn finds_sockets() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:A3EF 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0 100 0 0 10 0
   1: 0100007F:C350 0100007F:A3EF 01 00000000:00000000 00:00000000 00000000  1000        0 222 1 0 20 4 30 10 -1
   2: 0100007F:A3EF 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 333 1 0 20 4 30 10 -1";
        // The client's end: from its port to ours, not our end the other way round.
        assert_eq!(socket_inode(table, 50000, 41967), Some(222));
        assert_eq!(socket_inode(table, 50001, 41967), None);
    }

    #[test]
    fn knows_who_connects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let me = std::env::current_exe().unwrap().to_string_lossy().to_string();
        assert_eq!(peer(&stream).map(|p| p.key), Some(me));
    }

    #[test]
    fn names_programs() {
        let dir = std::env::temp_dir().join(format!("voicenux-peer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proc = dir.join("proc");
        std::fs::create_dir_all(&proc).unwrap();
        let link = |target: &Path, name: &str| {
            let _ = std::fs::remove_file(proc.join(name));
            std::os::unix::fs::symlink(target, proc.join(name)).unwrap();
        };
        // A script: the interpreter and the script, made whole with the folder it runs in.
        link(Path::new("/usr/bin/python3.13"), "exe");
        link(Path::new("/home/me/bots"), "cwd");
        std::fs::write(proc.join("cmdline"), b"python3\0-u\0deck.py\0").unwrap();
        let p = describe(&proc).unwrap();
        assert_eq!(
            (p.key.as_str(), p.name.as_str()),
            ("/usr/bin/python3.13 /home/me/bots/deck.py", "python3.13 deck.py")
        );
        // Any other program by itself.
        link(Path::new("/usr/bin/curl"), "exe");
        assert_eq!(describe(&proc).unwrap().name, "curl");
        // Our plugin, by the name in its manifest. It may in without asking.
        let plugin = dir.join("plugins/de.felitendo.voicenux.sdPlugin");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("manifest.json"), r#"{"Name": "Voicenux"}"#).unwrap();
        link(&plugin.join("bin/voicenux-plugin"), "exe");
        let p = describe(&proc).unwrap();
        assert_eq!(p.name, fill(t("Plugin {}"), &["Voicenux"]));
        assert!(ours(&p.key));
        assert!(!ours("/usr/bin/curl"));
        std::fs::remove_dir_all(&dir).unwrap();
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
