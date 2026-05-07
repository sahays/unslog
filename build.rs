#![allow(clippy::unwrap_used)]
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=static/css/input.css");
    watch_dir("templates");

    let status = Command::new("npx")
        .args([
            "@tailwindcss/cli",
            "-i",
            "static/css/input.css",
            "-o",
            "static/css/app.css",
            "--minify",
        ])
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => println!("cargo:warning=Tailwind CSS build exited with {s}"),
        Err(e) => println!("cargo:warning=Tailwind CSS build skipped (npx not found: {e})"),
    }
}

fn watch_dir(dir: &str) {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = std::path::Path::new(&manifest).join(dir);
    if let Ok(entries) = std::fs::read_dir(&path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(s) = p.strip_prefix(&manifest).ok().and_then(|r| r.to_str()) {
                    watch_dir(s);
                }
            } else if let Some(s) = p.strip_prefix(&manifest).ok().and_then(|r| r.to_str()) {
                println!("cargo:rerun-if-changed={s}");
            }
        }
    }
}
