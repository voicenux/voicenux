//! The audio engine: a PipeWire filter between the microphone, the virtual
//! "Voicenux Microphone" and the headphones.

pub mod easyeffects;
mod filter;
mod graph;
pub mod rt;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::dsp::chain::{BlockValues, Chain};
use crate::soundboard::SoundData;

pub use graph::{Device, Engine, Event};

/// Node name of the virtual microphone other apps record from.
pub const MIC_NODE: &str = "voicenux_mic";
pub const MIC_DESCRIPTION: &str = "Voicenux Microphone";
pub const ENGINE_NODE: &str = "voicenux_engine";

/// An f32 in an AtomicU32, for levels the UI reads.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    /// Keeps the highest value since the last `take`.
    pub fn raise(&self, v: f32) {
        // Bits of non-negative floats sort like the floats themselves.
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }

    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

/// What the realtime side tells everyone else.
#[derive(Default)]
pub struct Meters {
    pub rate: AtomicU32,
    pub input: AtomicF32,
    pub output: AtomicF32,
    pub sounds: AtomicF32,
    /// Playing sounds: id in the high 32 bits, progress in permille in the low bits.
    pub playing: [AtomicU64; 16],
}

impl Meters {
    /// Ids and progress (0..1) of the sounds that play right now.
    pub fn playing(&self) -> Vec<(u32, f32)> {
        self.playing
            .iter()
            .map(|m| m.load(Ordering::Relaxed))
            .filter(|v| *v != 0)
            .map(|v| ((v >> 32) as u32, (v & 0xffff_ffff) as f32 / 1000.0))
            .collect()
    }
}

/// Commands for the realtime side.
pub enum RtCmd {
    Chain(Box<Chain>),
    Param {
        block: u16,
        index: u16,
        value: f32,
    },
    VoiceOn(bool),
    HearMyself(bool),
    Mute(bool),
    HearSounds(bool),
    Background(bool),
    InputGain(f32),
    MicVolume(f32),
    MonitorVolume(f32),
    SoundVolume(f32),
    Denoise(bool),
    Gate(Option<f32>),
    /// A preview goes to the headphones only, never into the Voicenux Microphone.
    Play {
        id: u32,
        data: Arc<SoundData>,
        volume: f32,
        restart: bool,
        preview: bool,
    },
    Stop(u32),
    StopAll,
    SoundVolumeOf {
        id: u32,
        volume: f32,
    },
    /// Some: the microphone goes into this ring, and others no longer hear your voice. None: back
    /// to normal.
    Listen(Option<rtrb::Producer<f32>>),
}

/// Things the realtime side is done with. They are freed on another thread.
pub enum Trash {
    Chain(#[allow(dead_code)] Box<Chain>),
    Sound(#[allow(dead_code)] Arc<SoundData>),
    Listen(#[allow(dead_code)] rtrb::Producer<f32>),
}

/// Commands for the engine thread.
pub enum Ctl {
    Rt(RtCmd),
    Voice(Vec<BlockValues>),
    LowLatency(bool),
    Input(Option<String>),
    Output(Option<String>),
    EasyEffects(bool),
    DefaultMic { on: bool, previous: Option<String> },
    Quit,
}
