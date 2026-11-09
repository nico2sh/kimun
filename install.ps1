# Kimün installer for Windows: downloads the latest stable release, verifies its
# checksum, installs the app into a user-writable folder, and records an install
# marker so the app knows it may update itself (the "script" channel).
#
#   irm https://kimun.2co.dev/install.ps1 | iex                                           # the terminal app
#   & ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Gui                # the desktop app
#   $env:KIMUN_GUI = 1; irm https://kimun.2co.dev/install.ps1 | iex                        # the same, in one line
#
# Run it once for each app you want; both go in one folder and share one marker.
# No admin needed. Running it again updates the app (an open one keeps running
# until it quits).
#
# Options:
#   -Gui                install the desktop app (kimun-gui) instead of the terminal app
#
# Env overrides:
#   KIMUN_GUI=1         the same as -Gui, for the `irm | iex` form
#   KIMUN_INSTALL_DIR   install location (default: %LOCALAPPDATA%\Programs\kimun)
#   KIMUN_VERSION       install this version instead of the latest stable release
#                       (a pre-release too, e.g. 0.25.0-beta.1)
#   KIMUN_DOWNLOAD_BASE where the release's files are fetched from (default: the
#                       release's GitHub download URL); for testing
#
# The desktop app also gets a Start-menu shortcut. The folder is added to your
# user PATH, so `kimun` and `kimun-gui` work in a new terminal.

param([switch]$Gui)

# Its own scope: under `irm | iex` it would otherwise set the caller's variables.
& {
param([bool]$WantGui)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# throw, not exit: under `irm | iex`, exit would close the person's window.
function Fail($why) { throw "kimun: $why" }

$repo = "nico2sh/kimun"
$tagPrefix = "kimun-notes-v"
$bin = if ($WantGui) { "kimun-gui" } else { "kimun" }

$dir = if ($env:KIMUN_INSTALL_DIR) { $env:KIMUN_INSTALL_DIR.TrimEnd("\", "/") } else { Join-Path $env:LOCALAPPDATA "Programs\kimun" }
# Must match the app: %USERPROFILE%\kimun (see kimun_core::system::app_dir).
$configDir = Join-Path $env:USERPROFILE "kimun"
$marker = Join-Path $configDir "install.toml"

$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne "AMD64") { Fail "no build for $arch Windows yet (x64 only)" }
$platform = "windows-x64"

# Windows' own tar: Git Bash's, often first on PATH, takes C:\ for a remote host.
$tar = Join-Path $env:SystemRoot "System32\tar.exe"
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue) -or -not (Test-Path $tar)) {
    Fail "needs curl.exe and tar.exe (Windows 10 1803 or newer)"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) "kimun-install-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    # The latest *stable* app release. /releases/latest cannot be used: this repo
    # publishes several tag series (kimun-notes-v* for the app, kimun_core-v*,
    # kimun_server-v*), and "latest" is the newest of any. List them (newest first)
    # and take the first app tag with no pre-release suffix (a hyphen).
    $version = $env:KIMUN_VERSION
    if ($version) {
        Write-Host "Version (pinned): $version"
    } else {
        Write-Host "Resolving latest release..."
        curl.exe -fsSL "https://api.github.com/repos/$repo/releases?per_page=100" -o "$tmp\releases.json"
        if ($LASTEXITCODE) { Fail "could not reach the GitHub releases API" }
        $releases = Get-Content -Raw "$tmp\releases.json" | ConvertFrom-Json
        $version = $releases | ForEach-Object { $_.tag_name } |
            Where-Object { $_ -like "$tagPrefix*" } |
            ForEach-Object { $_.Substring($tagPrefix.Length) } |
            Where-Object { $_ -notmatch "-" } |
            Select-Object -First 1
        if (-not $version) { Fail "could not find a stable $tagPrefix* release" }
        Write-Host "Latest version: $version"
    }
    $tag = "$tagPrefix$version"

    $archive = "$bin-$version-$platform.zip"
    $base = if ($env:KIMUN_DOWNLOAD_BASE) { $env:KIMUN_DOWNLOAD_BASE.TrimEnd("/") } else { "https://github.com/$repo/releases/download/$tag" }

    Write-Host "Downloading $archive..."
    curl.exe -fsSL "$base/$archive" -o "$tmp\$archive"
    if ($LASTEXITCODE) {
        # The release is there (its checksums are) but this app's build is not,
        # or the release is so new its files are still being attached.
        # No -S and no 2>$null: under Stop, Windows PowerShell 5.1 turns redirected native stderr into an error.
        curl.exe -fsL "$base/checksums-sha256.txt" -o "$tmp\checksums-sha256.txt"
        if (-not $LASTEXITCODE) {
            if ($WantGui) { Fail "release $version has no desktop app build for $platform. Not every release has one yet: see https://github.com/$repo/releases, or pin a release that does with `$env:KIMUN_VERSION" }
            Fail "release $version has no $bin build for $platform (see https://github.com/$repo/releases)"
        }
        Fail "failed to download $archive (is $version a release? A release published minutes ago may still be attaching its files: try again shortly)"
    }

    # Verify against the published checksums file: an exact name match, the digest
    # from .NET (Get-FileHash fails when PowerShell 7 starts 5.1).
    Write-Host "Verifying checksum..."
    curl.exe -fsSL "$base/checksums-sha256.txt" -o "$tmp\checksums-sha256.txt"
    if ($LASTEXITCODE) { Fail "failed to download checksums-sha256.txt" }
    $expected = $null
    foreach ($line in Get-Content "$tmp\checksums-sha256.txt") {
        $f = $line.Trim() -split "\s+", 2
        if ($f.Count -eq 2 -and $f[1].TrimStart("*") -eq $archive) { $expected = $f[0].ToLowerInvariant(); break }
    }
    if (-not $expected) { Fail "no checksum found for $archive" }
    $stream = [IO.File]::OpenRead("$tmp\$archive")
    try { $actual = -join ([Security.Cryptography.SHA256]::Create().ComputeHash($stream) | ForEach-Object { $_.ToString("x2") }) } finally { $stream.Close() }
    if ($actual -ne $expected) { Fail "checksum mismatch for $archive (expected $expected, got $actual)" }

    Write-Host "Extracting..."
    $stage = Join-Path $tmp "stage"
    New-Item -ItemType Directory -Force $stage | Out-Null
    & $tar -xf "$tmp\$archive" -C $stage
    if ($LASTEXITCODE) { Fail "failed to extract $archive" }
    if (-not (Test-Path "$stage\$bin.exe")) { Fail "binary '$bin.exe' not found in archive" }

    # A running app's exe can be renamed, not deleted or overwritten: the old one
    # goes aside, and is removed now if it can be, else on the next install.
    New-Item -ItemType Directory -Force $dir | Out-Null
    $exe = Join-Path $dir "$bin.exe"
    Remove-Item -Force "$exe.old" -ErrorAction SilentlyContinue
    if (Test-Path $exe) { Move-Item -Force $exe "$exe.old" }
    Move-Item -Force "$stage\$bin.exe" $exe
    Remove-Item -Force "$exe.old" -ErrorAction SilentlyContinue
    Write-Host "Installed $bin to $exe"

    # Start-menu shortcut, desktop app only (a terminal app wants a terminal).
    if ($WantGui) {
        $name = "Kim$([char]0xFC)n.lnk"
        $menu = Join-Path ([Environment]::GetFolderPath("Programs")) $name
        $link = (New-Object -ComObject WScript.Shell).CreateShortcut($menu)
        $link.TargetPath = $exe
        $link.WorkingDirectory = $env:USERPROFILE
        $link.Description = "Kim$([char]0xFC)n notes"
        $link.Save()
        Write-Host "Added a Start-menu shortcut"
    }

    # The install marker (the "script" channel contract the app reads). One marker
    # for both apps, naming the one folder both live in. A TOML literal string
    # (single quotes) so the backslashes need no escaping.
    New-Item -ItemType Directory -Force $configDir | Out-Null
    $text = "# Kim$([char]0xFC)n install marker $([char]0x2014) written by install.ps1. Safe to delete; the app will`n" +
        "# then fall back to detecting the install channel from the binary path.`n" +
        "channel = `"script`"`n" +
        "install_dir = '$dir'`n" +
        "version = `"$version`"`n"
    [IO.File]::WriteAllText($marker, $text, (New-Object Text.UTF8Encoding $false))
    Write-Host "Wrote install marker: $marker"

    # The raw user PATH, so %VARIABLES% in it stay unexpanded and keep their type.
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    try {
        $raw = [string]$key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $listed = $raw -split ";" | Where-Object { $_.Trim().TrimEnd("\") -ieq $dir.TrimEnd("\") }
        if (-not $listed) {
            $kind = if ($key.GetValueNames() -contains "Path") { $key.GetValueKind("Path") } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
            $key.SetValue("Path", $(if ($raw) { "$($raw.TrimEnd(';'));$dir" } else { $dir }), $kind)
            # Setting any user variable tells running programs the environment changed.
            [Environment]::SetEnvironmentVariable("KIMUN_INSTALLING", "1", "User")
            [Environment]::SetEnvironmentVariable("KIMUN_INSTALLING", $null, "User")
            Write-Host "Added $dir to your PATH (open a new terminal to use it)"
        }
    } finally {
        $key.Close()
    }
    if (($env:Path -split ";") -notcontains $dir) { $env:Path += ";$dir" }

    Write-Host ""
    if ($WantGui) { Write-Host "Done. Open Kim$([char]0xFC)n from the Start menu, or run: $bin" }
    else { Write-Host "Done. Run '$bin' to start." }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
} -WantGui ($Gui.IsPresent -or $env:KIMUN_GUI -eq "1" -or $env:KIMUN_GUI -eq "true")
# Not left behind in the caller's session (the `irm | iex` form runs in it).
Remove-Item Env:KIMUN_GUI -ErrorAction SilentlyContinue
