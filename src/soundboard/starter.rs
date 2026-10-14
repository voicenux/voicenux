//! The starter sounds: short CC0 recordings from Freesound, built into the app. Sources and
//! authors are in `data/sounds/LICENSE`.

use super::{SoundData, decode_bytes};

pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    data: &'static [u8],
}

macro_rules! sound {
    ($id:literal, $name:literal, $icon:literal) => {
        Builtin { id: $id, name: $name, icon: $icon, data: include_bytes!(concat!("../../data/sounds/", $id, ".ogg")) }
    };
}

pub static BUILTIN: &[Builtin] = &[
    sound!("airhorn", "Air Horn", "postal-horn"),
    sound!("sad-trombone", "Sad Trombone", "trumpet"),
    sound!("rimshot", "Rimshot", "joy"),
    sound!("drumroll", "Drum Roll", "drum"),
    sound!("applause", "Applause", "clapping"),
    sound!("tada", "Ta-da!", "party-popper"),
    sound!("correct", "Correct", "bell"),
    sound!("wrong", "Wrong", "cross-mark"),
    sound!("crickets", "Crickets", "cricket"),
    sound!("boing", "Boing", "basketball"),
    sound!("laser", "Laser", "high-voltage"),
    sound!("explosion", "Explosion", "bomb"),
    sound!("coin", "Coin", "coin"),
    sound!("whoosh", "Whoosh", "dashing-away"),
];

pub fn load(id: &str) -> Option<SoundData> {
    let b = BUILTIN.iter().find(|b| b.id == id)?;
    decode_bytes(b.data.to_vec(), "ogg").map_err(|e| log::warn!("starter sound {id}: {e}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_decodes() {
        for b in BUILTIN {
            let d = load(b.id).unwrap();
            assert!(d.seconds() > 0.2 && d.seconds() < 8.0, "{} is {} s", b.id, d.seconds());
            assert!(d.frames.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
            let peak = d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
            assert!(peak > 0.2 && peak < 1.0, "{} peak {peak}", b.id);
        }
    }
}
