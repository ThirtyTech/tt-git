#Requires -Version 7
<#
.SYNOPSIS
Seeds a worktree's Cargo target dir from the primary checkout's, hard-linking
the compiled third-party crates so they take no extra disk.

.DESCRIPTION
Run it from the primary checkout root after `git worktree add` and before the
worktree's first build. For every profile under -From (debug, release...):

- deps/: third-party .rlib and .rmeta files are hard-linked. These are most of
  the target's size. rustc deletes an output before writing it again, so a
  rebuild in either checkout breaks the link instead of changing the other
  checkout's file.
- deps/ (other third-party files), .fingerprint/ and build/ are copied. Cargo
  rewrites files there in place, so they must not be shared.
- Workspace crates and incremental/ are skipped. They rebuild in the worktree
  anyway, because its sources are newer than the primary's outputs.

Hard links can't cross volumes, so -From and -To must be on the same drive.

.EXAMPLE
./scripts/seed-worktree-target.ps1 -Worktree .worktrees/graph-colors
Seeds .worktrees/graph-colors/target/agent from target/agent.

.EXAMPLE
./scripts/seed-worktree-target.ps1 -Worktree .worktrees/graph-colors -Dir target
Seeds the user's own target/debug and friends, like for like.
#>
param(
    # The worktree to seed.
    [Parameter(Mandatory)] [string] $Worktree,
    # The target dir, relative to each checkout: target/agent for agents, target for the user's builds.
    [string] $Dir = 'target/agent'
)

$ErrorActionPreference = 'Stop'

$from = Join-Path (Get-Location) $Dir
$to = Join-Path (Resolve-Path $Worktree) $Dir
if (-not (Test-Path $from)) { throw "Nothing to seed from: $from doesn't exist." }
if ((Test-Path $to) -and (Get-ChildItem $to -Force | Select-Object -First 1)) {
    throw "$to already has files. Seed only a fresh worktree."
}
if ([IO.Path]::GetPathRoot($from) -ne [IO.Path]::GetPathRoot($to)) {
    throw "Hard links can't cross volumes: $from and $to are on different drives."
}

# Every workspace target name, as it appears in file names (dashes become underscores).
$meta = cargo metadata --no-deps --offline --format-version 1 --manifest-path (Join-Path $Worktree Cargo.toml) | ConvertFrom-Json
$names = $meta.packages | ForEach-Object { $_.name; $_.targets.name } | ForEach-Object { $_ -replace '-', '_' } | Sort-Object -Unique
$workspace = '^(lib)?(' + (($names | ForEach-Object { [regex]::Escape($_) }) -join '|') + ')[-_][0-9a-f]{16}'

$linked = 0L; $copied = 0L
$profiles = Get-ChildItem $from -Directory | Where-Object { Test-Path (Join-Path $_.FullName deps) }
foreach ($profile in $profiles) {
    $src = $profile.FullName
    $dst = Join-Path $to $profile.Name

    New-Item -ItemType Directory -Force (Join-Path $dst deps) | Out-Null
    foreach ($file in Get-ChildItem (Join-Path $src deps) -File) {
        # Fingerprint and build dirs use the package name with dashes; deps files use underscores.
        if (($file.Name -replace '-', '_') -match $workspace) { continue }
        $target = Join-Path $dst deps $file.Name
        if ($file.Extension -in '.rlib', '.rmeta') {
            New-Item -ItemType HardLink -Path $target -Target $file.FullName | Out-Null
            $linked += $file.Length
        } else {
            Copy-Item $file.FullName $target
            $copied += $file.Length
        }
    }

    foreach ($sub in '.fingerprint', 'build') {
        if (-not (Test-Path (Join-Path $src $sub))) { continue }
        New-Item -ItemType Directory -Force (Join-Path $dst $sub) | Out-Null
        foreach ($crate in Get-ChildItem (Join-Path $src $sub) -Directory) {
            if (($crate.Name -replace '-', '_') -match $workspace) { continue }
            Copy-Item -Recurse $crate.FullName (Join-Path $dst $sub $crate.Name)
            $copied += (Get-ChildItem $crate.FullName -Recurse -File | Measure-Object Length -Sum).Sum
        }
    }
}

'Seeded {0} from {1}: {2:N1} GB hard-linked, {3:N1} GB copied ({4}).' -f `
    $to, $from, ($linked / 1GB), ($copied / 1GB), (($profiles.Name) -join ', ')
