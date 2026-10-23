//! The Windows release installer against local archives, without network or PATH changes.
#![cfg(windows)]

use std::path::Path;
use std::process::{Child, Command, Stdio};

use tt_git_testkit::Sandbox;

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");
const INSTALLED: &[u8] = b"fake new release";

struct Running(Child);

impl Running {
    fn start(sb: &Sandbox, exe: &Path) -> Self {
        let mut cmd = Command::new(exe);
        cmd.args(["__test-plugin", "sleep", "600000"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        sb.apply_env(&mut cmd);
        Self(cmd.spawn().expect("start a running installed copy"))
    }

    fn assert_alive(&mut self) {
        assert!(self.0.try_wait().unwrap().is_none(), "installer stopped a running copy");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn install(sb: &Sandbox) {
    let script = sb.root().join("run-installer.ps1");
    // Exercise the real download/checksum/extract/install flow. The final PATH
    // registration is deliberately excluded so tests never edit the registry.
    let installer = include_str!("../../../install.ps1")
        .split_once("$userPath =")
        .expect("installer PATH registration marker")
        .0;
    let fixture = r#"
$ErrorActionPreference = 'Stop'
$release = Join-Path $env:FIXTURE_ROOT 'release'
if (-not (Test-Path $release)) {
    New-Item -ItemType Directory $release | Out-Null
    foreach ($arch in @('x64', 'arm64')) {
        $name = "tt-git-v99.0.0-windows-$arch"
        $dir = Join-Path $release $name
        New-Item -ItemType Directory $dir | Out-Null
        [IO.File]::WriteAllText((Join-Path $dir 'tt-git.exe'), 'fake new release')
        $zip = Join-Path $release "$name.zip"
        Compress-Archive -Path $dir -DestinationPath $zip
        $hash = (Get-FileHash $zip -Algorithm SHA256).Hash
        Add-Content (Join-Path $release 'SHA256SUMS.txt') "$hash  $name.zip"
    }
}
function Invoke-WebRequest($Uri, $OutFile, [switch]$UseBasicParsing) {
    Copy-Item -LiteralPath (Join-Path $release (Split-Path $Uri -Leaf)) -Destination $OutFile
}
"#;
    std::fs::write(&script, format!("{fixture}\n{installer}")).unwrap();
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .env_remove("PSModulePath") // Let Windows PowerShell find its own built-in modules.
        .env("FIXTURE_ROOT", sb.root())
        .env("TTGIT_VERSION", "v99.0.0")
        .env("TTGIT_INSTALL_DIR", sb.root().join("installed"));
    sb.apply_env(&mut cmd);
    let out = cmd.output().expect("run Windows installer");
    assert!(
        out.status.success(),
        "installer failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(sb.root().join("installed/tt-git.exe")).unwrap(),
        INSTALLED
    );
}

#[test]
fn updates_with_running_current_and_backup_copies_then_cleans_up() {
    let sb = Sandbox::without_repo();
    install(&sb); // A fresh install still works.
    let dir = sb.root().join("installed");
    let exe = dir.join("tt-git.exe");
    let old = dir.join("tt-git.exe.old");
    std::fs::copy(BIN, &exe).unwrap();
    std::fs::copy(BIN, &old).unwrap();
    std::fs::write(dir.join("tt-git.exe.old-stale"), b"unused backup").unwrap();
    let mut current = Running::start(&sb, &exe);
    let mut previous = Running::start(&sb, &old);

    for _ in 0..2 {
        install(&sb);
        current.assert_alive();
        previous.assert_alive();
        assert!(old.exists(), "the running previous backup must survive");
        assert!(!dir.join("tt-git.exe.old-stale").exists(), "unused backups are removed");
    }

    drop(current);
    drop(previous);
    install(&sb);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "only the new binary and latest backup remain: {names:?}"
    );
    assert_eq!(std::fs::read(old).unwrap(), INSTALLED);
}
