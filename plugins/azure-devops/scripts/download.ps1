# The plugin's build step on Windows: downloads the prebuilt tt-git-ado.exe
# for this plugin version into bin\. Without a release for this version (a
# development checkout), it builds it with Go instead.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$version = (Select-String -Path tt-git-plugin.toml -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'amd64' }
    'ARM64' { 'arm64' }
    default { Write-Error "azure-devops: unsupported CPU $env:PROCESSOR_ARCHITECTURE" }
}
$url = "https://github.com/ThirtyTech/tt-git/releases/download/azure-devops-v$version/tt-git-ado-windows-$arch.exe"

New-Item -ItemType Directory -Force bin | Out-Null
Write-Host "Downloading $url"
try {
    Invoke-WebRequest -Uri $url -OutFile bin\tt-git-ado.part -UseBasicParsing
    Move-Item -Force bin\tt-git-ado.part bin\tt-git-ado.exe
} catch {
    Remove-Item -ErrorAction Ignore bin\tt-git-ado.part
    if (-not (Get-Command go -ErrorAction Ignore)) {
        Write-Error "azure-devops: couldn't download $url, and Go isn't installed to build it"
    }
    Write-Host "No release for azure-devops-v$version; building with Go"
    go build -o bin\tt-git-ado.exe .
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
