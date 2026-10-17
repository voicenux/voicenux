use std::io::Write;

mod ai;
mod api;
mod app;
mod audio;
mod config;
mod dsp;
mod i18n;
mod order;
mod shortcuts;
mod soundboard;
mod state;
mod tray;
mod ui;
mod voice;

fn main() -> gtk::glib::ExitCode {
    // GTK's Vulkan renderer misses about 6 % of frames on NVIDIA (measured at 180 Hz), GL almost
    // none. Someone who sets GSK_RENDERER keeps their choice.
    if std::env::var_os("GSK_RENDERER").is_none() && std::path::Path::new("/sys/module/nvidia").exists() {
        // SAFETY: first thing in main, no other threads yet.
        unsafe { std::env::set_var("GSK_RENDERER", "gl") };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Commands that only read go without starting the app.
    match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            println!("{}", app::usage());
            return gtk::glib::ExitCode::SUCCESS;
        }
        Some("--version") => {
            println!("voicenux {}", env!("CARGO_PKG_VERSION"));
            return gtk::glib::ExitCode::SUCCESS;
        }
        Some("list") => {
            let mut voices = voice::builtin();
            voices.extend(voice::load_custom(&voice::custom_dir()));
            let mut out = std::io::stdout().lock();
            for v in voices {
                // Ignore a closed pipe, like `voicenux list | head`.
                let _ = writeln!(out, "{:14} {}", v.id, i18n::t_dyn(&v.name));
            }
            return gtk::glib::ExitCode::SUCCESS;
        }
        Some("sounds") => {
            let mut out = std::io::stdout().lock();
            for s in soundboard::Library::load().sound {
                let _ = writeln!(out, "{:14} {}", s.id, i18n::t_dyn(&s.name));
            }
            return gtk::glib::ExitCode::SUCCESS;
        }
        Some("--headless") => {
            headless(args.get(1).cloned().unwrap_or_else(|| "robot".into()), args.get(2).cloned());
            return gtk::glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    app::run()
}

/// Runs only the audio engine, without a window. For tests.
fn headless(voice_id: String, input: Option<String>) {
    let (tx, rx) = async_channel::unbounded();
    let engine = audio::Engine::start(tx, false);
    let v = voice::builtin().into_iter().find(|v| v.id == voice_id).expect("no such voice");
    engine.send(audio::Ctl::Voice(v.block_values()));
    engine.send(audio::Ctl::Input(input));
    while let Ok(e) = rx.recv_blocking() {
        println!("{e:?}");
    }
}
