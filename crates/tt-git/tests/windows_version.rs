//! The version resource that fills in the Details tab of tt-git.exe's Properties.
#![cfg(windows)]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");

#[test]
fn exe_carries_the_cargo_version_and_names() {
    let script = "$v = (Get-Item -LiteralPath $env:EXE).VersionInfo; \
        $v.FileVersion; $v.ProductVersion; $v.FileMajorPart; $v.FileMinorPart; \
        $v.FileBuildPart; $v.FilePrivatePart; $v.ProductName; $v.FileDescription; \
        $v.CompanyName; $v.OriginalFilename; $v.InternalName; $v.LegalCopyright";
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env_remove("PSModulePath") // Let Windows PowerShell find its own built-in modules.
        .env("EXE", BIN)
        .output()
        .expect("run powershell");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        lines,
        [
            version,
            version,
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
            "0",
            "tt-git",
            "tt-git terminal app",
            "ThirtyTech",
            "tt-git.exe",
            "tt-git",
            "Copyright (c) ThirtyTech. MIT License.",
        ]
    );
}
