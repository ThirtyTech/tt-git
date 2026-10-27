# Packages tt-git-gui for Windows:
#
#   package.ps1 <bin-dir> <tag> <platform>
#
# <bin-dir> holds release builds of tt-git-gui.exe and tt-git.exe, <tag> is
# the release tag (v0.9.0) and <platform> the release's name for the target
# (windows-x64). Writes, in the current directory:
#
#   tt-git-gui-<tag>-<platform>.zip        tt-git-gui.exe and tt-git.exe in a
#                                          folder, to unpack anywhere
#   tt-git-gui-<tag>-<platform>-setup.exe  an installer, when Inno Setup's
#                                          ISCC.exe is installed
#
# In the zip, tt-git.exe sits next to tt-git-gui.exe; the installer puts it
# in bin beside it, the folder it adds to PATH. The GUI looks in both for the
# binary it gives plugins as TTGIT_BIN_PATH.
#
# Authenticode signing, from the environment (the release workflow passes
# its secrets):
#
#   WINDOWS_CERT_PFX       base64 of a code signing certificate and key (.pfx)
#   WINDOWS_CERT_PASSWORD  the .pfx's password
#
# Without WINDOWS_CERT_PFX nothing is signed, and SmartScreen warns on the
# first run.

param(
    [Parameter(Mandatory)] [string] $BinDir,
    [Parameter(Mandatory)] [string] $Tag,
    [Parameter(Mandatory)] [string] $Platform
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$root = (Resolve-Path (Join-Path $here '..\..\..\..')).Path
$name = "tt-git-gui-$Tag-$Platform"
$version = $Tag -replace '^v', ''
$out = (Get-Location).Path

foreach ($exe in 'tt-git-gui.exe', 'tt-git.exe') {
    if (-not (Test-Path -LiteralPath (Join-Path $BinDir $exe))) { throw "package.ps1: no $exe in $BinDir" }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
$dir = Join-Path $stage $name
New-Item -ItemType Directory $dir | Out-Null

try {
    Copy-Item (Join-Path $BinDir 'tt-git-gui.exe'), (Join-Path $BinDir 'tt-git.exe') $dir
    Copy-Item (Join-Path $root 'README.md') $dir

    # Signing, when there's a certificate.
    $sign = $null
    if ($env:WINDOWS_CERT_PFX) {
        $pfx = Join-Path $stage 'cert.pfx'
        [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX))
        $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
            Sort-Object FullName | Select-Object -Last 1
        if (-not $signtool) { throw 'package.ps1: signtool.exe not found' }
        $sign = {
            param([string[]] $files)
            & $signtool.FullName sign /f $pfx /p $env:WINDOWS_CERT_PASSWORD /fd SHA256 `
                /tr http://timestamp.digicert.com /td SHA256 /d tt-git @files
            if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
        }
        & $sign -files (Join-Path $dir 'tt-git-gui.exe'), (Join-Path $dir 'tt-git.exe')
    } else {
        Write-Host 'No WINDOWS_CERT_PFX: not signing'
    }

    $zip = Join-Path $out "$name.zip"
    Remove-Item -LiteralPath $zip -ErrorAction Ignore
    # 7-Zip, as the TUI's zip is made; it's on GitHub's Windows runners.
    & 7z a $zip $dir | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "7z failed ($LASTEXITCODE)" }
    Write-Host "Wrote $name.zip"

    # The installer.
    $iscc = if ($env:ISCC) { $env:ISCC } else { "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
    if (Test-Path -LiteralPath $iscc) {
        $arch = if ($Platform -like '*arm64') { 'arm64' } else { 'x64compatible' }
        # Windows file versions are numbers only: 0.9.0-rc.1 becomes 0.9.0.
        $fileVersion = $version -replace '-.*$', ''
        & $iscc /Q "/DAppVersion=$version" "/DFileVersion=$fileVersion" "/DStageDir=$dir" "/DOutputBase=$name-setup" `
            "/DArch=$arch" "/O$out" (Join-Path $here 'tt-git.iss')
        if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
        $setup = Join-Path $out "$name-setup.exe"
        if ($sign) { & $sign -files $setup }
        Write-Host "Wrote $name-setup.exe"
    } else {
        Write-Host "No Inno Setup at ${iscc}: skipping the installer"
    }
} finally {
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
