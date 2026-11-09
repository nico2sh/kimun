//! The app's identity files (`assets/packaging`): what a package ships so the
//! desktop shows the right name and icon. The repo does not install them yet
//! (the desktop binary is unpublished); these tests keep them consistent with
//! each other and with the window's application id, so a package that picks
//! them up works.
#![cfg(test)]

use crate::workspace::titlebar::APP_ID;

const DESKTOP: &str = include_str!("../../assets/packaging/linux/kimun.desktop");

macro_rules! hicolor {
    ($($n:literal),*) => {
        [$(($n, &include_bytes!(concat!(
            "../../assets/packaging/linux/icons/hicolor/",
            stringify!($n), "x", stringify!($n), "/apps/kimun.png"
        ))[..])),*]
    };
}

fn entry(key: &str) -> Option<&'static str> {
    DESKTOP
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(str::trim_end)
}

#[test]
fn the_desktop_entry_names_the_window_and_the_binary() {
    // A window's app id is how the desktop pairs it with its entry.
    assert_eq!(entry("StartupWMClass"), Some(APP_ID));
    assert_eq!(entry("Icon"), Some("kimun"), "the hicolor icons' name");
    assert!(
        entry("Exec").is_some_and(|e| e.starts_with(env!("CARGO_PKG_NAME"))),
        "Exec names the binary"
    );
    assert_eq!(entry("Type"), Some("Application"));
    assert_eq!(entry("Terminal"), Some("false"));
}

#[test]
fn every_hicolor_icon_is_the_size_its_folder_says() {
    for (n, png) in hicolor!(16, 24, 32, 48, 64, 128, 256, 512) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{n}: a PNG");
        // IHDR: width and height, big-endian, right after the signature.
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), (n, n), "{n}x{n}/apps/kimun.png");
    }
}

#[test]
fn the_windows_and_macos_icons_are_what_their_formats_start_with() {
    let ico = include_bytes!("../../assets/packaging/windows/kimun.ico");
    // ICONDIR: reserved 0, type 1 (icon), then the image count.
    assert_eq!(&ico[..4], &[0, 0, 1, 0]);
    assert!(u16::from_le_bytes([ico[4], ico[5]]) >= 4, "several sizes");
    let icns = include_bytes!("../../assets/packaging/macos/kimun.icns");
    assert_eq!(&icns[..4], b"icns");
}
