+++
title = "Downloads"
weight = 3
+++

# Downloads

These links always give the newest stable release of Kimün. The file names have no version, so the links never change.

For most people, the [install script](@/getting-started/installation.md) is the easiest way to install Kimün and keep it up to date. Use a link on this page if you want to download a file yourself.

## Desktop app

| Platform | File |
| --- | --- |
| macOS, Apple silicon | [kimun-gui-macos-arm64.dmg](https://github.com/nico2sh/kimun/releases/latest/download/kimun-gui-macos-arm64.dmg) |
| macOS, Intel | [kimun-gui-macos-x64.dmg](https://github.com/nico2sh/kimun/releases/latest/download/kimun-gui-macos-x64.dmg) |
| Windows, x64 | [kimun-gui-windows-x64.zip](https://github.com/nico2sh/kimun/releases/latest/download/kimun-gui-windows-x64.zip) |
| Linux, x64 | [kimun-gui-linux-x64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-gui-linux-x64.tar.gz) |
| Linux, arm64 | [kimun-gui-linux-arm64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-gui-linux-arm64.tar.gz) |

The macOS file is a signed and notarized disk image. Open it and drag `Kimun.app` to your Applications folder. The Windows archive holds `kimun-gui.exe`. Unpack it and run the file. The Linux archives hold `kimun-gui` and a `share` folder with the launcher entry and the icons.

## Terminal app

| Platform | File |
| --- | --- |
| macOS, Apple silicon | [kimun-macos-arm64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-macos-arm64.tar.gz) |
| macOS, Intel | [kimun-macos-x64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-macos-x64.tar.gz) |
| Windows, x64 | [kimun-windows-x64.zip](https://github.com/nico2sh/kimun/releases/latest/download/kimun-windows-x64.zip) |
| Linux, x64 | [kimun-linux-x64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-linux-x64.tar.gz) |
| Linux, arm64 | [kimun-linux-arm64.tar.gz](https://github.com/nico2sh/kimun/releases/latest/download/kimun-linux-arm64.tar.gz) |

Each archive holds the `kimun` program at its root. Put it in a folder on your `PATH`.

## Check a download

Every release has a file named `checksums-sha256.txt`. It lists the SHA-256 checksum (a fingerprint of the file) of each download, and `checksums-sha256.txt.minisig` is its signature. Download both from the [latest release](https://github.com/nico2sh/kimun/releases/latest). To check a file on Linux or macOS, run `sha256sum <file>` (or `shasum -a 256 <file>` on macOS) and compare the result with the line for that file name.

The file names with a version, such as `kimun-gui-<version>-macos-arm64.dmg`, are on the same release page. A file with a version and the file without it are the same file.
