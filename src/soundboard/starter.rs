//! The starter sounds: well-known sounds from MyInstants. Only their links are in the app, so no
//! clip of someone else ships with it. Voicenux downloads them into the soundboard folder on the
//! first start that is online, and levels them when it loads them.

use std::path::PathBuf;

use super::online::{self, Found};
use super::{RATE, SoundData, decode, decode_bytes};

pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    /// The file on MyInstants, in `/media/sounds/`.
    file: &'static str,
    /// The starter pack that brought it, see [`PACK`].
    pub since: u32,
}

/// The starter pack of this version. Pack 1 had the classic kinds as CC0 recordings in the app,
/// pack 2 has memes too, and all of them from MyInstants. A soundboard made with an older pack
/// gets the sounds that are new.
pub const PACK: u32 = 2;

const fn b(id: &'static str, name: &'static str, icon: &'static str, file: &'static str, since: u32) -> Builtin {
    Builtin { id, name, icon, file, since }
}

pub static BUILTIN: &[Builtin] = &[
    // Memes.
    b("vine-boom", "Vine Boom", "collision", "vine-boom.mp3", 2),
    b("bruh", "Bruh", "unamused-face", "movie_1.mp3", 2),
    b("emotional-damage", "Emotional Damage", "broken-heart", "emotional-damage-meme.mp3", 2),
    b("sad-violin", "Sad Violin", "violin", "tf_nemesis.mp3", 2),
    b("metal-pipe", "Metal Pipe", "nut-and-bolt", "metal-pipe-clang.mp3", 2),
    b("windows-error", "Windows Error", "warning", "erro.mp3", 2),
    b("oh-no", "Oh No No No", "face-with-hand-over-mouth", "oh-no-no-no-no-laugh.mp3", 2),
    b("rizz", "Rizz", "smirking-face", "rizz-sound-effect.mp3", 2),
    b("anime-wow", "Anime Wow", "star-struck", "anime-wow-sound-effect.mp3", 2),
    b("among-us", "Among Us", "detective", "among-us-role-reveal-sound.mp3", 2),
    b("dun-dun-dun", "Dun Dun Dun", "face-screaming-in-fear", "dun-dun-dun-sound-effect-brass_8nFBccR.mp3", 2),
    b("discord-ping", "Discord Ping", "speech-balloon", "discord-notification.mp3", 2),
    b("fart", "Fart", "pile-of-poo", "dry-fart.mp3", 2),
    // The classics of the first pack, with the same ids, so soundboards keep them.
    b("airhorn", "Air Horn", "postal-horn", "mlg-airhorn.mp3", 1),
    b("sad-trombone", "Sad Trombone", "trumpet", "sadtrombone.swf.mp3", 1),
    b("rimshot", "Rimshot", "joy", "badumtss.swf.mp3", 1),
    b("drumroll", "Drum Roll", "drum", "drum-roll-gaming-sound-effect-hd.mp3", 1),
    b("applause", "Applause", "clapping", "applause-4.mp3", 1),
    b("tada", "Ta-da!", "party-popper", "win31.mp3", 1),
    b("correct", "Correct", "bell", "correct.mp3", 1),
    b("wrong", "Wrong", "cross-mark", "family-fortunes-wrong-buzzer.mp3", 1),
    b("crickets", "Crickets", "cricket", "crickets.swf.mp3", 1),
    b("boing", "Boing", "basketball", "cartoon-boing_7vRWDlc.mp3", 1),
    b("laser", "Laser", "high-voltage", "064-laser-thum_hYpSdXS.mp3", 1),
    b("explosion", "Explosion", "bomb", "explosion-meme_dTCfAHs.mp3", 1),
    b("coin", "Coin", "coin", "mario_coin_sound.mp3", 1),
    b("whoosh", "Whoosh", "dashing-away", "whoosh-sfx.mp3", 1),
];

pub fn find(id: &str) -> Option<&'static Builtin> {
    BUILTIN.iter().find(|b| b.id == id)
}

impl Builtin {
    fn ext(&self) -> &'static str {
        self.file.rsplit_once('.').map_or("mp3", |(_, e)| e)
    }

    /// Where the download goes: a folder of its own in the soundboard folder.
    fn path(&self) -> PathBuf {
        super::dir().join("starter").join(format!("{}.{}", self.id, self.ext()))
    }

    pub fn downloaded(&self) -> bool {
        self.path().is_file()
    }

    /// Downloads the sound and keeps it, if it plays. Blocks: call it off the main thread.
    pub fn fetch(&self) -> Result<(), online::Error> {
        let found = Found {
            title: self.name.into(),
            url: format!("https://www.myinstants.com/media/sounds/{}", self.file),
            ext: self.ext().into(),
            page: None,
        };
        let bytes = online::download(&found)?;
        decode_bytes(bytes.clone(), self.ext()).map_err(|e| {
            log::warn!("starter sound {}: {e}", self.id);
            online::Error::Failed
        })?;
        let path = self.path();
        let part = path.with_extension("part");
        // Written under another name first, so a broken download never looks done.
        std::fs::create_dir_all(path.parent().unwrap())
            .and_then(|_| std::fs::write(&part, &bytes))
            .and_then(|_| std::fs::rename(&part, &path))
            .map_err(|e| {
                log::warn!("starter sound {}: {e}", self.id);
                online::Error::Failed
            })
    }
}

/// The audio of a starter sound, leveled. None while it is not downloaded.
pub fn load(id: &str) -> Option<SoundData> {
    let b = find(id)?;
    if !b.downloaded() {
        return None;
    }
    decode(&b.path()).map(level).map_err(|e| log::warn!("starter sound {id}: {e}")).ok()
}

/// Brings a sound to about -16 LUFS, with peaks at -1 dB at most, so the sounds of the pack are
/// about as loud as each other. Loudness here is the mean power of the 100 ms blocks that are
/// not silence, without the K-weighting of LUFS: close enough for short clips.
fn level(mut d: SoundData) -> SoundData {
    /// -16 LUFS in this measure: LUFS adds the power of both channels and takes 0.69 dB off.
    const TARGET_DB: f32 = -18.3;
    const PEAK: f32 = 0.89;
    let powers: Vec<f32> = d
        .frames
        .chunks(RATE as usize / 10)
        .map(|c| c.iter().map(|f| (f[0] * f[0] + f[1] * f[1]) / 2.0).sum::<f32>() / c.len() as f32)
        .filter(|p| *p > 1e-7)
        .collect();
    if powers.is_empty() {
        return d;
    }
    // Like LUFS, quiet parts 10 dB under the rest do not count.
    let mean = powers.iter().sum::<f32>() / powers.len() as f32;
    let loud: Vec<f32> = powers.into_iter().filter(|p| *p > mean * 0.1).collect();
    let db = 10.0 * (loud.iter().sum::<f32>() / loud.len() as f32).log10();
    let peak = d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let gain = 10f32.powf((TARGET_DB - db) / 20.0).min(PEAK / peak.max(1e-6));
    for f in &mut d.frames {
        *f = f.map(|x| x * gain);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_is_sound() {
        for (i, b) in BUILTIN.iter().enumerate() {
            assert!(BUILTIN[..i].iter().all(|o| o.id != b.id && o.file != b.file), "{} twice", b.id);
            assert!(b.since >= 1 && b.since <= PACK, "{}", b.id);
            assert!(["mp3", "ogg", "wav"].contains(&b.ext()), "{}", b.file);
            assert!(!b.file.contains(['/', ' ', '?', '#']) && !b.file.contains(".."), "{}", b.file);
            assert!(std::path::Path::new(&format!("data/icons/{}.png", b.icon)).exists(), "no picture for {}", b.id);
        }
    }

    #[test]
    fn levels_loud_and_quiet_sounds() {
        let tone = |amp: f32| SoundData { frames: (0..RATE).map(|i| [amp * (i as f32 * 0.05).sin(); 2]).collect() };
        let peak = |d: &SoundData| d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
        // A loud tone comes down and a quiet one up, to the same level.
        let (loud, quiet) = (peak(&level(tone(1.0))), peak(&level(tone(0.01))));
        assert!(quiet > 0.1 && quiet < 0.3, "{quiet}");
        assert!((loud - quiet).abs() < 0.01, "{loud} {quiet}");
        // A short, peaky click stops at -1 dB.
        let mut click = tone(0.0);
        click.frames[100] = [1.0, 1.0];
        click.frames[101] = [-1.0, -1.0];
        assert!(peak(&level(click)) <= 0.9);
        // Silence stays silence.
        assert_eq!(peak(&level(tone(0.0))), 0.0);
    }

    /// Talks to the real MyInstants, so only on request: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn every_starter_downloads_and_plays() {
        for b in BUILTIN {
            let found = Found {
                title: b.name.into(),
                url: format!("https://www.myinstants.com/media/sounds/{}", b.file),
                ext: b.ext().into(),
                page: None,
            };
            let bytes = online::download(&found).unwrap_or_else(|e| panic!("{}: {e:?}", b.id));
            let d = level(decode_bytes(bytes, b.ext()).unwrap());
            let seconds = d.seconds();
            assert!(seconds > 0.2 && seconds < 12.0, "{} is {seconds} s", b.id);
            assert!(d.frames.iter().all(|f| f[0].is_finite() && f[1].is_finite()), "{}", b.id);
        }
    }
}
