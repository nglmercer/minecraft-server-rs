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

    println!("cargo:rerun-if-changed=../../web/dist");
}
