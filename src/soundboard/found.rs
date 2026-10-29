//! Sounds that apps found online for a key of their own, like Say a Sound of the plugin for
//! OpenDeck. They are not on the soundboard: they have a folder and a list of their own, the most
//! recently used first. The oldest go once there are more than [`KEEP`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{Sound, delete_file, dir, file_stem, free_path};

/// A key holds one sound, and few people have more than a dozen keys.
pub const KEEP: usize = 50;
/// Their folder, in the soundboard folder.
const FOLDER: &str = "found";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Found {
    #[serde(default)]
    pub sound: Vec<Sound>,
}

fn path() -> PathBuf {
    crate::config::config_dir().join("found.toml")
}

impl Found {
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let result = toml::to_string_pretty(self).map_err(anyhow::Error::from).and_then(|t| {
            std::fs::create_dir_all(crate::config::config_dir())?;
            Ok(std::fs::write(path(), t)?)
        });
        if let Err(e) = result {
            log::error!("could not save the found sounds: {e}");
        }
    }

    pub fn find(&self, id: &str) -> Option<&Sound> {
        self.sound.iter().find(|s| s.id == id)
    }

    /// The one from this file on MyInstants.
    pub fn with_source(&self, source: &str) -> Option<&Sound> {
        self.sound.iter().find(|s| s.source.as_deref() == Some(source))
    }

    /// Puts a sound first, it was just used. False when it was first already, or is not here.
    pub fn used(&mut self, id: &str) -> bool {
        match self.sound.iter().position(|s| s.id == id) {
            Some(i) if i > 0 => {
                let s = self.sound.remove(i);
                self.sound.insert(0, s);
                true
            }
            _ => false,
        }
    }

    /// Keeps a downloaded sound, first in the list. Gives it, and the sounds that had to go.
    pub fn add(
        &mut self,
        name: &str,
        ext: &str,
        data: &[u8],
        icon: &str,
        source: &str,
    ) -> anyhow::Result<(Sound, Vec<Sound>)> {
        let folder = dir().join(FOLDER);
        std::fs::create_dir_all(&folder)?;
        let target = free_path(&folder, &file_stem(name), &format!(".{ext}"));
        std::fs::write(&target, data)?;
        let file = target.file_name().unwrap_or_default().to_string_lossy();
        let mut n = 1;
        while self.find(&format!("found-{n}")).is_some() {
            n += 1;
        }
        let sound = Sound {
            id: format!("found-{n}"),
            name: name.trim().to_string(),
            icon: icon.to_string(),
            file: Some(format!("{FOLDER}/{file}")),
            builtin: None,
            volume: 1.0,
            // Most sounds there are very loud.
            level: true,
            start: 0.0,
            end: None,
            folder: None,
            favorite: false,
            source: Some(source.to_string()),
        };
        let gone = self.put(sound.clone());
        for s in &gone {
            delete_file(s);
        }
        Ok((sound, gone))
    }

    /// Puts a sound first. Gives the ones past [`KEEP`], which are out of the list then.
    fn put(&mut self, sound: Sound) -> Vec<Sound> {
        self.sound.insert(0, sound);
        self.sound.split_off(self.sound.len().min(KEEP))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(id: &str) -> Sound {
        Sound {
            id: id.into(),
            name: id.into(),
            icon: String::new(),
            file: None,
            builtin: None,
            volume: 1.0,
            level: true,
            start: 0.0,
            end: None,
            folder: None,
            favorite: false,
            source: Some(format!("https://www.myinstants.com/media/sounds/{id}.mp3")),
        }
    }

    #[test]
    fn the_last_used_comes_first() {
        let mut found = Found { sound: vec![sound("found-1"), sound("found-2")] };
        assert!(found.used("found-2"));
        assert!(!found.used("found-2"), "first already");
        assert!(!found.used("found-9"));
        assert_eq!(found.sound[0].id, "found-2");
        assert_eq!(
            found.with_source("https://www.myinstants.com/media/sounds/found-1.mp3").map(|s| s.id.as_str()),
            Some("found-1")
        );
    }

    #[test]
    fn the_oldest_go() {
        let mut found = Found::default();
        for n in 1..=KEEP {
            assert!(found.put(sound(&format!("found-{n}"))).is_empty());
        }
        let gone = found.put(sound("found-new"));
        assert_eq!(gone.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["found-1"]);
        assert_eq!((found.sound.len(), found.sound[0].id.as_str()), (KEEP, "found-new"));
    }
}
