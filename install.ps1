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
#   -Uninstall          remove what this script installed (the terminal app, or with -Gui the
#                       desktop app). Your notes and configuration are not touched
#
# Env overrides:
#   KIMUN_GUI=1         the same as -Gui, for the `irm | iex` form
#   KIMUN_UNINSTALL=1   the same as -Uninstall, for the `irm | iex` form
#   KIMUN_INSTALL_DIR   install location (default: %LOCALAPPDATA%\Programs\kimun)
#   KIMUN_VERSION       install this version instead of the latest stable release
#                       (a pre-release too, e.g. 0.25.0-beta.1)
#   KIMUN_DOWNLOAD_BASE where the release's files are fetched from (default: the
#                       release's GitHub download URL); for testing
#   KIMUN_MANIFEST_URL  the release manifest (default: https://kimun.2co.dev/latest.json);
#                       for testing
#   KIMUN_RELEASES_API  the GitHub releases list used when the manifest cannot be read
#                       (default: the repository's api.github.com URL); for testing
#
# The desktop app also gets a Start-menu shortcut. The folder is added to your
# user PATH, so `kimun` and `kimun-gui` work in a new terminal.

param([switch]$Gui, [switch]$Uninstall)

# Its own scope: under `irm | iex` it would otherwise set the caller's variables.
# The try/finally clears the switches even when the script block throws: a left-over
# KIMUN_UNINSTALL=1 would make the caller's next install run the uninstaller.
try {
& {
param([bool]$WantGui, [bool]$WantUninstall)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# throw, not exit: under `irm | iex`, exit would close the person's window.
function Fail($why) { throw "kimun: $why" }

# Releases carry checksums-sha256.txt.minisig, a minisign signature of their checksums file
# (.github/RELEASING.md, "Signing"). $signingPubKey is release-signing.pub, line 2; empty means
# signing is not live yet and nothing is checked. With a key and minisign installed the signature
# is checked and the digest comes from the file it signed. A missing signature is an error for the
# latest release (what the unsigned manifest names) and a warning for a version pinned with
# KIMUN_VERSION, which may predate signing. Without minisign the install goes on on the sha256
# alone, and says so.
$signingPubKey = ""

$repo = "nico2sh/kimun"
$tagPrefix = "kimun-notes-v"
$bin = if ($WantGui) { "kimun-gui" } else { "kimun" }

$dir = if ($env:KIMUN_INSTALL_DIR) { $env:KIMUN_INSTALL_DIR.TrimEnd("\", "/") } else { Join-Path $env:LOCALAPPDATA "Programs\kimun" }
# Must match the app: %USERPROFILE%\kimun (see kimun_core::system::app_dir).
$configDir = Join-Path $env:USERPROFILE "kimun"
$marker = Join-Path $configDir "install.toml"

if ($WantUninstall) {
    # Remove what the installer put in place: the app's exe, for the desktop app its
    # Start-menu shortcut, and, once neither app is left, the marker, the PATH entry and
    # the folder (only if empty). Notes and configuration are not touched. The folder is
    # the one the marker records, unless KIMUN_INSTALL_DIR says otherwise.
    # The marker names the folder of the install it belongs to. The installer writes it as a
    # TOML literal string (single quotes); a hand-edited double-quoted one is read too.
    $markerDir = ""
    if (Test-Path $marker) {
        foreach ($l in Get-Content $marker) {
            if ($l -match "^install_dir = '(.*)'\s*$") { $markerDir = $Matches[1]; break }
            if ($l -match '^install_dir = "(.*)"\s*$') { $markerDir = $Matches[1].Replace('\\', '\').Replace('\"', '"'); break }
        }
        $markerDir = $markerDir.TrimEnd("\", "/")
    }
    if (-not $env:KIMUN_INSTALL_DIR) {
        if ($markerDir) { $dir = $markerDir }
        elseif (Test-Path $marker) { Write-Warning "could not read install_dir from $marker : using $dir" }
    }
    $removed = $false
    # A running app's exe cannot be deleted but can be renamed: it goes to .old and the
    # next install or uninstall removes it.
    $exe = Join-Path $dir "$bin.exe"
    foreach ($f in @("$exe.old", $exe)) {
        if (Test-Path $f) {
            try { Remove-Item -Force $f; Write-Host "Removed $f"; $removed = $true }
            catch {
                if ($f -eq $exe) {
                    # The exe stays when this fails too, so the marker and PATH entry stay for the re-run.
                    try {
                        Move-Item -Force $exe "$exe.old"
                        Write-Warning "$bin is running: close it, then run the uninstall again to remove $exe.old"
                        $removed = $true
                    } catch {
                        Write-Warning "$bin is running and could not be renamed: close it, then run the uninstall again"
                    }
                } else {
                    Write-Warning "could not remove $f : close the app, then run the uninstall again"
                }
            }
        }
    }
    if ($WantGui) {
        $programs = [Environment]::GetFolderPath("Programs")
        if (-not $programs) { $programs = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs" }
        $menu = Join-Path $programs "Kim$([char]0xFC)n.lnk"
        if (Test-Path $menu) { Remove-Item -Force $menu; Write-Host "Removed $menu"; $removed = $true }
    }
    # A leftover .old file is an app that is still there: the second run needs the marker and
    # the PATH entry to find it.
    $left = $false
    foreach ($n in @("kimun.exe", "kimun-gui.exe", "kimun.exe.old", "kimun-gui.exe.old")) {
        if (Test-Path (Join-Path $dir $n)) { $left = $true }
    }
    if (-not $left) {
        # The marker goes only when it belongs to this folder (or names none we could read).
        if ((Test-Path $marker) -and (-not $markerDir -or $markerDir -ieq $dir.TrimEnd("\", "/"))) {
            Remove-Item -Force $marker; Write-Host "Removed $marker"; $removed = $true
        }
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
        try {
            $raw = [string]$key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $parts = @($raw -split ";" | Where-Object { $_ -ne "" })
            # An entry matches as written or once its %VARIABLES% are expanded.
            $target = $dir.TrimEnd("\")
            $kept = @($parts | Where-Object {
                $entry = $_.Trim().TrimEnd("\")
                ($entry -ine $target) -and ([Environment]::ExpandEnvironmentVariables($entry).TrimEnd("\") -ine $target)
            })
            if ($kept.Count -ne $parts.Count) {
                $kind = $key.GetValueKind("Path")
                $key.SetValue("Path", ($kept -join ";"), $kind)
                [Environment]::SetEnvironmentVariable("KIMUN_INSTALLING", "1", "User")
                [Environment]::SetEnvironmentVariable("KIMUN_INSTALLING", $null, "User")
                Write-Host "Removed $dir from your PATH"
                $removed = $true
            }
        } finally {
            $key.Close()
        }
        if ((Test-Path $dir) -and -not (Get-ChildItem -Force $dir)) { Remove-Item -Force $dir; Write-Host "Removed $dir" }
    }
    if ($removed) { Write-Host "Done. Your notes and configuration in $configDir were not touched." }
    else { Write-Host "Nothing to remove: no $bin found in $dir." }
    return
}

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
    # The latest *stable* app release: from the manifest (latest.json, see
    # .github/scripts/release_manifest.py), else from the GitHub API. The API's
    # /releases/latest cannot be used: this repo publishes several tag series
    # (kimun-notes-v* for the app, kimun_core-v*, kimun_server-v*), and "latest" is the
    # newest of any. So that list (newest first, and only the newest 100 across all
    # series) is scanned for the first app tag with no pre-release suffix (a hyphen).
    $manifestUrl = if ($env:KIMUN_MANIFEST_URL) { $env:KIMUN_MANIFEST_URL } else { "https://kimun.2co.dev/latest.json" }
    $releasesApi = if ($env:KIMUN_RELEASES_API) { $env:KIMUN_RELEASES_API } else { "https://api.github.com/repos/$repo/releases?per_page=100" }
    $app = $null   # the manifest's "app" section, when it was used
    $version = $env:KIMUN_VERSION
    if ($version) {
        Write-Host "Version (pinned): $version"
    } else {
        Write-Host "Resolving latest release..."
        # Anything unexpected about the manifest is not an error: say why and use the API.
        $why = $null
        curl.exe -fsL $manifestUrl -o "$tmp\latest.json"
        if ($LASTEXITCODE) {
            $why = "cannot reach $manifestUrl"
        } else {
            try { $m = Get-Content -Raw "$tmp\latest.json" | ConvertFrom-Json } catch { $m = $null }
            if (-not $m -or $m.schema -ne 1) { $why = "$manifestUrl is in a format this installer does not know" }
            elseif (-not $m.app -or $m.app.version -notmatch '^[0-9A-Za-z.+-]+$') { $why = "$manifestUrl names no app release" }
            else { $app = $m.app; $version = $m.app.version }
        }
        if ($why) {
            Write-Warning "$why; using the GitHub API"
            curl.exe -fsSL $releasesApi -o "$tmp\releases.json"
            if ($LASTEXITCODE) { Fail "could not reach the GitHub releases API" }
            $releases = Get-Content -Raw "$tmp\releases.json" | ConvertFrom-Json
            $version = $releases | ForEach-Object { $_.tag_name } |
                Where-Object { $_ -like "$tagPrefix*" } |
                ForEach-Object { $_.Substring($tagPrefix.Length) } |
                Where-Object { $_ -notmatch "-" } |
                Select-Object -First 1
            if (-not $version) { Fail "could not find a stable $tagPrefix* release" }
        }
        Write-Host "Latest version: $version"
    }
    $tag = "$tagPrefix$version"

    $archive = "$bin-$version-$platform.zip"
    $base = if ($env:KIMUN_DOWNLOAD_BASE) { $env:KIMUN_DOWNLOAD_BASE.TrimEnd("/") } else { "https://github.com/$repo/releases/download/$tag" }

    # The manifest names this file's url and sha256: use them, unless the files are being
    # fetched from somewhere else (KIMUN_DOWNLOAD_BASE). A file it does not list is looked
    # for the usual way, so the errors below stay the same.
    $archiveUrl = "$base/$archive"
    $expected = $null
    if ($app -and -not $env:KIMUN_DOWNLOAD_BASE) {
        $entry = $app.assets.PSObject.Properties[$archive]
        if ($entry -and $entry.Value.url -and $entry.Value.sha256) {
            $archiveUrl = $entry.Value.url
            $expected = ([string]$entry.Value.sha256).ToLowerInvariant()
        }
    }

    Write-Host "Downloading $archive..."
    curl.exe -fsSL $archiveUrl -o "$tmp\$archive"
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

    # Verify the checksum: the signed list when minisign can check it, else the manifest's,
    # else the release's published checksums file (an exact name match). The digest is from
    # .NET (Get-FileHash fails when PowerShell 7 starts 5.1).
    Write-Host "Verifying checksum..."
    $signed = $false
    $sums = "$tmp\checksums-sha256.txt"
    $minisign = Get-Command minisign.exe -ErrorAction SilentlyContinue
    $haveSums = $false
    if (-not $signingPubKey) {
        # signing is not live yet
    } elseif (-not $minisign) {
        Write-Warning "minisign is not installed: the release signature was not checked (sha256 only)"
    } else {
        # No -S and no redirects: under Stop, Windows PowerShell 5.1 turns redirected native stderr into an error.
        curl.exe -fsL "$base/checksums-sha256.txt" -o $sums
        $haveSums = -not $LASTEXITCODE
        curl.exe -fsL "$base/checksums-sha256.txt.minisig" -o "$sums.minisig"
        if ($haveSums -and -not $LASTEXITCODE) {
            & $minisign.Source -V -q -P $signingPubKey -m $sums -x "$sums.minisig" | Out-Null
            if ($LASTEXITCODE) { Fail "the signature of checksums-sha256.txt is NOT valid: refusing to install" }
            $expected = $null   # the signed list is the authority, not the manifest
            $signed = $true
        } elseif ($env:KIMUN_VERSION) {
            Write-Warning "this release has no signature (it may predate signing): sha256 only"
        } else {
            Fail "the latest release should be signed but its signature could not be fetched: refusing to install"
        }
    }
    if (-not $expected) {
        if (-not $haveSums) {
            curl.exe -fsSL "$base/checksums-sha256.txt" -o $sums
            if ($LASTEXITCODE) { Fail "failed to download checksums-sha256.txt" }
        }
        foreach ($line in Get-Content $sums) {
            $f = $line.Trim() -split "\s+", 2
            if ($f.Count -eq 2 -and $f[1].TrimStart("*") -eq $archive) { $expected = $f[0].ToLowerInvariant(); break }
        }
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
        # A shortcut is a convenience: failing to make one must not fail the install.
        try {
            $programs = [Environment]::GetFolderPath("Programs")
            # Empty when the profile's folders cannot be resolved (a stripped-down account).
            if (-not $programs) { $programs = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs" }
            New-Item -ItemType Directory -Force $programs | Out-Null
            $menu = Join-Path $programs "Kim$([char]0xFC)n.lnk"
            $link = (New-Object -ComObject WScript.Shell).CreateShortcut($menu)
            $link.TargetPath = $exe
            $link.WorkingDirectory = $env:USERPROFILE
            $link.Description = "Kim$([char]0xFC)n notes"
            $link.Save()
            Write-Host "Added a Start-menu shortcut"
        } catch {
            Write-Warning "could not add the Start-menu shortcut: $($_.Exception.Message)"
        }
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
} -WantGui ($Gui.IsPresent -or $env:KIMUN_GUI -eq "1" -or $env:KIMUN_GUI -eq "true") -WantUninstall ($Uninstall.IsPresent -or $env:KIMUN_UNINSTALL -eq "1" -or $env:KIMUN_UNINSTALL -eq "true")
} finally {
    # Not left behind in the caller's session (the `irm | iex` form runs in it).
    Remove-Item Env:KIMUN_GUI -ErrorAction SilentlyContinue
    Remove-Item Env:KIMUN_UNINSTALL -ErrorAction SilentlyContinue
}
