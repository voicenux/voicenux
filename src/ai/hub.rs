//! Finds more AI voices on Hugging Face: RVC models people made of characters. They have no tag
//! there, so this searches the names of repos and keeps those with a model, a .pth or a zip
//! with one inside, whose name has the words. Nothing goes out before you search; then only the
//! words you search for.

use std::collections::HashMap;

use serde::Deserialize;

use super::Source;
use crate::soundboard::online::call;

const SITE: &str = "https://huggingface.co";
/// Repos a search reads per page, and pages at most. Most that match a name are other kinds of
/// models, and voices have few downloads, so they come last: "goku" has them after 2000 others.
const PAGE: u32 = 1000;
const PAGES: usize = 5;
/// Hits a search shows at most.
const MAX_HITS: usize = 80;
/// A repo with more models than this shows only those whose name has a word of the search.
const MANY: usize = 6;

/// A voice the search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// What to download. Its size is 0 until [`sizes`] tells it.
    pub source: Source,
    /// People who liked the repo. Most have none.
    pub likes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No connection, or Hugging Face had a problem.
    Offline,
    /// Hugging Face wants fewer requests for a while.
    TooMany,
}

#[derive(Deserialize)]
struct Repo {
    id: String,
    sha: Option<String>,
    #[serde(default)]
    likes: u32,
    #[serde(default)]
    downloads: u64,
    library_name: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    /// false, or how one gets access.
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    siblings: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    rfilename: String,
    size: Option<u64>,
}

fn error(e: ureq::Error) -> Error {
    log::warn!("voice search: {e}");
    match e {
        ureq::Error::StatusCode(429) => Error::TooMany,
        _ => Error::Offline,
    }
}

/// Voices for `words`, the most liked first. Blocks: call it off the main thread.
pub fn search(words: &str) -> Result<Vec<Hit>, Error> {
    let started = std::time::Instant::now();
    let mut repos: Vec<Repo> = Vec::new();
    let mut url = Some(format!("{SITE}/api/models"));
    for page in 0..PAGES {
        let Some(u) = url.take() else { break };
        let mut resp = call(|a| {
            let r = a.get(&u);
            // The next pages come with all of it in their link.
            let r = if page == 0 {
                r.query("search", words)
                    .query("limit", PAGE.to_string())
                    .query("full", "true")
                    .query("sort", "downloads")
            } else {
                r
            };
            r.call()
        })
        .map_err(error)?;
        url = resp.headers().get("link").and_then(|l| l.to_str().ok()).and_then(next_page);
        let got: Vec<Repo> = resp.body_mut().with_config().limit(20 << 20).read_json().map_err(error)?;
        repos.extend(got);
    }
    let n = repos.len();
    let found = hits(repos, words);
    log::info!("voice search \"{words}\": {} voices in {n} repos, {:?}", found.len(), started.elapsed());
    Ok(found)
}

/// The next page from a Link header: `<https://...>; rel="next"`. Only one on Hugging Face.
fn next_page(link: &str) -> Option<String> {
    link.split(',').find(|l| l.contains("rel=\"next\"")).and_then(|l| {
        let url = l.split_once('<')?.1.split_once('>')?.0;
        url.starts_with(&format!("{SITE}/api/models?")).then(|| url.to_string())
    })
}

/// The size of each file of a repo, in bytes. Blocks: call it off the main thread.
pub fn sizes(repo: &str, commit: &str) -> Result<HashMap<String, u64>, Error> {
    let info: Repo =
        call(|a| a.get(format!("{SITE}/api/models/{repo}/revision/{commit}")).query("blobs", "true").call())
            .map_err(error)?
            .body_mut()
            .with_config()
            .limit(20 << 20)
            .read_json()
            .map_err(error)?;
    Ok(info.siblings.into_iter().filter_map(|f| Some((f.rfilename, f.size?))).collect())
}

/// How big a hit is to download, in MB, from the sizes of its repo. None when a file is not
/// among them.
pub fn mb(source: &Source, sizes: &HashMap<String, u64>) -> Option<u32> {
    let bytes = sizes.get(&source.file)? + source.index.as_ref().map_or(Some(0), |i| sizes.get(i).copied())?;
    Some(bytes.div_ceil(1_000_000).max(1) as u32)
}

/// The page of a repo, to read what its maker wrote about it.
pub fn page(repo: &str) -> String {
    format!("{SITE}/{repo}")
}

/// Where a file of a repo downloads, at a commit.
pub fn file_url(repo: &str, commit: &str, file: &str) -> String {
    let mut path = String::new();
    for b in file.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            path.push(b as char);
        } else {
            path.push_str(&format!("%{b:02X}"));
        }
    }
    format!("{SITE}/{repo}/resolve/{commit}/{path}")
}

/// Letters and digits only, in lowercase: "Mario_V2" and "mario v2" are the same.
fn plain(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(path: &str) -> &str {
    let b = base(path);
    b.rsplit_once('.').map_or(b, |(s, _)| s)
}

/// The voices in the repos of a search, the most liked and downloaded first.
fn hits(repos: Vec<Repo>, words: &str) -> Vec<Hit> {
    let words: Vec<String> = words.split_whitespace().map(plain).filter(|w| !w.is_empty()).collect();
    // Hugging Face also finds the words in the name of the maker: "Spongebobbbbbbbb" makes models
    // that find dancers in videos. Only voices whose own name has them.
    let named = |repo: &str, file: &str| {
        let both = plain(&format!("{} {file}", base(repo)));
        words.iter().all(|w| both.contains(w.as_str()))
    };
    let mut found: Vec<(Hit, u64)> = Vec::new();
    for r in repos {
        let open = !r.private && matches!(r.gated, serde_json::Value::Bool(false) | serde_json::Value::Null);
        // Language models, pictures, game players: RVC repos name no library, or RVC.
        let rvc = r.library_name.as_deref().is_none_or(|l| l.to_lowercase().contains("rvc"))
            && !r.tags.iter().any(|t| t == "reinforcement-learning")
            && !is_other_tool(base(&r.id));
        let Some(commit) = r.sha.filter(|s| s.len() == 40) else { continue };
        if !open || !rvc {
            continue;
        }
        let files: Vec<&str> = r.siblings.iter().map(|f| f.rfilename.as_str()).collect();
        let mut models: Vec<&str> = files.iter().copied().filter(|f| is_model(f)).collect();
        // A zip next to what it holds, unpacked: the model alone is the smaller download.
        let pths: Vec<&str> = models.iter().copied().filter(|f| f.to_lowercase().ends_with(".pth")).collect();
        models.retain(|f| {
            !f.to_lowercase().ends_with(".zip") || !pths.iter().any(|p| p.starts_with(&format!("{}/", stem(f))))
        });
        // A repo of several voices names each by its file, also when only one is shown.
        let several = models.len() > 1;
        models.retain(|f| named(&r.id, f));
        if models.len() > MANY {
            models.retain(|f| words.iter().any(|w| plain(stem(f)).contains(w.as_str())));
        }
        for file in models {
            let index = if file.to_lowercase().ends_with(".pth") { index_for(file, &files, pths.len()) } else { None };
            let name = name(file, &r.id, several);
            let source =
                Source { repo: r.id.clone(), commit: commit.clone(), file: file.to_string(), index, mb: 0, name };
            // Only what a voice could keep.
            if Source::from_value(&source.to_value()).is_some() {
                found.push((Hit { source, likes: r.likes }, r.downloads));
            }
        }
    }
    // Stable: the order of a repo stays.
    found.sort_by_key(|(h, downloads)| std::cmp::Reverse((h.likes, *downloads)));
    found.into_iter().take(MAX_HITS).map(|(h, _)| h).collect()
}

/// Whether a file of a repo can be a voice: a .pth or a zip, without what RVC needs for training
/// or other tools.
fn is_model(path: &str) -> bool {
    let name = base(path).to_lowercase();
    let kind = name.ends_with(".pth") || name.ends_with(".zip");
    // G_ and D_ are left from training, an IVF is an index on its own, the rest are base models.
    const NOT: &[&str] = &["pretrain", "hubert", "rmvpe", "f0g", "f0d", "dataset", "ivf", "logs"];
    kind && !name.starts_with("g_")
        && !name.starts_with("d_")
        && !NOT.iter().any(|n| name.contains(n))
        && !is_other_tool(&name)
}

/// Voices made for other tools, which RVC can not play.
fn is_other_tool(name: &str) -> bool {
    let name = name.to_lowercase();
    ["sovits", "so-vits", "gpt"].iter().any(|n| name.contains(n))
}

/// The index of a .pth in its repo: one that names it, or else the only one when the repo has
/// only one model. RVC calls the right one "added_...", "trained_..." is a step on the way.
fn index_for(pth: &str, files: &[&str], models: usize) -> Option<String> {
    let indexes: Vec<&str> = files
        .iter()
        .copied()
        .filter(|f| f.to_lowercase().ends_with(".index") && !base(f).to_lowercase().starts_with("trained"))
        .collect();
    let me = plain(stem(pth));
    let named = indexes.iter().find(|i| me.len() >= 3 && plain(stem(i)).contains(&me));
    let only =
        || (models == 1).then(|| indexes.iter().max_by_key(|i| base(i).to_lowercase().starts_with("added"))).flatten();
    named.or_else(only).map(|i| i.to_string())
}

/// A name to read for a voice: its file without what RVC people add, like "RVC v2" or "500
/// epochs". The repo's when the file says nothing, or when it holds a single voice and its name
/// is the clearer one.
fn name(file: &str, repo: &str, several: bool) -> String {
    let from_file = pretty(stem(file));
    let from_repo = pretty(base(repo));
    if from_file.is_empty() || (!several && !from_repo.is_empty() && from_repo.len() >= from_file.len()) {
        if from_repo.is_empty() { base(repo).to_string() } else { from_repo }
    } else {
        from_file
    }
}

/// "Spongebob_Squarepants_RVC_V2" to "Spongebob Squarepants".
pub fn pretty(raw: &str) -> String {
    let spaced: String =
        raw.chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == '&' { c } else { ' ' }).collect();
    let noise = |w: &str| {
        let w = w.to_lowercase();
        const WORDS: &[&str] = &[
            "rvc", "rvc2", "rvcv1", "rvcv2", "v1", "v2", "v3", "v4", "model", "models", "weights", "weight", "gg",
            "epoch", "epochs", "steps", "step", "ep", "voice", "voices", "vmodel", "vmodels", "ai", "by", "the",
            "final", "new", "index", "pth",
        ];
        let units = ["e", "s", "k", "ep", "epoch", "epochs", "step", "steps"];
        let number_with_unit = units.iter().any(|u| {
            w.strip_suffix(u).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                || w.strip_prefix(u).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        });
        WORDS.contains(&w.as_str()) || number_with_unit
    };
    let words: Vec<String> = spaced
        .split_whitespace()
        .filter(|w| !noise(w))
        .map(|w| {
            let mut c = w.chars();
            // "spongebob" reads better as "Spongebob". "SpongeBob" and "GLaDOS" stay.
            match c.next() {
                Some(first) if w.chars().all(|c| !c.is_uppercase()) => first.to_uppercase().chain(c).collect(),
                _ => w.to_string(),
            }
        })
        .collect();
    let mut name = words.join(" ");
    if name.chars().count() > 40 {
        name = name.chars().take(40).collect::<String>().trim_end().to_string();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn repo(id: &str, files: &[&str]) -> Repo {
        Repo {
            id: id.into(),
            sha: Some(COMMIT.into()),
            likes: 0,
            downloads: 0,
            library_name: None,
            tags: Vec::new(),
            gated: serde_json::Value::Bool(false),
            private: false,
            siblings: files.iter().map(|f| File { rfilename: f.to_string(), size: None }).collect(),
        }
    }

    fn files(hits: &[Hit]) -> Vec<(&str, &str)> {
        hits.iter().map(|h| (h.source.file.as_str(), h.source.name.as_str())).collect()
    }

    #[test]
    fn names_read_well() {
        assert_eq!(pretty("Spongebob_Squarepants_RVC_V2"), "Spongebob Squarepants");
        assert_eq!(pretty("Spongebob Squarepants - Weights.gg Model"), "Spongebob Squarepants");
        assert_eq!(pretty("Plankton__SpongeBob___RVC_v2__1K_Epochs_69K_Steps"), "Plankton SpongeBob");
        assert_eq!(pretty("MrKrabsV1_210e_4410s"), "MrKrabsV1");
        assert_eq!(pretty("darth_vader-male"), "Darth Vader Male");
        assert_eq!(pretty("SpongeBob-2005"), "SpongeBob 2005");
        assert_eq!(pretty("Shrek-VModels"), "Shrek");
        assert_eq!(pretty("model"), "");
    }

    #[test]
    fn keeps_repos_with_voices() {
        let mut gpt = repo("someone/mario-gpt", &["model.safetensors"]);
        gpt.library_name = Some("transformers".into());
        let mut game = repo("someone/ppo-mario", &["ppo-mario.zip"]);
        game.tags = vec!["reinforcement-learning".into()];
        game.library_name = Some("stable-baselines3".into());
        let mut gated = repo("someone/Mario_RVC", &["Mario.zip"]);
        gated.gated = serde_json::Value::String("manual".into());
        let mut liked = repo("someone/Mario-Kart", &["Mario Kart.zip", "added_IVF1072_Flat_nprobe_1_mario_v2.zip"]);
        liked.likes = 2;
        let found = hits(
            vec![
                gpt,
                game,
                gated,
                // Only the maker matches.
                repo("mario1990/photos", &["checkpoints/RealESRGAN_x2plus.pth"]),
                repo("MarioTime/Voices", &["HomerSimpson.zip", "Mario.zip"]),
                repo("binant/mario-male", &["model.pth", "model.index", "README.md"]),
                repo(
                    "someone/Mario_Characters",
                    &["Mario.zip", "Luigi_RVC_v2.zip", "G_2333.pth", "D_2333.pth", "hubert_base.pt"],
                ),
                liked,
            ],
            "mario",
        );
        assert_eq!(
            files(&found),
            [
                ("Mario Kart.zip", "Mario Kart"),
                ("Mario.zip", "Mario"),
                ("model.pth", "Mario Male"),
                ("Mario.zip", "Mario"),
                ("Luigi_RVC_v2.zip", "Luigi"),
            ]
        );
        assert_eq!(found[1].source.repo, "MarioTime/Voices");
        assert_eq!(found[2].source.index.as_deref(), Some("model.index"));
        assert_eq!(found[3].source.index, None);
    }

    #[test]
    fn finds_the_index_of_a_model() {
        let found = hits(
            vec![repo(
                "someone/Spongebob-RVC",
                &[
                    "Spongebob-SEP64.zip",
                    "Spongebob-SEP64/weights/Spongebob.pth",
                    "Spongebob-SEP64/logs/trained_IVF6717_Flat_Spongebob_v2.index",
                    "Spongebob-SEP64/logs/added_IVF6717_Flat_nprobe_1_Spongebob_v2.index",
                ],
            )],
            "spongebob",
        );
        // The zip holds the same as the folder next to it.
        assert_eq!(files(&found), [("Spongebob-SEP64/weights/Spongebob.pth", "Spongebob")]);
        let index = found[0].source.index.as_deref();
        assert_eq!(index, Some("Spongebob-SEP64/logs/added_IVF6717_Flat_nprobe_1_Spongebob_v2.index"));
    }

    #[test]
    fn a_big_collection_shows_what_was_searched() {
        let names: Vec<String> = (0..20).map(|i| format!("Voice{i}.zip")).chain(["Mario64.zip".into()]).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let found = hits(vec![repo("someone/RVC-Models", &names)], "mario");
        assert_eq!(files(&found), [("Mario64.zip", "Mario64")]);
    }

    #[test]
    fn reads_the_next_page() {
        let link = format!("<{SITE}/api/models?search=goku&cursor=abc>; rel=\"next\"");
        assert_eq!(next_page(&link), Some(format!("{SITE}/api/models?search=goku&cursor=abc")));
        assert_eq!(next_page("<https://example.com/api/models?x>; rel=\"next\""), None);
        assert_eq!(next_page(""), None);
    }

    #[test]
    fn sizes_add_up() {
        let s = Source {
            repo: "a/b".into(),
            commit: COMMIT.into(),
            file: "m.pth".into(),
            index: Some("added.index".into()),
            mb: 0,
            name: String::new(),
        };
        let sizes: HashMap<String, u64> = [("m.pth".into(), 55_000_000), ("added.index".into(), 20_400_000)].into();
        assert_eq!(mb(&s, &sizes), Some(76));
        assert_eq!(mb(&Source { index: Some("gone.index".into()), ..s }, &sizes), None);
    }

    #[test]
    #[ignore]
    fn finds_voices() {
        let _ = env_logger::builder().is_test(true).try_init();
        for words in ["spongebob", "goku", "darth vader", "shrek"] {
            let found = search(words).unwrap();
            for h in &found {
                eprintln!(
                    "{words}: {} ({} likes) {}/{} {:?}",
                    h.source.name, h.likes, h.source.repo, h.source.file, h.source.index
                );
            }
            assert!(!found.is_empty(), "nothing for {words}");
            let s = &found[0].source;
            assert!(mb(s, &sizes(&s.repo, &s.commit).unwrap()).is_some(), "{words}: no size");
        }
    }

    #[test]
    fn urls_are_encoded() {
        assert_eq!(
            file_url("a/b", COMMIT, "Spongebob Squarepants - Weights.gg Model.zip"),
            format!("{SITE}/a/b/resolve/{COMMIT}/Spongebob%20Squarepants%20-%20Weights.gg%20Model.zip")
        );
        assert_eq!(
            file_url("a/b", COMMIT, "x/Bläck#1.zip"),
            format!("{SITE}/a/b/resolve/{COMMIT}/x/Bl%C3%A4ck%231.zip")
        );
    }
}
