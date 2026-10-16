//! Compiles `data/lists/*.txt` into the files the browser embeds:
//! `engine.dat` (the engine snapshot), `generic.txt` (class and id hide
//! rules) and `versions.txt`. Run by `scripts\update-adblock-lists.ps1`.

use std::fs;
use std::path::PathBuf;

fn main() {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
    let mut names: Vec<_> = fs::read_dir(data.join("lists"))
        .expect("data/lists")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    names.sort();
    let lists: Vec<String> = names
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("list is UTF-8");
            // AdGuard's DNS list ships allow-rules for whole hosts (awin1,
            // appsflyer, onetrust...) so DNS filtering does not break apps.
            // In a browser they would undo blocks the other lists make.
            if path.file_stem().is_some_and(|stem| stem == "adguard-dns") {
                text.lines()
                    .filter(|line| !line.starts_with("@@"))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                text
            }
        })
        .collect();
    let texts = || lists.iter().map(String::as_str);

    fs::write(
        data.join("engine.dat"),
        agentgo_adblock::compile_snapshot(texts()),
    )
    .unwrap();
    fs::write(
        data.join("generic.txt"),
        agentgo_adblock::generic_selectors(texts()),
    )
    .unwrap();

    let mut versions = String::new();
    for (path, text) in names.iter().zip(&lists) {
        let find = |key: &str| {
            text.lines()
                .take(30)
                .find_map(|line| line.strip_prefix(key))
                .map(|value| value.trim().to_string())
        };
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let title = find("! Title:").unwrap_or(stem);
        let version = find("! Version:")
            .or_else(|| find("! Last modified:"))
            .or_else(|| find("! Updated:"))
            .unwrap_or_else(|| "snapshot".into());
        versions.push_str(&format!("{title}\t{version}\n"));
    }
    fs::write(data.join("versions.txt"), versions).unwrap();
    println!("snapshot written for {} lists", names.len());
}
