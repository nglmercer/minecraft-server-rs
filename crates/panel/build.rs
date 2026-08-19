use std::path::PathBuf;

/// Guarantee the embedded-asset directory exists before the derive runs.
///
/// `web/dist` is a build artifact and is not committed, but `#[derive(RustEmbed)]`
/// fails at compile time if its folder is missing. Without this, a fresh clone
/// cannot `cargo build` at all — it dies on a confusing macro error long before
/// reaching the "frontend not built" page that handles exactly this case.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let dist = manifest.join("../../web/dist");

    if let Err(e) = std::fs::create_dir_all(&dist) {
        // Not fatal on its own: the derive below produces the real error if the
        // directory genuinely cannot be used.
        println!("cargo:warning=could not create {}: {e}", dist.display());
    }

    // Every embedded file is declared individually. Watching only the directory
    // is not enough: `rust-embed` bakes the files in at compile time, and cargo
    // will happily skip recompiling when a nested asset changed but the top
    // directory's own timestamp did not — leaving a stale UI inside a binary
    // that was just rebuilt.
    println!("cargo:rerun-if-changed={}", dist.display());
    watch(&dist);
}

/// Declare every file under `dir` as a build input.
fn watch(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());

        if path.is_dir() {
            watch(&path);
        }
    }
}
