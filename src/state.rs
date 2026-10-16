//! What the app knows and does, apart from how it looks. Widgets call into this and
//! subscribe to changes.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;

use crate::audio::easyeffects::Use as EasyEffects;
use crate::audio::{Ctl, Device, Engine, Event, RtCmd};
use crate::config::Config;
use crate::soundboard::{self, Library, PlayMode, SoundData};
use crate::voice::{self, BlockDef, Voice};

/// The realtime id of a preview. Pads count up from 1, so they never get it.
const PREVIEW: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Another voice was picked.
    Voice,
    /// Settings of a voice changed (tweaks, Voicelab edits).
    VoiceEdited(String),
    /// The list of voices changed (custom voice added or removed).
    Voices,
    Favorites,
    /// Voice changer, hear myself, background or mute.
    Toggles,
    Devices,
    Users,
    Sounds,
    /// A sound was removed. It can come back until its note is gone.
    SoundRemoved { id: String, name: String },
    Settings,
    /// The engine is up, or failed with a message.
    Engine(Option<String>),
    /// Something changed without a click, worth a short note (shortcut, tray, command line).
    Notice(String),
    /// The API for other apps started, stopped, failed or got a client.
    Api,
    /// An AI voice was picked that is not downloaded yet: the window offers the download.
    AiMissing(String),
}

/// How the API for other apps is doing, for Settings.
#[derive(Clone, Debug, Default)]
pub struct ApiStatus {
    pub running: bool,
    /// Why it could not start.
    pub error: Option<String>,
    /// Apps connected over WebSocket.
    pub clients: usize,
}

/// Smoothed meter positions (0..1) for the widgets, updated once per frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Levels {
    pub input: f32,
    pub output: f32,
    pub sounds: f32,
    /// Seconds, from the frame clock.
    pub time: f64,
}

pub struct State {
    pub keys: crate::shortcuts::Handle,
    pub levels: Cell<Levels>,
    pub config: RefCell<Config>,
    pub voices: RefCell<Vec<Voice>>,
    pub library: RefCell<Library>,
    pub engine: Engine,
    pub inputs: RefCell<Vec<Device>>,
    pub outputs: RefCell<Vec<Device>>,
    pub active_input: RefCell<Option<Device>>,
    pub active_output: RefCell<Option<Device>>,
    pub users: RefCell<Vec<String>>,
    pub easyeffects: Cell<EasyEffects>,
    pub muted: Cell<bool>,
    pub ready: Cell<bool>,
    /// A tray icon is there to bring the window back.
    pub tray: Cell<bool>,
    /// Starter sounds are downloading.
    fetching: Cell<bool>,
    /// The last try to download them left some out, likely for being offline.
    starters_failed: Cell<bool>,
    pub api: RefCell<ApiStatus>,
    pub error: RefCell<Option<String>>,
    listeners: RefCell<Vec<Listener>>,
    sounds: RefCell<HashMap<String, Arc<SoundData>>>,
    /// Sounds that decode on a thread of their own, to play when they are done.
    loading: RefCell<HashSet<String>>,
    sound_ids: RefCell<HashMap<String, u32>>,
    /// Removed sounds and where they were, kept for Undo. Their files go when the note goes.
    trash: RefCell<Vec<(usize, soundboard::Sound)>>,
    save_source: RefCell<Option<glib::SourceId>>,
}

pub type St = Rc<State>;
type Listener = Rc<dyn Fn(&Change)>;

impl State {
    pub fn new() -> St {
        let mut config = Config::load();
        crate::ai::set_cpu(config.ai_cpu);
        let (tx, rx) = async_channel::unbounded();
        let engine = Engine::start(tx, config.low_latency);
        let mut voices = voice::builtin();
        voices.extend(voice::load_custom(&voice::custom_dir()));
        // A voice can go away in an update: then start with the first one.
        if !voices.iter().any(|v| v.id == config.voice) {
            config.voice = voices[0].id.clone();
        }
        let st = Rc::new(State {
            keys: Rc::default(),
            levels: Cell::default(),
            config: RefCell::new(config),
            voices: RefCell::new(voices),
            library: RefCell::new(Library::load()),
            engine,
            inputs: RefCell::default(),
            outputs: RefCell::default(),
            active_input: RefCell::default(),
            active_output: RefCell::default(),
            users: RefCell::default(),
            easyeffects: Cell::default(),
            muted: Cell::new(false),
            ready: Cell::new(false),
            tray: Cell::new(false),
            fetching: Cell::new(false),
            starters_failed: Cell::new(false),
            api: RefCell::default(),
            error: RefCell::default(),
            listeners: RefCell::default(),
            sounds: RefCell::default(),
            loading: RefCell::default(),
            sound_ids: RefCell::default(),
            trash: RefCell::default(),
            save_source: RefCell::default(),
        });
        st.push_all();
        let weak = Rc::downgrade(&st);
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(st) = weak.upgrade() else { break };
                st.on_event(ev);
            }
        });
        st
    }

    /// Sends every setting to the engine, at start.
    fn push_all(&self) {
        let c = self.config.borrow().clone();
        if !self.voices.borrow().iter().any(|v| v.id == c.voice) {
            self.config.borrow_mut().voice = "baby".into();
        }
        self.send_voice();
        let e = &self.engine;
        e.send(Ctl::Input(c.input.clone()));
        e.send(Ctl::Output(c.output.clone()));
        e.send(Ctl::EasyEffects(c.easyeffects));
        e.rt(RtCmd::VoiceOn(c.voice_on));
        e.rt(RtCmd::HearMyself(c.hear_myself));
        e.rt(RtCmd::Background(c.background));
        e.rt(RtCmd::HearSounds(c.hear_sounds));
        e.rt(RtCmd::InputGain(c.input_gain_db));
        e.rt(RtCmd::MicVolume(c.mic_volume_db));
        e.rt(RtCmd::MonitorVolume(c.monitor_volume));
        e.rt(RtCmd::SoundVolume(c.sound_volume));
        e.rt(RtCmd::Denoise(c.noise_suppression));
        e.rt(RtCmd::Gate(c.gate.then_some(c.gate_db)));
    }

    fn on_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::Ready => {
                self.ready.set(true);
                *self.error.borrow_mut() = None;
                if self.config.borrow().default_mic {
                    let prev = self.config.borrow().previous_default.clone();
                    self.engine.send(Ctl::DefaultMic { on: true, previous: prev });
                }
                self.notify(Change::Engine(None));
            }
            Event::Failed(msg) => {
                *self.error.borrow_mut() = Some(msg.clone());
                self.notify(Change::Engine(Some(msg)));
            }
            Event::Devices { inputs, outputs } => {
                *self.inputs.borrow_mut() = inputs;
                *self.outputs.borrow_mut() = outputs;
                self.notify(Change::Devices);
            }
            Event::Active { input, output } => {
                *self.active_input.borrow_mut() = input;
                *self.active_output.borrow_mut() = output;
                self.notify(Change::Devices);
            }
            Event::Users(users) => {
                *self.users.borrow_mut() = users;
                self.notify(Change::Users);
            }
            Event::EasyEffects(u) => {
                self.easyeffects.set(u);
                self.notify(Change::Devices);
            }
            Event::DefaultMic { previous, .. } => {
                if previous.is_some() {
                    self.config.borrow_mut().previous_default = previous;
                    self.save_later();
                }
            }
        }
    }

    /// Reads the meters of the engine. The window calls this once per frame.
    pub fn poll_levels(&self, now_us: i64) {
        let m = &self.engine.meters;
        let mut l = self.levels.get();
        let now = now_us as f64 / 1e6;
        let dt = (now - l.time).clamp(0.0, 0.1) as f32;
        l.time = now;
        let fall = (-dt / 0.22).exp();
        for (v, peak) in
            [(&mut l.input, m.input.take()), (&mut l.output, m.output.take()), (&mut l.sounds, m.sounds.take())]
        {
            let target = crate::ui::meter(peak);
            *v = if target > *v { target } else { target + (*v - target) * fall };
        }
        self.levels.set(l);
    }

    /// Closing the window keeps Voicenux running only when the tray can bring it back.
    pub fn runs_in_tray(&self) -> bool {
        self.config.borrow().close_to_tray && self.tray.get()
    }

    pub fn subscribe(&self, f: impl Fn(&Change) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn notify(&self, change: Change) {
        // Copy the list, a listener may subscribe while we go through it.
        let list: Vec<_> = self.listeners.borrow().clone();
        for l in list {
            l(&change);
        }
    }

    /// Saves the settings a moment later, so dragging a slider does not write on every step.
    pub fn save_later(self: &Rc<Self>) {
        if self.save_source.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(st) = weak.upgrade() {
                st.save_source.borrow_mut().take();
                st.config.borrow().save();
            }
        });
        *self.save_source.borrow_mut() = Some(id);
    }

    pub fn save_now(&self) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        self.config.borrow().save();
    }

    // Voices

    /// A voice with the user's changes applied.
    pub fn voice(&self, id: &str) -> Option<Voice> {
        let mut v = self.voices.borrow().iter().find(|v| v.id == id)?.clone();
        if let Some(blocks) = self.config.borrow().tweaks.get(id)
            && blocks.len() == v.blocks.len()
            && blocks.iter().zip(&v.blocks).all(|(a, b)| a.kind == b.kind)
        {
            v.blocks = blocks.clone();
        }
        Some(v)
    }

    pub fn current(&self) -> Voice {
        let id = self.config.borrow().voice.clone();
        self.voice(&id).or_else(|| self.voices.borrow().first().cloned()).expect("there are always voices")
    }

    pub fn is_tweaked(&self, id: &str) -> bool {
        self.config.borrow().tweaks.contains_key(id)
    }

    fn send_voice(&self) {
        let v = self.current();
        self.engine.send(Ctl::Voice(v.block_values()));
    }

    pub fn set_voice(self: &Rc<Self>, id: &str) {
        let Some(v) = self.voice(id) else { return };
        // Also for the voice in use: an AI voice can be missing since the last start.
        if let Some(model) = crate::ai::model_of(&v.block_values()) {
            if !crate::ai::usable() {
                let text = crate::i18n::t("AI voices need an NVIDIA graphics card, or Run on the processor in Settings.");
                self.notify(Change::Notice(text.into()));
                return;
            }
            if !crate::ai::ready(model) {
                self.notify(Change::AiMissing(id.to_string()));
                return;
            }
        }
        if self.config.borrow().voice == id && self.ready.get() {
            if !self.config.borrow().voice_on {
                self.set_voice_on(true);
            }
            return;
        }
        self.config.borrow_mut().voice = id.to_string();
        self.send_voice();
        if !self.config.borrow().voice_on {
            self.set_voice_on(true);
        }
        self.save_later();
        self.notify(Change::Voice);
    }

    /// Whether a voice can play right away: AI voices need their download. Random and next voice
    /// skip the others, so they never open the download dialog.
    fn playable(&self, id: &str) -> bool {
        self.voice(id).is_some_and(|v| {
            crate::ai::model_of(&v.block_values()).is_none_or(|m| crate::ai::usable() && crate::ai::ready(m))
        })
    }

    /// Picks a voice from favorites (or all voices if there are none), not the current one.
    pub fn random_voice(self: &Rc<Self>) {
        let current = self.config.borrow().voice.clone();
        let favs = self.config.borrow().favorites.clone();
        let pool: Vec<String> =
            if favs.len() >= 2 { favs } else { self.voices.borrow().iter().map(|v| v.id.clone()).collect() };
        let pool: Vec<&String> = pool.iter().filter(|id| **id != current && self.playable(id)).collect();
        if pool.is_empty() {
            return;
        }
        let pick = glib::random_int_range(0, pool.len() as i32) as usize;
        let id = pool[pick].clone();
        self.set_voice(&id);
    }

    /// Next or previous voice in the list (favorites first if there are any).
    pub fn step_voice(self: &Rc<Self>, forward: bool) {
        let favs = self.config.borrow().favorites.clone();
        let list: Vec<String> =
            if favs.is_empty() { self.voices.borrow().iter().map(|v| v.id.clone()).collect() } else { favs };
        let current = self.config.borrow().voice.clone();
        let pos = list.iter().position(|id| *id == current).unwrap_or(0) as i64;
        let len = list.len() as i64;
        let step = if forward { 1 } else { -1 };
        let next = (1..=len).map(|i| list[(pos + i * step).rem_euclid(len) as usize].clone()).find(|id| self.playable(id));
        if let Some(id) = next {
            self.set_voice(&id);
        }
    }

    /// Changes one setting of a voice. Built-in voices remember it as a tweak.
    pub fn set_param(self: &Rc<Self>, id: &str, block: usize, key: &str, value: f32) {
        let Some(mut v) = self.voice(id) else { return };
        let Some(b) = v.blocks.get_mut(block) else { return };
        b.set(key, value);
        let spec = crate::dsp::blocks::find(&b.kind);
        let index = spec.and_then(|s| s.param_index(key));
        self.store_blocks(&v);
        if id == self.config.borrow().voice
            && let Some(index) = index
        {
            let value = spec.unwrap().params[index].clamp(value);
            self.engine.rt(RtCmd::Param { block: block as u16, index: index as u16, value });
        }
        self.notify(Change::VoiceEdited(id.to_string()));
    }

    fn store_blocks(self: &Rc<Self>, v: &Voice) {
        let builtin = self.voices.borrow().iter().find(|x| x.id == v.id).map(|x| x.builtin).unwrap_or(false);
        if builtin {
            let original = self.voices.borrow().iter().find(|x| x.id == v.id).unwrap().blocks.clone();
            let mut c = self.config.borrow_mut();
            if original == v.blocks {
                c.tweaks.remove(&v.id);
            } else {
                c.tweaks.insert(v.id.clone(), v.blocks.clone());
            }
            drop(c);
            self.save_later();
        } else {
            if let Some(x) = self.voices.borrow_mut().iter_mut().find(|x| x.id == v.id) {
                x.blocks = v.blocks.clone();
            }
            if let Err(e) = voice::save_custom(&voice::custom_dir(), v) {
                log::error!("could not save voice {}: {e}", v.id);
            }
        }
    }

    pub fn reset_voice(self: &Rc<Self>, id: &str) {
        self.config.borrow_mut().tweaks.remove(id);
        self.save_later();
        if id == self.config.borrow().voice {
            self.send_voice();
        }
        self.notify(Change::VoiceEdited(id.to_string()));
    }

    /// Saves a voice from Voicelab: new or changed blocks, name, icon.
    pub fn save_custom(self: &Rc<Self>, v: Voice) {
        if let Err(e) = voice::save_custom(&voice::custom_dir(), &v) {
            log::error!("could not save voice {}: {e}", v.id);
        }
        let mut list = self.voices.borrow_mut();
        let added = match list.iter_mut().find(|x| x.id == v.id) {
            Some(x) => {
                *x = v.clone();
                false
            }
            None => {
                list.push(v.clone());
                true
            }
        };
        drop(list);
        if v.id == self.config.borrow().voice {
            self.send_voice();
        }
        self.notify(if added { Change::Voices } else { Change::VoiceEdited(v.id.clone()) });
    }

    pub fn delete_custom(self: &Rc<Self>, id: &str) {
        voice::delete_custom(&voice::custom_dir(), id);
        self.voices.borrow_mut().retain(|v| v.id != id);
        self.config.borrow_mut().favorites.retain(|f| f != id);
        if self.config.borrow().voice == id {
            self.config.borrow_mut().voice = "baby".into();
            self.send_voice();
            self.notify(Change::Voice);
        }
        self.save_later();
        self.notify(Change::Voices);
    }

    /// A copy of any voice as a new custom voice.
    pub fn duplicate(self: &Rc<Self>, id: &str) -> Option<Voice> {
        let src = self.voice(id)?;
        let taken: Vec<String> = self.voices.borrow().iter().map(|v| v.id.clone()).collect();
        let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
        let name = if src.builtin {
            crate::i18n::fill(crate::i18n::t("My {}"), &[&crate::i18n::t_dyn(&src.name)])
        } else {
            crate::i18n::fill(crate::i18n::t("{} copy"), &[&src.name])
        };
        let v = Voice {
            id: voice::new_id(&name, &taken),
            name,
            icon: src.icon.clone(),
            category: "custom".into(),
            description: String::new(),
            blocks: src.blocks.clone(),
            builtin: false,
        };
        self.save_custom(v.clone());
        Some(v)
    }

    pub fn new_custom(self: &Rc<Self>) -> Voice {
        let taken: Vec<String> = self.voices.borrow().iter().map(|v| v.id.clone()).collect();
        let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
        let name = crate::i18n::t("New voice").to_string();
        let v = Voice {
            id: voice::new_id(&name, &taken),
            name,
            icon: "sparkles".into(),
            category: "custom".into(),
            description: String::new(),
            blocks: vec![BlockDef::new("pitch")],
            builtin: false,
        };
        self.save_custom(v.clone());
        v
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.config.borrow().favorites.iter().any(|f| f == id)
    }

    pub fn toggle_favorite(self: &Rc<Self>, id: &str) {
        {
            let mut c = self.config.borrow_mut();
            if let Some(pos) = c.favorites.iter().position(|f| f == id) {
                c.favorites.remove(pos);
            } else {
                c.favorites.push(id.to_string());
            }
        }
        self.save_later();
        self.notify(Change::Favorites);
    }

    // Toggles

    pub fn set_voice_on(self: &Rc<Self>, on: bool) {
        self.config.borrow_mut().voice_on = on;
        self.engine.rt(RtCmd::VoiceOn(on));
        self.save_later();
        self.notify(Change::Toggles);
    }

    pub fn set_hear_myself(self: &Rc<Self>, on: bool) {
        self.config.borrow_mut().hear_myself = on;
        self.engine.rt(RtCmd::HearMyself(on));
        self.save_later();
        self.notify(Change::Toggles);
    }

    pub fn set_background(self: &Rc<Self>, on: bool) {
        self.config.borrow_mut().background = on;
        self.engine.rt(RtCmd::Background(on));
        self.save_later();
        self.notify(Change::Toggles);
    }

    pub fn set_muted(&self, on: bool) {
        self.muted.set(on);
        self.engine.rt(RtCmd::Mute(on));
        self.notify(Change::Toggles);
    }

    // Settings

    pub fn update(self: &Rc<Self>, f: impl FnOnce(&mut Config)) {
        let before = self.config.borrow().clone();
        f(&mut self.config.borrow_mut());
        let c = self.config.borrow().clone();
        let e = &self.engine;
        if c.input != before.input {
            e.send(Ctl::Input(c.input.clone()));
        }
        if c.output != before.output {
            e.send(Ctl::Output(c.output.clone()));
        }
        if c.easyeffects != before.easyeffects {
            e.send(Ctl::EasyEffects(c.easyeffects));
        }
        if c.input_gain_db != before.input_gain_db {
            e.rt(RtCmd::InputGain(c.input_gain_db));
        }
        if c.mic_volume_db != before.mic_volume_db {
            e.rt(RtCmd::MicVolume(c.mic_volume_db));
        }
        if c.monitor_volume != before.monitor_volume {
            e.rt(RtCmd::MonitorVolume(c.monitor_volume));
        }
        if c.sound_volume != before.sound_volume {
            e.rt(RtCmd::SoundVolume(c.sound_volume));
        }
        if c.hear_sounds != before.hear_sounds {
            e.rt(RtCmd::HearSounds(c.hear_sounds));
        }
        if c.noise_suppression != before.noise_suppression {
            e.rt(RtCmd::Denoise(c.noise_suppression));
        }
        if c.gate != before.gate || c.gate_db != before.gate_db {
            e.rt(RtCmd::Gate(c.gate.then_some(c.gate_db)));
        }
        if c.low_latency != before.low_latency {
            e.send(Ctl::LowLatency(c.low_latency));
        }
        if c.default_mic != before.default_mic {
            e.send(Ctl::DefaultMic { on: c.default_mic, previous: c.previous_default.clone() });
        }
        self.save_later();
        self.notify(Change::Settings);
    }

    // Soundboard

    fn sound_rt_id(&self, id: &str) -> u32 {
        let mut ids = self.sound_ids.borrow_mut();
        let next = ids.len() as u32 + 1;
        *ids.entry(id.to_string()).or_insert(next)
    }

    pub fn sound_id_of(&self, rt: u32) -> Option<String> {
        self.sound_ids.borrow().iter().find(|(_, v)| **v == rt).map(|(k, _)| k.clone())
    }

    /// Decodes a sound on a thread of its own and plays it when it is done. Decoding takes a
    /// moment, and on the main thread the window would stutter.
    fn load_and_play(self: &Rc<Self>, sound: soundboard::Sound) {
        if !self.loading.borrow_mut().insert(sound.id.clone()) {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn({
            let sound = sound.clone();
            move || {
                let data = soundboard::load(&sound).map(|d| soundboard::playable(&sound, d));
                let _ = tx.send_blocking(data);
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(data) = rx.recv().await else { return };
            let Some(st) = weak.upgrade() else { return };
            st.loading.borrow_mut().remove(&sound.id);
            let data = match data {
                Ok(d) => d,
                Err(e) => return st.sound_failed(&sound, e),
            };
            // Removed or cut again while it decoded: this audio is not the sound any more.
            let now = st.library.borrow().find(&sound.id).cloned();
            let Some(now) = now.filter(|n| (n.start, n.end) == (sound.start, sound.end)) else { return };
            st.sounds.borrow_mut().insert(now.id.clone(), data.clone());
            st.start_sound(&now, data);
        });
    }

    fn sound_failed(self: &Rc<Self>, sound: &soundboard::Sound, e: anyhow::Error) {
        log::warn!("could not load sound {}: {e}", sound.id);
        let name = crate::i18n::t_dyn(&sound.name);
        let waits = sound.builtin.as_deref().and_then(soundboard::starter::find).is_some_and(|b| !b.downloaded());
        let text = if waits {
            let failed = self.starters_failed.get();
            self.fetch_starters();
            if failed {
                crate::i18n::t("{} is not downloaded yet. Are you online?")
            } else {
                crate::i18n::t("{} is still downloading. Try again in a moment.")
            }
        } else {
            crate::i18n::t("Could not play {}")
        };
        self.notify(Change::Notice(crate::i18n::fill(text, &[&name])));
    }

    fn start_sound(&self, sound: &soundboard::Sound, data: Arc<SoundData>) {
        let mode = self.sound_mode(sound);
        self.engine.rt(RtCmd::Play {
            id: self.sound_rt_id(&sound.id),
            data,
            volume: sound.volume,
            looped: mode == PlayMode::Loop,
            restart: mode != PlayMode::Overlap,
            preview: false,
        });
    }

    /// How a sound plays: its own mode, or the one of the soundboard.
    pub fn sound_mode(&self, sound: &soundboard::Sound) -> PlayMode {
        sound.mode_on(self.config.borrow().sound_mode)
    }

    /// What a click does for every sound without a mode of its own.
    pub fn set_sound_mode(self: &Rc<Self>, mode: PlayMode) {
        self.update(|c| c.sound_mode = mode);
        self.notify(Change::Sounds);
    }

    pub fn is_playing(&self, id: &str) -> bool {
        let rt = self.sound_ids.borrow().get(id).copied();
        rt.is_some_and(|rt| self.engine.meters.playing().iter().any(|(p, _)| *p == rt))
    }

    pub fn play_sound(self: &Rc<Self>, id: &str) {
        let Some(sound) = self.library.borrow().find(id).cloned() else { return };
        let rt = self.sound_rt_id(id);
        let playing = self.is_playing(id);
        if playing && matches!(self.sound_mode(&sound), PlayMode::Toggle | PlayMode::Loop) {
            self.engine.rt(RtCmd::Stop(rt));
            return;
        }
        let cached = self.sounds.borrow().get(id).cloned();
        match cached {
            Some(data) => self.start_sound(&sound, data),
            None => self.load_and_play(sound),
        }
    }

    pub fn stop_sounds(&self) {
        self.engine.rt(RtCmd::StopAll);
    }

    pub fn stop_sound(&self, id: &str) {
        let rt = self.sound_rt_id(id);
        self.engine.rt(RtCmd::Stop(rt));
    }

    /// The sounds that play right now.
    pub fn playing_sounds(&self) -> Vec<String> {
        let mut ids: Vec<String> =
            self.engine.meters.playing().into_iter().filter_map(|(rt, _)| self.sound_id_of(rt)).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Plays a sound found online in the headphones only, to listen before adding it.
    pub fn preview(&self, data: Arc<SoundData>) {
        self.engine.rt(RtCmd::Play { id: PREVIEW, data, volume: 1.0, looped: false, restart: true, preview: true });
    }

    pub fn stop_preview(&self) {
        self.engine.rt(RtCmd::Stop(PREVIEW));
    }

    /// How far the preview has played, from 0 to 1. None when it does not play.
    pub fn preview_progress(&self) -> Option<f32> {
        // Right after a restart, the old one still fades out: the new one is the one further back.
        self.engine.meters.playing().into_iter().filter(|(id, _)| *id == PREVIEW).map(|(_, p)| p).reduce(f32::min)
    }

    /// All of a sound, without its cut, for the trim editor.
    pub fn sound_uncut(&self, id: &str) -> Option<Arc<SoundData>> {
        let sound = self.library.borrow().find(id)?.clone();
        soundboard::load(&sound).map_err(|e| log::warn!("could not load sound {id}: {e}")).ok()
    }

    /// Adds a sound found online.
    pub fn add_found(&self, name: &str, ext: &str, data: &[u8], icon: &str) -> anyhow::Result<()> {
        {
            let mut lib = self.library.borrow_mut();
            lib.add_bytes(name, ext, data, icon)?;
            lib.save();
        }
        self.notify(Change::Sounds);
        Ok(())
    }

    pub fn update_sound(&self, sound: soundboard::Sound) {
        let rt = self.sound_rt_id(&sound.id);
        self.engine.rt(RtCmd::SoundVolumeOf { id: rt, volume: sound.volume });
        {
            let mut lib = self.library.borrow_mut();
            if let Some(s) = lib.sound.iter_mut().find(|s| s.id == sound.id) {
                // A new cut needs the audio cut again.
                if (s.start, s.end) != (sound.start, sound.end) {
                    self.sounds.borrow_mut().remove(&sound.id);
                }
                *s = sound;
            }
            lib.save();
        }
        self.notify(Change::Sounds);
    }

    /// Volume of one sound, live and without redrawing the soundboard.
    pub fn set_sound_volume(&self, id: &str, volume: f32) {
        let rt = self.sound_rt_id(id);
        self.engine.rt(RtCmd::SoundVolumeOf { id: rt, volume });
        let mut lib = self.library.borrow_mut();
        if let Some(s) = lib.sound.iter_mut().find(|s| s.id == id) {
            s.volume = volume;
        }
        lib.save();
    }

    /// Adds audio files one after the other, on a thread of their own: checking and copying
    /// takes a moment each, and the window stays smooth. Each one shows up when it is in.
    pub fn import_sounds(self: &Rc<Self>, paths: Vec<std::path::PathBuf>) {
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            for p in paths {
                let r = soundboard::copy_in(&p);
                if tx.send_blocking((p, r)).is_err() {
                    break;
                }
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut failed = Vec::new();
            while let Ok((path, r)) = rx.recv().await {
                let Some(st) = weak.upgrade() else { return };
                match r {
                    Ok((file, name)) => {
                        let mut lib = st.library.borrow_mut();
                        lib.add_file(file, &name, "loudspeaker");
                        lib.save();
                        drop(lib);
                        st.notify(Change::Sounds);
                    }
                    Err(e) => {
                        log::warn!("could not import {}: {e}", path.display());
                        failed.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
                    }
                }
            }
            if let Some(st) = weak.upgrade()
                && !failed.is_empty()
            {
                let text = crate::i18n::fill(crate::i18n::t("Could not add {}"), &[&failed.join(", ")]);
                st.notify(Change::Notice(text));
            }
        });
    }

    /// Removes a sound. [`State::undo_remove`] brings it back, until [`State::forget_removed`].
    pub fn remove_sound(&self, id: &str) {
        let rt = self.sound_rt_id(id);
        self.engine.rt(RtCmd::Stop(rt));
        self.sounds.borrow_mut().remove(id);
        let taken = {
            let mut lib = self.library.borrow_mut();
            let taken = lib.take(id);
            lib.save();
            taken
        };
        let Some((pos, sound)) = taken else { return };
        let name = crate::i18n::t_dyn(&sound.name);
        self.trash.borrow_mut().push((pos, sound));
        self.notify(Change::Sounds);
        self.notify(Change::SoundRemoved { id: id.to_string(), name });
    }

    pub fn undo_remove(&self, id: &str) {
        let Some(i) = self.trash.borrow().iter().position(|(_, s)| s.id == id) else { return };
        let (pos, sound) = self.trash.borrow_mut().remove(i);
        {
            let mut lib = self.library.borrow_mut();
            let pos = pos.min(lib.sound.len());
            lib.sound.insert(pos, sound);
            lib.save();
        }
        self.notify(Change::Sounds);
    }

    /// Brings back the sound removed last. False when there is none.
    pub fn undo_last_remove(&self) -> bool {
        let last = self.trash.borrow().last().map(|(_, s)| s.id.clone());
        match last {
            Some(id) => {
                self.undo_remove(&id);
                true
            }
            None => false,
        }
    }

    /// The Undo for a removed sound is gone: so is its file.
    pub fn forget_removed(&self, id: &str) {
        let gone: Vec<_> = {
            let mut trash = self.trash.borrow_mut();
            let (gone, keep) = trash.drain(..).partition(|(_, s)| s.id == id);
            *trash = keep;
            gone
        };
        for (_, s) in gone {
            soundboard::delete_file(&s);
        }
    }

    /// Deletes the files of all removed sounds, when the app quits.
    pub fn empty_trash(&self) {
        for (_, s) in self.trash.borrow_mut().drain(..) {
            soundboard::delete_file(&s);
        }
    }

    /// Moves a sound before or after another one.
    pub fn move_sound(&self, id: &str, target: &str, after: bool) {
        {
            let mut lib = self.library.borrow_mut();
            lib.move_next_to(id, target, after);
            lib.save();
        }
        self.notify(Change::Sounds);
    }

    /// Moves a sound one place left or right.
    pub fn shift_sound(&self, id: &str, right: bool) {
        let target = {
            let lib = self.library.borrow();
            let Some(pos) = lib.sound.iter().position(|s| s.id == id) else { return };
            let to = if right { pos + 1 } else { pos.wrapping_sub(1) };
            match lib.sound.get(to) {
                Some(s) => s.id.clone(),
                None => return,
            }
        };
        self.move_sound(id, &target, right);
    }

    /// Sorts the soundboard by name, as the user reads them.
    /// Shows the soundboard by name, or in your own order again. Your order stays as it is.
    pub fn set_sounds_by_name(self: &Rc<Self>, on: bool) {
        self.update(|c| c.sounds_by_name = on);
        self.notify(Change::Sounds);
    }

    /// The sounds in the order the soundboard shows them.
    pub fn sounds_shown(&self) -> Vec<soundboard::Sound> {
        let mut sounds = self.library.borrow().sound.clone();
        if self.config.borrow().sounds_by_name {
            sounds.sort_by_cached_key(|s| crate::i18n::t_dyn(&s.name).to_lowercase());
        }
        sounds
    }

    pub fn restore_starter_sounds(self: &Rc<Self>) {
        {
            let mut lib = self.library.borrow_mut();
            for s in Library::starter().sound {
                if lib.find(&s.id).is_none() {
                    lib.sound.push(s);
                }
            }
            lib.save();
        }
        self.notify(Change::Sounds);
        self.fetch_starters();
    }

    /// Downloads the starter sounds on the soundboard that this computer does not have yet, a
    /// few at a time. Offline it tries again at the next start, or when one is played.
    pub fn fetch_starters(self: &Rc<Self>) {
        if self.fetching.get() {
            return;
        }
        let lib = self.library.borrow();
        let missing: Vec<&'static soundboard::starter::Builtin> = lib
            .sound
            .iter()
            .filter_map(|s| s.builtin.as_deref().and_then(soundboard::starter::find))
            .filter(|b| !b.downloaded())
            .collect();
        if missing.is_empty() {
            return;
        }
        self.fetching.set(true);
        let queue = Arc::new(std::sync::Mutex::new(missing));
        let (tx, rx) = async_channel::unbounded::<bool>();
        for _ in 0..4 {
            let (queue, tx) = (queue.clone(), tx.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(b) = queue.lock().ok().and_then(|mut q| q.pop()) else { break };
                    let _ = tx.send_blocking(b.fetch().is_ok());
                }
            });
        }
        drop(tx);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (mut got, mut failed) = (0, 0);
            while let Ok(ok) = rx.recv().await {
                if ok { got += 1 } else { failed += 1 }
            }
            let Some(st) = weak.upgrade() else { return };
            st.fetching.set(false);
            st.starters_failed.set(failed > 0);
            log::info!("downloaded {got} starter sounds, {failed} failed");
            if got > 0 {
                st.notify(Change::Sounds);
            }
        });
    }
}
