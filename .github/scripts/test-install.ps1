# Self-test for install.ps1 against a fake release served from a local directory
# (file:// URLs, a pinned version): the terminal app, the desktop app and its
# Start-menu shortcut, the one shared folder and marker, the user PATH, a bad
# checksum, a missing desktop build, replacing an exe that is open, and the three
# ways the script is invoked (file, script block, `irm | iex`).
#
# It touches what the installer touches on the machine it runs on: the user PATH in
# the registry and the Start-menu shortcut. So it runs only on CI (or with
# KIMUN_TEST_ALLOW=1) and puts both back when it is done.
#
# Run, once per shell the installer must work in:
#   powershell -NoProfile -ExecutionPolicy Bypass -File .github/scripts/test-install.ps1
#   pwsh -NoProfile -File .github/scripts/test-install.ps1
$ErrorActionPreference = "Stop"

if (-not ($env:CI -or $env:KIMUN_TEST_ALLOW -eq "1")) {
    Write-Host "skipped: this test edits the user PATH and Start menu; set KIMUN_TEST_ALLOW=1 to run it here"
    exit 0
}
if ($env:OS -ne "Windows_NT") { Write-Host "skipped: not Windows"; exit 0 }

$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script = Join-Path $root "install.ps1"
$v = "9.9.9"
$work = Join-Path ([IO.Path]::GetTempPath()) "kimun-test-install-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$me = (Get-Process -Id $PID).Path   # the shell running this test: the installer runs in the same one

function Fail($why) { throw "FAIL: $why" }
function Ok($what) { Write-Host "ok: $what" }

function Sha256($path) {
    $s = [IO.File]::OpenRead($path)
    try { -join ([Security.Cryptography.SHA256]::Create().ComputeHash($s) | ForEach-Object { $_.ToString("x2") }) } finally { $s.Close() }
}

# A release: both apps' Windows archives (the exe at the archive's root, as
# release-dist.sh packs them) and the checksums file, in sha256sum's format.
function New-Release($name, [switch]$NoGui) {
    $out = Join-Path $work $name
    New-Item -ItemType Directory -Force $out | Out-Null
    foreach ($app in @("kimun", "kimun-gui")) {
        if ($NoGui -and $app -eq "kimun-gui") { continue }
        $stage = Join-Path $work "stage-$name-$app"
        New-Item -ItemType Directory -Force $stage | Out-Null
        [IO.File]::WriteAllText((Join-Path $stage "$app.exe"), "$app $name")
        Compress-Archive -Path (Join-Path $stage "*") -DestinationPath (Join-Path $out "$app-$v-windows-x64.zip")
    }
    $lines = Get-ChildItem $out -Filter "*.zip" | ForEach-Object { "$(Sha256 $_.FullName)  $($_.Name)" }
    [IO.File]::WriteAllLines((Join-Path $out "checksums-sha256.txt"), $lines)
    return $out
}

# Run the installer in a child shell, in a fresh home, as the user would. $How picks
# the invocation. Returns the exit code and what it printed.
function Invoke-Install($home_, $rel, $How, [string[]]$Env = @()) {
    New-Item -ItemType Directory -Force $home_ | Out-Null
    $env:USERPROFILE = $home_
    $env:KIMUN_INSTALL_DIR = Join-Path $home_ "bin"
    $env:KIMUN_VERSION = $v
    $env:KIMUN_DOWNLOAD_BASE = "file:///" + ($rel -replace "\\", "/")
    Remove-Item Env:KIMUN_GUI -ErrorAction SilentlyContinue
    foreach ($e in $Env) { $k, $val = $e -split "=", 2; Set-Item "Env:$k" $val }
    $q = "'" + ($script -replace "'", "''") + "'"
    $args_ = switch ($How) {
        "file"        { @("-File", $script) }
        "file-gui"    { @("-File", $script, "-Gui") }
        "block-gui"   { @("-Command", "& ([scriptblock]::Create((Get-Content -Raw $q))) -Gui") }
        "iex"         { @("-Command", "Get-Content -Raw $q | Invoke-Expression") }
    }
    # Continue, not Stop: under Stop, Windows PowerShell 5.1 turns the child's stderr
    # (where a `throw` is printed) into a terminating error here.
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $me -NoProfile -ExecutionPolicy Bypass @args_ 2>&1 | Out-String
    $code = $LASTEXITCODE
    $ErrorActionPreference = $saved
    Remove-Item Env:KIMUN_GUI -ErrorAction SilentlyContinue
    return [pscustomobject]@{ Code = $code; Output = $output }
}

function Get-RawUserPath {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $false)
    try { [string]$key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } finally { $key.Close() }
}

$menuDir = [Environment]::GetFolderPath("Programs")
# The installer runs with a fake USERPROFILE, where .NET may not resolve this folder:
# it then falls back to %APPDATA%. The test looks in the same place.
if (-not $menuDir) { $menuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs" }
$shortcut = Join-Path $menuDir "Kim$([char]0xFC)n.lnk"
$pathBefore = Get-RawUserPath
$pathKindBefore = $null
$k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $false)
if ($k.GetValueNames() -contains "Path") { $pathKindBefore = $k.GetValueKind("Path") }
$k.Close()
$shortcutBefore = Test-Path $shortcut
$userprofile = $env:USERPROFILE

try {
    $rel = New-Release "rel"

    # 1. The terminal app: its exe and the marker, no desktop shortcut.
    $h = Join-Path $work "h1"
    $r = Invoke-Install $h $rel "file"
    if ($r.Code -ne 0) { Fail "terminal install exited $($r.Code):`n$($r.Output)" }
    $dir = Join-Path $h "bin"
    if (-not (Test-Path "$dir\kimun.exe")) { Fail "terminal exe not installed" }
    if (Test-Path "$dir\kimun-gui.exe") { Fail "the terminal install put kimun-gui.exe in place" }
    $marker = Join-Path $h "kimun\install.toml"
    $m = Get-Content -Raw $marker
    if ($m -notmatch 'channel = "script"') { Fail "marker channel: $m" }
    if ($m -notmatch "version = `"$v`"") { Fail "marker version: $m" }
    if ($m -notmatch [regex]::Escape("install_dir = '$dir'")) { Fail "marker install_dir: $m" }
    if (-not $shortcutBefore -and (Test-Path $shortcut)) { Fail "the terminal install made a Start-menu shortcut" }
    Ok "terminal app: exe + marker, no shortcut"

    # 2. The desktop app, by file switch: into the same folder, same marker, with a shortcut.
    $r = Invoke-Install $h $rel "file-gui"
    if ($r.Code -ne 0) { Fail "desktop install exited $($r.Code):`n$($r.Output)" }
    if (-not (Test-Path "$dir\kimun-gui.exe")) { Fail "desktop exe not installed" }
    if (-not (Test-Path "$dir\kimun.exe")) { Fail "the desktop install removed kimun.exe" }
    if (-not (Test-Path $shortcut)) { Fail "no Start-menu shortcut" }
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    if ($target -ne "$dir\kimun-gui.exe") { Fail "shortcut points at $target" }
    if ((Get-Content -Raw $marker) -notmatch [regex]::Escape("install_dir = '$dir'")) { Fail "marker lost its folder" }
    Ok "desktop app: same folder and marker, Start-menu shortcut to the exe"

    # 3. The folder is on the user PATH, once, and the value's kind is kept.
    $listed = @((Get-RawUserPath) -split ";" | Where-Object { $_.TrimEnd("\") -ieq $dir })
    if ($listed.Count -ne 1) { Fail "PATH lists the folder $($listed.Count) times" }
    if ($pathKindBefore -and ([Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment").GetValueKind("Path") -ne $pathKindBefore)) { Fail "PATH changed its value kind" }
    Ok "PATH: the folder once, value kind kept"

    # 4. The other invocations: a script block with -Gui, `irm | iex` bare, `irm | iex` with KIMUN_GUI=1.
    foreach ($case in @(
            @{ How = "block-gui"; Env = @(); Exe = "kimun-gui.exe"; Name = "script block -Gui" },
            @{ How = "iex"; Env = @(); Exe = "kimun.exe"; Name = "iex, terminal app" },
            @{ How = "iex"; Env = @("KIMUN_GUI=1"); Exe = "kimun-gui.exe"; Name = "iex with KIMUN_GUI=1" })) {
        $hc = Join-Path $work ("h4-" + ($case.Name -replace "\W", ""))
        $r = Invoke-Install $hc $rel $case.How $case.Env
        if ($r.Code -ne 0) { Fail "$($case.Name) exited $($r.Code):`n$($r.Output)" }
        $other = if ($case.Exe -eq "kimun.exe") { "kimun-gui.exe" } else { "kimun.exe" }
        if (-not (Test-Path (Join-Path $hc "bin\$($case.Exe)"))) { Fail "$($case.Name): $($case.Exe) not installed" }
        if (Test-Path (Join-Path $hc "bin\$other")) { Fail "$($case.Name): $other installed too" }
    }
    Ok "invoked as a script block, as irm | iex, and with KIMUN_GUI=1"

    # 5. A bad checksum: refused, nothing replaced.
    $bad = New-Release "bad"
    $sums = Join-Path $bad "checksums-sha256.txt"
    [IO.File]::WriteAllLines($sums, @(Get-Content $sums | ForEach-Object { "0" * 64 + "  " + ($_ -split "\s+", 2)[1] }))
    $h5 = Join-Path $work "h5"
    $r = Invoke-Install $h5 $bad "file"
    if ($r.Code -eq 0) { Fail "a bad checksum was accepted" }
    if ($r.Output -notmatch "checksum mismatch") { Fail "bad checksum said: $($r.Output)" }
    if (Test-Path (Join-Path $h5 "bin\kimun.exe")) { Fail "a bad checksum still installed the exe" }
    Ok "bad checksum: refused, nothing installed"

    # 6. A release with no desktop build: the message says so, not a bare 404.
    $nogui = New-Release "nogui" -NoGui
    $r = Invoke-Install (Join-Path $work "h6") $nogui "file-gui"
    if ($r.Code -eq 0) { Fail "a missing desktop build was accepted" }
    if ($r.Output -notmatch "no desktop app build") { Fail "missing desktop build said: $($r.Output)" }
    Ok "no desktop build: a plain message"

    # 7. Updating while the app is open: a running exe can be renamed, not deleted.
    $rel2 = New-Release "rel2"
    $exe = "$dir\kimun.exe"
    $open = New-Object IO.FileStream($exe, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::Read -bor [IO.FileShare]::Delete))
    try {
        $r = Invoke-Install $h $rel2 "file"
    } finally {
        $open.Close()
    }
    if ($r.Code -ne 0) { Fail "the update over an open exe exited $($r.Code):`n$($r.Output)" }
    if ((Get-Content -Raw $exe) -ne "kimun rel2") { Fail "the open exe was not replaced" }
    $r = Invoke-Install $h $rel2 "file"
    if ($r.Code -ne 0) { Fail "the next install exited $($r.Code):`n$($r.Output)" }
    if (Test-Path "$exe.old") { Fail "the old exe was not cleaned up by the next install" }
    Ok "update over an open exe: replaced, old one removed next time"
} finally {
    # Put back what was changed on this machine.
    $env:USERPROFILE = $userprofile
    foreach ($n in "KIMUN_INSTALL_DIR", "KIMUN_VERSION", "KIMUN_DOWNLOAD_BASE", "KIMUN_GUI") { Remove-Item "Env:$n" -ErrorAction SilentlyContinue }
    if (-not $shortcutBefore) { Remove-Item -Force $shortcut -ErrorAction SilentlyContinue }
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    try {
        if ($pathKindBefore) { $key.SetValue("Path", $pathBefore, $pathKindBefore) }
        else { $key.DeleteValue("Path", $false) }
    } finally { $key.Close() }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
Write-Host "all install.ps1 tests passed"
