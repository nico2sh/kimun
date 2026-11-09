# Desktop app identity files

What a package of the desktop app (`kimun-gui`) ships so the desktop shows the right name and icon. Nothing in the repo installs
these yet: the release workflow builds and publishes only the TUI (`kimun`); the desktop binary is held back (plan M11f). They
are here so that slice, or a distro package, has them ready.

| Platform | File | Used by |
|---|---|---|
| Linux | `linux/kimun.desktop`, `linux/icons/hicolor/<n>x<n>/apps/kimun.png` (16-512) | install to `share/applications/` and `share/icons/` |
| Windows | `windows/kimun.ico` | embedded in `kimun-gui.exe` by `gui/build.rs` (`winresource`); also what an installer/shortcut points at |
| macOS | `macos/kimun.icns` | the app bundle's `Contents/Resources/` (`CFBundleIconFile`); the repo builds no bundle yet |

- The window's app id is `kimun` (`APP_ID` in `gui/src/workspace/titlebar.rs`): Wayland `app_id` and X11 `WM_CLASS`. The desktop
  entry's `StartupWMClass` and `Icon` match it; `gui/src/identity_tests.rs` keeps them in step. CI lints the entry with
  `desktop-file-validate`.
- Regenerate every icon from `assets/logo.png` with `python3 assets/packaging/make-icons.py` (needs Pillow).
- Not verified here: the Windows resource (build.rs only runs on a Windows host; a failing resource compile warns and the build
  goes on without the icon), the `.icns` in a real bundle, the entry in a real desktop.
