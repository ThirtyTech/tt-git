# Installs tt-git from its GitHub releases on Windows (x64, arm64):
#
#   powershell -ExecutionPolicy Bypass -c "irm https://github.com/ThirtyTech/tt-git/releases/latest/download/install.ps1 | iex"
#
# Environment:
#   TTGIT_VERSION      tag to install, such as v0.2.0 (default: latest release)
#   TTGIT_INSTALL_DIR  where to put the binary (default: %LOCALAPPDATA%\Programs\tt-git\bin)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is very slow with the progress bar on
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'ThirtyTech/tt-git'
$installDir = if ($env:TTGIT_INSTALL_DIR) { $env:TTGIT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\tt-git\bin' }

function Fail($msg) {
    Write-Host "tt-git install: $msg" -ForegroundColor Red
    throw $msg
}

# The OS architecture, not the process's: x64 PowerShell on an ARM machine reports AMD64.
$osArch = try {
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
} catch {
    if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
}
$arch = switch ($osArch) {
    { $_ -in 'X64', 'AMD64' } { 'x64' }
    { $_ -in 'Arm64', 'ARM64' } { 'arm64' }
    default { Fail "unsupported architecture: $osArch" }
}

if ($env:TTGIT_VERSION) {
    $version = $env:TTGIT_VERSION
} else {
    $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'tt-git-installer' }
    $version = $latest.tag_name
    if (-not $version) { Fail "couldn't find the latest release" }
}

$name = "tt-git-$version-windows-$arch"
$base = "https://github.com/$repo/releases/download/$version"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null

try {
    Write-Host "Downloading tt-git $version (windows-$arch)"
    $zip = Join-Path $tmp "$name.zip"
    try {
        Invoke-WebRequest "$base/$name.zip" -OutFile $zip -UseBasicParsing
        Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile (Join-Path $tmp 'SHA256SUMS.txt') -UseBasicParsing
    } catch {
        Fail "download failed: $($_.Exception.Message)"
    }

    $line = Get-Content (Join-Path $tmp 'SHA256SUMS.txt') | Where-Object { $_ -match " $([regex]::Escape("$name.zip"))$" }
    if (-not $line) { Fail "no checksum for $name.zip" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash
    if ($expected -ne $actual) { Fail "checksum mismatch for $name.zip" }

    Expand-Archive $zip -DestinationPath $tmp
    New-Item -ItemType Directory -Force $installDir | Out-Null

    # A running exe can't be overwritten but can be renamed, so move the old
    # one aside first. It's removed on the next install once it has exited.
    $exe = Join-Path $installDir 'tt-git.exe'
    $old = Join-Path $installDir 'tt-git.exe.old'
    Remove-Item $old -Force -ErrorAction SilentlyContinue
    if (Test-Path $exe) { Move-Item $exe $old -Force }
    Copy-Item (Join-Path $tmp "$name\tt-git.exe") $exe

    Write-Host "Installed tt-git $version to $exe"
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$entries = if ($userPath) { $userPath -split ';' } else { @() }
if ($entries -notcontains $installDir) {
    $newPath = (@($entries | Where-Object { $_ }) + $installDir) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$env:Path;$installDir"
    Write-Host "Added $installDir to your PATH. Restart your terminal to use tt-git."
}
