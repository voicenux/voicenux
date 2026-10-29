//! The starter sounds: well-known sounds from MyInstants. Only their links are in the app, so no
//! clip of someone else ships with it. Voicenux downloads them into the soundboard folder on the
//! first start that is online, and levels them when it loads them.

use std::path::PathBuf;

use super::online::{self, Found};
use super::{SoundData, decode, decode_bytes, level};

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

    /// Its file on MyInstants.
    pub fn url(&self) -> String {
        format!("https://www.myinstants.com/media/sounds/{}", self.file)
    }

    pub fn downloaded(&self) -> bool {
        self.path().is_file()
    }

    /// Downloads the sound and keeps it, if it plays. Blocks: call it off the main thread.
    pub fn fetch(&self) -> Result<(), online::Error> {
        let found = Found { title: self.name.into(), url: self.url(), ext: self.ext().into(), page: None, id: 0 };
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
                id: 0,
            };
            let bytes = online::download(&found).unwrap_or_else(|e| panic!("{}: {e:?}", b.id));
            let d = level(decode_bytes(bytes, b.ext()).unwrap());
            let seconds = d.seconds();
            assert!(seconds > 0.2 && seconds < 12.0, "{} is {seconds} s", b.id);
            assert!(d.frames.iter().all(|f| f[0].is_finite() && f[1].is_finite()), "{}", b.id);
        }
    }
}
