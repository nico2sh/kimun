//! Windows only: embed the app icon and the version info in the exe, so
//! Explorer, the taskbar and the Start menu show the logo and the file's
//! properties say what it is. Other platforms take the icon from the desktop
//! entry (Linux) or the bundle (macOS); see `assets/packaging`.
//!
//! A resource that cannot be built (no resource compiler) is a warning, not a
//! failed build: the app runs the same without its icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/packaging/windows/kimun.ico");
    println!("cargo:rerun-if-changed=../tui/Cargo.toml");
    // `cfg!(windows)` would test the host; the target is what gets the resource.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../assets/packaging/windows/kimun.ico")
        .set("ProductName", "Kimün")
        .set("FileDescription", "Kimün, plain-text notes");
    // This crate is unpublished (0.0.0); the release version is the TUI's.
    if let Some(v) = release_version() {
        res.set("FileVersion", &v).set("ProductVersion", &v);
    }
    if let Err(e) = res.compile() {
        println!("cargo:warning=the exe's icon resource was not built: {e}");
    }
}

/// The `version = "x.y.z"` of `../tui/Cargo.toml`'s `[package]`.
fn release_version() -> Option<String> {
    let toml = std::fs::read_to_string("../tui/Cargo.toml").ok()?;
    let v = toml
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))?
        .split('"')
        .next()?;
    Some(v.to_string())
}
