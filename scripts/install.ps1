param(
    [string]$Version = $env:AYAME_VERSION,
    [string]$InstallDir = $env:AYAME_INSTALL_DIR,
    [switch]$NoShortcut
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Repo = "hjosugi/ayame-editor"
$BaseUrl = "https://github.com/$Repo"

function Say([string]$Message) {
    Write-Host $Message
}

function Fail([string]$Message) {
    throw "ayame install: $Message"
}

function Resolve-Version {
    if ([string]::IsNullOrWhiteSpace($script:Version)) {
        $script:Version = "latest"
    }

    if ($script:Version -eq "latest") {
        $release = Invoke-RestMethod `
            -Headers @{ "User-Agent" = "ayame-install" } `
            -Uri "https://api.github.com/repos/$Repo/releases/latest"
        $tag = [string]$release.tag_name
        if (-not $tag.StartsWith("v")) {
            Fail "could not resolve latest release tag: $tag"
        }
        $script:Version = $tag.Substring(1)
        return
    }

    $script:Version = $script:Version.TrimStart("v")
}

function Default-InstallDir {
    if (-not [string]::IsNullOrWhiteSpace($script:InstallDir)) {
        return
    }
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Fail "LOCALAPPDATA is not set"
    }
    $script:InstallDir = Join-Path $env:LOCALAPPDATA "Programs\Ayame"
}

function Test-Platform {
    if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows
    )) {
        Fail "this installer is for Windows; use scripts/install.sh on macOS/Linux"
    }

    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    if ($arch -ne [System.Runtime.InteropServices.Architecture]::X64) {
        Fail "unsupported Windows architecture: $arch"
    }
}

function Get-TempDir {
    $path = Join-Path ([System.IO.Path]::GetTempPath()) "ayame-install-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force -Path $path | Out-Null
    return $path
}

function Download-Release([string]$TempDir) {
    $asset = "ayame-v$Version-windows-x86_64.exe"
    $url = "$BaseUrl/releases/download/v$Version/$asset"
    $sumUrl = "$url.sha256"
    $assetPath = Join-Path $TempDir $asset
    $sumPath = "$assetPath.sha256"

    Say "download: $url"
    Invoke-WebRequest -Uri $url -OutFile $assetPath
    Invoke-WebRequest -Uri $sumUrl -OutFile $sumPath

    return @{
        Asset = $asset
        AssetPath = $assetPath
        SumPath = $sumPath
    }
}

function Verify-Sha256([string]$AssetPath, [string]$SumPath) {
    Say "verify: $(Split-Path -Leaf $SumPath)"
    $expected = ((Get-Content -Raw $SumPath) -split "\s+")[0].ToLowerInvariant()
    if ($expected -notmatch "^[0-9a-f]{64}$") {
        Fail "invalid sha256 file: $SumPath"
    }

    $actual = (Get-FileHash -Algorithm SHA256 $AssetPath).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        Fail "sha256 mismatch for $(Split-Path -Leaf $AssetPath)"
    }
}

function Add-UserPath([string]$PathToAdd) {
    $normalized = $PathToAdd.TrimEnd("\")
    $old = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @()
    if (-not [string]::IsNullOrWhiteSpace($old)) {
        $parts = @($old -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    }

    $already = $parts | Where-Object { $_.TrimEnd("\") -ieq $normalized } | Select-Object -First 1
    if (-not $already) {
        $new = if ($parts.Count -gt 0) {
            ($parts + $PathToAdd) -join ";"
        } else {
            $PathToAdd
        }
        [Environment]::SetEnvironmentVariable("Path", $new, "User")
        Say "PATH updated for new terminals."
    }

    $sessionParts = @($env:Path -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $sessionAlready = $sessionParts | Where-Object { $_.TrimEnd("\") -ieq $normalized } | Select-Object -First 1
    if (-not $sessionAlready) {
        $env:Path = "$PathToAdd;$env:Path"
    }
}

function Set-Shortcut([string]$ShortcutPath, [string]$TargetPath) {
    $parent = Split-Path -Parent $ShortcutPath
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($ShortcutPath)
    $shortcut.TargetPath = $TargetPath
    $shortcut.WorkingDirectory = Split-Path -Parent $TargetPath
    $shortcut.IconLocation = "$TargetPath,0"
    $shortcut.Save()
}

function Install-Artifact([string]$AssetPath) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $dest = Join-Path $InstallDir "ayame.exe"
    Copy-Item -Force $AssetPath $dest
    Unblock-File -Path $dest -ErrorAction SilentlyContinue
    Add-UserPath $InstallDir

    if (-not $NoShortcut) {
        $desktop = [Environment]::GetFolderPath("Desktop")
        if (-not [string]::IsNullOrWhiteSpace($desktop)) {
            Set-Shortcut (Join-Path $desktop "Ayame.lnk") $dest
        }

        $programs = [Environment]::GetFolderPath("Programs")
        if (-not [string]::IsNullOrWhiteSpace($programs)) {
            Set-Shortcut (Join-Path $programs "Ayame.lnk") $dest
        }
    }

    Say "installed: $dest"
    if (-not $NoShortcut) {
        Say "shortcuts updated: Desktop and Start Menu"
    }
    & $dest --version
}

Test-Platform
Resolve-Version
Default-InstallDir

$tmp = Get-TempDir
try {
    $download = Download-Release $tmp
    Verify-Sha256 $download.AssetPath $download.SumPath
    Install-Artifact $download.AssetPath
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
