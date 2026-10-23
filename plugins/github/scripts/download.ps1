# The plugin's build step on Windows: downloads the prebuilt tt-git-github.exe
# for this plugin version into bin\. Without a release for this version (a
# development checkout), it builds it with Go instead.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$version = (Select-String -Path tt-git-plugin.toml -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'amd64' }
    'ARM64' { 'arm64' }
    default { Write-Error "github: unsupported CPU $env:PROCESSOR_ARCHITECTURE" }
}
$url = "https://github.com/ThirtyTech/tt-git/releases/download/github-v$version/tt-git-github-windows-$arch.exe"

New-Item -ItemType Directory -Force bin | Out-Null
Write-Host "Downloading $url"
try {
    Invoke-WebRequest -Uri $url -OutFile bin\tt-git-github.part -UseBasicParsing
    Move-Item -Force bin\tt-git-github.part bin\tt-git-github.exe
} catch {
    Remove-Item -ErrorAction Ignore bin\tt-git-github.part
    if (-not (Get-Command go -ErrorAction Ignore)) {
        Write-Error "github: couldn't download $url, and Go isn't installed to build it"
    }
    Write-Host "No release for github-v$version; building with Go"
    go build -o bin\tt-git-github.exe .
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
