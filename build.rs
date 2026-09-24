use embed_manifest::manifest::DpiAwareness;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Per-Monitor v2 DPI, Common Controls v6 (нужны rfd), asInvoker.
        embed_manifest(new_manifest("Frostshot").dpi_awareness(DpiAwareness::PerMonitorV2))
            .expect("unable to embed manifest");

        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/frostshot.ico")
            .set("ProductName", "Frostshot")
            .set("FileDescription", "Frostshot")
            .set("LegalCopyright", "MIT OR Apache-2.0");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource not embedded: {e}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/frostshot.ico");
}
