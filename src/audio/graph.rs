//! The engine thread: owns the PipeWire connection, creates the virtual microphone and the
//! filter, watches devices and keeps the links between them right.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::types::ObjectType;

use super::filter::Filter;
use super::rt::Processor;
use super::{Ctl, ENGINE_NODE, MIC_DESCRIPTION, MIC_NODE, Meters, RtCmd, Trash};
use crate::dsp::BuildCtx;
use crate::dsp::chain::{BlockValues, Chain};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub description: String,
}

#[derive(Debug)]
pub enum Event {
    Ready,
    Failed(String),
    Devices {
        inputs: Vec<Device>,
        outputs: Vec<Device>,
    },
    /// The devices that are linked right now.
    Active {
        input: Option<Device>,
        output: Option<Device>,
    },
    /// Apps that record from the Voicenux Microphone.
    Users(Vec<String>),
    /// Whether the Voicenux Microphone is the default, and what was the default before.
    DefaultMic {
        previous: Option<String>,
    },
}

pub struct Engine {
    tx: pw::channel::Sender<Ctl>,
    pub meters: Arc<Meters>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn start(events: async_channel::Sender<Event>, low_latency: bool) -> Self {
        let meters = Arc::new(Meters::default());
        let (tx, rx) = pw::channel::channel();
        let m = meters.clone();
        let thread = std::thread::Builder::new()
            .name("voicenux-engine".into())
            .spawn(move || {
                if let Err(e) = run(rx, events.clone(), m, low_latency) {
                    log::error!("audio engine stopped: {e:#}");
                    let _ = events.send_blocking(Event::Failed(format!("{e:#}")));
                }
            })
            .expect("could not start the audio thread");
        Self { tx, meters, thread: Some(thread) }
    }

    pub fn send(&self, ctl: Ctl) {
        let _ = self.tx.send(ctl);
    }

    pub fn rt(&self, cmd: RtCmd) {
        self.send(Ctl::Rt(cmd));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.send(Ctl::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn latency(low: bool) -> &'static str {
    if low { "256/48000" } else { "512/48000" }
}

#[derive(Clone, Debug)]
struct NodeInfo {
    name: String,
    description: String,
    class: String,
    app: String,
}

#[derive(Clone, Debug)]
struct PortInfo {
    node: u32,
    output: bool,
    name: String,
    channel: String,
    monitor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LinkInfo {
    out_node: u32,
    out_port: u32,
    in_node: u32,
    in_port: u32,
}

#[derive(Default)]
struct Graph {
    nodes: HashMap<u32, NodeInfo>,
    ports: HashMap<u32, PortInfo>,
    links: HashMap<u32, LinkInfo>,
    default_source: Option<String>,
    default_sink: Option<String>,
    configured_source: Option<String>,
    /// The microphone to use for "System default" while the default is our own mic.
    fallback_input: Option<String>,
    /// Default mic switch waiting for the metadata object, and the mic to go back to.
    want_default: Option<bool>,
    default_previous: Option<String>,
    want_input: Option<String>,
    want_output: Option<String>,
    dirty: bool,
    created: HashMap<(u32, u32), pw::link::Link>,
    metadata: Option<pw::metadata::Metadata>,
    metadata_listener: Option<pw::metadata::MetadataListener>,
    sent_devices: Option<(Vec<Device>, Vec<Device>)>,
    sent_active: Option<(Option<Device>, Option<Device>)>,
    sent_users: Option<Vec<String>>,
    sent_default: Option<bool>,
}

/// Apps that only watch levels (volume controls), not real users of the microphone.
const WATCHERS: &[&str] =
    &["pavucontrol", "PulseAudio Volume Control", "plasmashell", "Plasma PA", "pwvucontrol", "Volume Control"];

fn parse_default(value: Option<&str>) -> Option<String> {
    // Values look like {"name":"alsa_input.usb-..."}.
    let v = value?;
    let start = v.find("\"name\"")? + 6;
    let rest = &v[start..];
    let q1 = rest.find('"')? + 1;
    let q2 = rest[q1..].find('"')? + q1;
    Some(rest[q1..q2].to_string())
}

impl Graph {
    fn node_id(&self, name: &str) -> Option<u32> {
        self.nodes.iter().find(|(_, n)| n.name == name).map(|(id, _)| *id)
    }

    fn device(&self, id: u32) -> Option<Device> {
        self.nodes.get(&id).map(|n| Device { name: n.name.clone(), description: n.description.clone() })
    }

    fn sources(&self) -> Vec<(u32, &NodeInfo)> {
        let mut v: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.class.starts_with("Audio/Source") && n.name != MIC_NODE)
            .map(|(id, n)| (*id, n))
            .collect();
        v.sort_by(|a, b| a.1.description.cmp(&b.1.description));
        v
    }

    fn sinks(&self) -> Vec<(u32, &NodeInfo)> {
        let mut v: Vec<_> =
            self.nodes.iter().filter(|(_, n)| n.class.starts_with("Audio/Sink")).map(|(id, n)| (*id, n)).collect();
        v.sort_by(|a, b| a.1.description.cmp(&b.1.description));
        v
    }

    fn pick_input(&self) -> Option<u32> {
        let sources = self.sources();
        let by_name = |name: &Option<String>| {
            name.as_ref().and_then(|n| sources.iter().find(|(_, s)| &s.name == n).map(|(id, _)| *id))
        };
        by_name(&self.want_input)
            .or_else(|| by_name(&self.default_source))
            // The default is our own mic (or gone): use the one that was default before.
            .or_else(|| by_name(&self.fallback_input))
            .or_else(|| by_name(&self.configured_source))
            .or_else(|| sources.iter().find(|(_, s)| !s.class.contains("Virtual")).map(|(id, _)| *id))
            .or_else(|| sources.first().map(|(id, _)| *id))
    }

    fn pick_output(&self) -> Option<u32> {
        let sinks = self.sinks();
        let by_name = |name: &Option<String>| {
            name.as_ref().and_then(|n| sinks.iter().find(|(_, s)| &s.name == n).map(|(id, _)| *id))
        };
        by_name(&self.want_output).or_else(|| by_name(&self.default_sink)).or_else(|| sinks.first().map(|(id, _)| *id))
    }

    fn ports_of(&self, node: u32, output: bool) -> Vec<(u32, &PortInfo)> {
        let mut v: Vec<_> = self
            .ports
            .iter()
            .filter(|(_, p)| p.node == node && p.output == output && !p.monitor)
            .map(|(id, p)| (*id, p))
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v
    }

    fn port_named(&self, node: u32, output: bool, name: &str) -> Option<u32> {
        self.ports_of(node, output).into_iter().find(|(_, p)| p.name == name).map(|(id, _)| id)
    }

    fn port_channel(&self, node: u32, output: bool, channel: &str) -> Option<u32> {
        self.ports_of(node, output).into_iter().find(|(_, p)| p.channel == channel).map(|(id, _)| id)
    }

    /// The links the engine should have right now.
    fn wanted(&self) -> Vec<LinkInfo> {
        let mut want = Vec::new();
        let (Some(engine), Some(mic)) = (self.node_id(ENGINE_NODE), self.node_id(MIC_NODE)) else {
            return want;
        };
        let mut link = |out_node, out_port: Option<u32>, in_node, in_port: Option<u32>| {
            if let (Some(out_port), Some(in_port)) = (out_port, in_port) {
                want.push(LinkInfo { out_node, out_port, in_node, in_port });
            }
        };
        if let Some(src) = self.pick_input() {
            let ports = self.ports_of(src, true);
            // Mono mics have MONO, stereo ones FL and FR. Take one channel only.
            let port = ports
                .iter()
                .find(|(_, p)| p.channel == "MONO")
                .or_else(|| ports.iter().find(|(_, p)| p.channel == "FL"))
                .or(ports.first())
                .map(|(id, _)| *id);
            link(src, port, engine, self.port_named(engine, false, "input_MONO"));
        }
        for ch in ["FL", "FR"] {
            link(
                engine,
                self.port_named(engine, true, &format!("output_{ch}")),
                mic,
                self.port_channel(mic, false, ch),
            );
        }
        if let Some(sink) = self.pick_output() {
            for ch in ["FL", "FR"] {
                let target = self.port_channel(sink, false, ch).or_else(|| self.port_channel(sink, false, "MONO"));
                link(engine, self.port_named(engine, true, &format!("monitor_{ch}")), sink, target);
            }
        }
        want
    }

    fn relink(&mut self, core: &pw::core::Core, registry: &pw::registry::Registry) {
        self.dirty = false;
        let want = self.wanted();
        if want.is_empty() {
            return;
        }
        let wanted: HashSet<(u32, u32)> = want.iter().map(|l| (l.out_port, l.in_port)).collect();
        let existing: HashSet<(u32, u32)> = self.links.values().map(|l| (l.out_port, l.in_port)).collect();
        for l in &want {
            let key = (l.out_port, l.in_port);
            if existing.contains(&key) || self.created.contains_key(&key) {
                continue;
            }
            let props = properties! {
                "link.output.node" => l.out_node.to_string(),
                "link.output.port" => l.out_port.to_string(),
                "link.input.node" => l.in_node.to_string(),
                "link.input.port" => l.in_port.to_string(),
                "object.linger" => "false",
            };
            match core.create_object::<pw::link::Link>("link-factory", &props) {
                Ok(link) => {
                    self.created.insert(key, link);
                }
                Err(e) => log::warn!("could not link {key:?}: {e}"),
            }
        }
        let stale: Vec<(u32, u32)> = self.created.keys().filter(|k| !wanted.contains(k)).copied().collect();
        for key in stale {
            if let Some(link) = self.created.remove(&key) {
                let _ = core.destroy_object(link);
            }
        }
        // Links someone else made to the engine's ports (a patchbay, an old session).
        let engine = self.node_id(ENGINE_NODE);
        let foreign: Vec<u32> = self
            .links
            .iter()
            .filter(|(_, l)| Some(l.in_node) == engine || Some(l.out_node) == engine)
            .filter(|(_, l)| !wanted.contains(&(l.out_port, l.in_port)))
            .map(|(id, _)| *id)
            .collect();
        for id in foreign {
            registry.destroy_global(id);
        }
    }

    fn users(&self) -> Vec<String> {
        let Some(mic) = self.node_id(MIC_NODE) else { return Vec::new() };
        let mut users: Vec<String> = self
            .links
            .values()
            .filter(|l| l.out_node == mic)
            .filter_map(|l| self.nodes.get(&l.in_node))
            .filter(|n| n.name != ENGINE_NODE)
            .map(|n| if n.app.is_empty() { n.description.clone() } else { n.app.clone() })
            .filter(|a| !WATCHERS.iter().any(|w| a.eq_ignore_ascii_case(w)))
            .collect();
        users.sort();
        users.dedup();
        users
    }

    fn report(&mut self, events: &async_channel::Sender<Event>) {
        let inputs: Vec<Device> = self.sources().iter().map(|(id, _)| self.device(*id).unwrap()).collect();
        let outputs: Vec<Device> = self.sinks().iter().map(|(id, _)| self.device(*id).unwrap()).collect();
        if self.sent_devices.as_ref() != Some(&(inputs.clone(), outputs.clone())) {
            self.sent_devices = Some((inputs.clone(), outputs.clone()));
            let _ = events.try_send(Event::Devices { inputs, outputs });
        }
        let active = (self.pick_input().and_then(|i| self.device(i)), self.pick_output().and_then(|o| self.device(o)));
        if self.sent_active.as_ref() != Some(&active) {
            self.sent_active = Some(active.clone());
            let _ = events.try_send(Event::Active { input: active.0, output: active.1 });
        }
        let users = self.users();
        if self.sent_users.as_ref() != Some(&users) {
            self.sent_users = Some(users.clone());
            let _ = events.try_send(Event::Users(users));
        }
        let ours = self.configured_source.as_deref() == Some(MIC_NODE);
        if self.sent_default != Some(ours) {
            self.sent_default = Some(ours);
            let _ = events.try_send(Event::DefaultMic { previous: None });
        }
    }

    /// Makes our mic the default (or gives the default back) once the metadata is there.
    fn apply_default(&mut self, events: &async_channel::Sender<Event>) {
        let Some(md) = &self.metadata else { return };
        let Some(on) = self.want_default.take() else { return };
        let key = "default.configured.audio.source";
        if on {
            let before = self.configured_source.clone().filter(|s| s != MIC_NODE);
            if before.is_some() {
                self.fallback_input = before.clone();
            }
            md.set_property(0, key, Some("Spa:String:JSON"), Some(&format!("{{ \"name\": \"{MIC_NODE}\" }}")));
            let _ = events.try_send(Event::DefaultMic { previous: before });
        } else {
            match &self.default_previous {
                Some(p) => md.set_property(0, key, Some("Spa:String:JSON"), Some(&format!("{{ \"name\": \"{p}\" }}"))),
                None => md.set_property(0, key, None, None),
            }
        }
        self.dirty = true;
    }

    fn add(&mut self, obj: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>) {
        let Some(props) = obj.props else { return };
        let get = |k: &str| props.get(k).unwrap_or("").to_string();
        match obj.type_ {
            ObjectType::Node => {
                let name = get("node.name");
                let description =
                    props.get("node.description").or_else(|| props.get("node.nick")).unwrap_or(&name).to_string();
                self.nodes.insert(
                    obj.id,
                    NodeInfo { name, description, class: get("media.class"), app: get("application.name") },
                );
            }
            ObjectType::Port => {
                let Ok(node) = get("node.id").parse() else { return };
                self.ports.insert(
                    obj.id,
                    PortInfo {
                        node,
                        output: get("port.direction") == "out",
                        name: get("port.name"),
                        channel: get("audio.channel"),
                        monitor: get("port.monitor") == "true",
                    },
                );
            }
            ObjectType::Link => {
                let p = |k: &str| get(k).parse().unwrap_or(0);
                self.links.insert(
                    obj.id,
                    LinkInfo {
                        out_node: p("link.output.node"),
                        out_port: p("link.output.port"),
                        in_node: p("link.input.node"),
                        in_port: p("link.input.port"),
                    },
                );
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn remove(&mut self, id: u32) {
        if self.nodes.remove(&id).is_some() || self.ports.remove(&id).is_some() || self.links.remove(&id).is_some() {
            self.dirty = true;
        }
        // Forget link proxies whose ports are gone, so they get made again later.
        let ports = &self.ports;
        self.created.retain(|(o, i), _| ports.contains_key(o) && ports.contains_key(i));
    }
}

fn run(
    rx: pw::channel::Receiver<Ctl>,
    events: async_channel::Sender<Event>,
    meters: Arc<Meters>,
    low_latency: bool,
) -> anyhow::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<RtCmd>::new(512);
    let (trash_tx, trash_rx) = rtrb::RingBuffer::<Trash>::new(512);
    let cmd_tx = Rc::new(RefCell::new(cmd_tx));
    let trash_rx = Rc::new(RefCell::new(trash_rx));
    let filter = Rc::new(Filter::new(&core, Processor::new(meters.clone(), cmd_rx, trash_tx), latency(low_latency))?);

    let _mic: pw::node::Node = core.create_object(
        "adapter",
        &properties! {
            "factory.name" => "support.null-audio-sink",
            "node.name" => MIC_NODE,
            "node.description" => MIC_DESCRIPTION,
            "media.class" => "Audio/Source/Virtual",
            "audio.position" => "FL,FR",
            "monitor.channel-volumes" => "true",
            "device.icon-name" => "audio-input-microphone",
            "priority.session" => "1",
            "object.linger" => "false",
        },
    )?;

    let graph = Rc::new(RefCell::new(Graph::default()));

    let _core_listener = core
        .add_listener_local()
        .error(|id, _seq, res, msg| log::warn!("pipewire error on {id}: {res} {msg}"))
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let graph = graph.clone();
            let events = events.clone();
            let registry = registry.downgrade();
            move |obj| {
                if obj.type_ == ObjectType::Metadata
                    && obj.props.and_then(|p| p.get("metadata.name")) == Some("default")
                {
                    let Some(registry) = registry.upgrade() else { return };
                    let Ok(md) = registry.bind::<pw::metadata::Metadata, _>(obj) else { return };
                    let g = graph.clone();
                    let listener = md
                        .add_listener_local()
                        .property(move |subject, key, _type, value| {
                            if subject == 0 {
                                let mut g = g.borrow_mut();
                                match key {
                                    Some("default.audio.source") => g.default_source = parse_default(value),
                                    Some("default.audio.sink") => g.default_sink = parse_default(value),
                                    Some("default.configured.audio.source") => {
                                        g.configured_source = parse_default(value)
                                    }
                                    _ => return 0,
                                }
                                g.dirty = true;
                            }
                            0
                        })
                        .register();
                    let mut g = graph.borrow_mut();
                    g.metadata = Some(md);
                    g.metadata_listener = Some(listener);
                    g.apply_default(&events);
                    return;
                }
                graph.borrow_mut().add(obj);
            }
        })
        .global_remove({
            let graph = graph.clone();
            move |id| graph.borrow_mut().remove(id)
        })
        .register();

    // The voice as last sent, so it can be built again when the sample rate changes.
    let voice: Rc<RefCell<(Vec<BlockValues>, bool, u32)>> = Rc::new(RefCell::new((Vec::new(), low_latency, 0)));
    let push = {
        let cmd_tx = cmd_tx.clone();
        move |cmd: RtCmd| {
            if cmd_tx.borrow_mut().push(cmd).is_err() {
                log::warn!("audio command queue is full");
            }
        }
    };
    let build = {
        let voice = voice.clone();
        let meters = meters.clone();
        let push = push.clone();
        move || {
            let mut v = voice.borrow_mut();
            let rate = match meters.rate.load(std::sync::atomic::Ordering::Relaxed) {
                0 => 48000,
                r => r,
            };
            v.2 = rate;
            let chain = Chain::build_leveled(&v.0, &BuildCtx::new(rate as f32, v.1));
            push(RtCmd::Chain(Box::new(chain)));
        }
    };

    let timer = mainloop.loop_().add_timer({
        let graph = graph.clone();
        let core = core.clone();
        let registry = registry.clone();
        let trash_rx = trash_rx.clone();
        let events = events.clone();
        let voice = voice.clone();
        let meters = meters.clone();
        let build = build.clone();
        let ready = std::cell::Cell::new(false);
        move |_| {
            // Free what the realtime thread is done with.
            while trash_rx.borrow_mut().pop().is_ok() {}
            let rate = meters.rate.load(std::sync::atomic::Ordering::Relaxed);
            if rate != 0 && rate != voice.borrow().2 {
                build();
            }
            let mut g = graph.borrow_mut();
            if g.dirty {
                g.relink(&core, &registry);
                g.report(&events);
            }
            if !ready.get() && g.node_id(ENGINE_NODE).is_some() && g.node_id(MIC_NODE).is_some() {
                ready.set(true);
                let _ = events.try_send(Event::Ready);
            }
        }
    });
    timer.update_timer(Some(Duration::from_millis(40)), Some(Duration::from_millis(40))).into_result()?;

    let _rx = rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.downgrade();
        let graph = graph.clone();
        let filter = filter.clone();
        let events = events.clone();
        move |ctl| match ctl {
            Ctl::Rt(cmd) => push(cmd),
            Ctl::Voice(blocks) => {
                voice.borrow_mut().0 = blocks;
                build();
            }
            Ctl::LowLatency(on) => {
                voice.borrow_mut().1 = on;
                filter.set_latency(latency(on));
                build();
            }
            Ctl::Input(name) => {
                let mut g = graph.borrow_mut();
                g.want_input = name;
                g.dirty = true;
            }
            Ctl::Output(name) => {
                let mut g = graph.borrow_mut();
                g.want_output = name;
                g.dirty = true;
            }
            Ctl::DefaultMic { on, previous } => {
                let mut g = graph.borrow_mut();
                g.want_default = Some(on);
                if on && previous.is_some() && g.fallback_input.is_none() {
                    g.fallback_input = previous.clone();
                }
                g.default_previous = previous;
                g.apply_default(&events);
            }
            Ctl::Quit => {
                if let Some(m) = mainloop.upgrade() {
                    m.quit();
                }
            }
        }
    });

    mainloop.run();
    // Links and the virtual mic go away with the connection (object.linger = false).
    drop(filter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_metadata() {
        assert_eq!(parse_default(Some("{\"name\":\"alsa_input.pci\"}")), Some("alsa_input.pci".into()));
        assert_eq!(parse_default(Some("{ \"name\": \"x y\" }")), Some("x y".into()));
        assert_eq!(parse_default(None), None);
    }
}
