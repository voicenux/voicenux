//! Packs everything in `data/` that the app shows into one GResource.

use std::fmt::Write;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n");
    xml.push_str("  <gresource prefix=\"/io/github/voicenux/Voicenux\">\n");
    xml.push_str("    <file>style.css</file>\n");
    xml.push_str("    <file alias=\"logo.svg\">app/io.github.voicenux.Voicenux.svg</file>\n");
    xml.push_str("    <file alias=\"hicolor/scalable/apps/io.github.voicenux.Voicenux.svg\">app/io.github.voicenux.Voicenux.svg</file>\n");
    let mut icons: Vec<String> = std::fs::read_dir("data/icons")
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".png"))
        .collect();
    icons.sort();
    for icon in icons {
        writeln!(xml, "    <file>icons/{icon}</file>").unwrap();
    }
    xml.push_str("  </gresource>\n</gresources>\n");
    let xml_path = format!("{out}/voicenux.gresource.xml");
    std::fs::write(&xml_path, xml).unwrap();
    glib_build_tools::compile_resources(&["data"], &xml_path, "voicenux.gresource");
    println!("cargo:rerun-if-changed=data");
    println!("cargo:rerun-if-changed=po");
}
