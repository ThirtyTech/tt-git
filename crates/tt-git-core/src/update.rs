//! Updating tt-git itself from its GitHub releases.
//!
//! Rather than reimplement the download, this runs the release's own install
//! script (`install.sh`, or `install.ps1` on Windows), which checks the
//! archive against the release's checksums and swaps the binary in a way that
//! is safe while it's running. Both `curl` and the script's shell ship with
//! every supported OS, so no HTTP client is needed.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// The GitHub repository releases come from.
pub const REPO: &str = "ThirtyTech/tt-git";

/// The version of this build, such as `0.3.0`.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("can't find the running tt-git: {0}")]
    NoExe(std::io::Error),
    #[error("{} isn't named tt-git, so the install script wouldn't replace it", .0.display())]
    NotTtGit(PathBuf),
    #[error(
        "tt-git is running from a cargo build directory ({}); build it again instead of updating",
        .0.display()
    )]
    CargoBuild(PathBuf),
    #[error("couldn't run {program}: {source}")]
    Spawn {
        program: &'static str,
        source: std::io::Error,
    },
    #[error("couldn't find the latest release: {0}")]
    Latest(String),
    #[error("couldn't download the install script for {tag}: {detail}")]
    Download { tag: String, detail: String },
    #[error("the install script failed{}", exit_detail(*.code))]
    Install { code: Option<i32> },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

fn exit_detail(code: Option<i32>) -> String {
    code.map(|c| format!(" (exit {c})")).unwrap_or_default()
}

/// The folder to install into: the running binary's own, so the copy that
/// is actually run gets replaced. Refuses a binary the script wouldn't
/// replace (another name) and a cargo build directory, so `cargo run` and
/// tests never overwrite a build.
pub fn install_dir(exe: &Path) -> Result<PathBuf, UpdateError> {
    let named_tt_git = exe.file_stem().is_some_and(|s| s == "tt-git");
    let dir = exe.parent().unwrap_or(Path::new("."));
    if !named_tt_git {
        return Err(UpdateError::NotTtGit(exe.to_path_buf()));
    }
    // Every cargo profile directory (target/debug, target/release) has one.
    if dir.join(".fingerprint").is_dir() {
        return Err(UpdateError::CargoBuild(dir.to_path_buf()));
    }
    Ok(dir.to_path_buf())
}

/// [`install_dir`] for the running binary.
pub fn current_install_dir() -> Result<PathBuf, UpdateError> {
    install_dir(&std::env::current_exe().map_err(UpdateError::NoExe)?)
}

/// The tag of the latest release, such as `v0.4.0`. GitHub redirects
/// `/releases/latest` to `/releases/tag/<tag>`; the install script finds it
/// the same way.
pub fn latest_release() -> Result<String, UpdateError> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSLI", "-o", null, "-w", "%{url_effective}", &url]);
    let out = output(cmd, "curl")?;
    if !out.status.success() {
        return Err(UpdateError::Latest(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let effective = String::from_utf8_lossy(&out.stdout);
    tag_from_url(&effective).ok_or_else(|| UpdateError::Latest(format!("unexpected address {effective}")))
}

fn tag_from_url(url: &str) -> Option<String> {
    let (_, tag) = url.trim().trim_end_matches('/').rsplit_once("/tag/")?;
    (!tag.is_empty() && !tag.contains('/')).then(|| tag.to_string())
}

/// A version as a release tag: `0.2.0` becomes `v0.2.0`.
pub fn normalize_tag(version: &str) -> String {
    let v = version.trim();
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

/// Whether release `tag` is newer than version `current`. Versions compare
/// by their numbers; a pre-release (`0.4.0-rc.1`) comes before its release.
/// A tag that doesn't parse is never newer.
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(new), Some(cur)) => new > cur,
        _ => false,
    }
}

/// `(major, minor, patch, is_release)`, so a release sorts after its
/// pre-releases.
fn parse_version(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (nums, pre) = match v.split_once('-') {
        Some((nums, _)) => (nums, true),
        None => (v, false),
    };
    let mut parts = nums.split('.').map(|p| p.parse::<u64>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, !pre))
}

/// Where release `tag`'s install script for this OS is.
pub fn script_url(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{}", script_name())
}

fn script_name() -> &'static str {
    if cfg!(windows) { "install.ps1" } else { "install.sh" }
}

/// What to do with the install script's output.
pub enum Output<'a> {
    /// Shown as it runs (the command line).
    Inherit,
    /// Collected and returned, and each line of its standard output passed
    /// here as it's printed (the UI, which owns the terminal).
    Lines(&'a mut dyn FnMut(&str)),
}

/// Downloads release `tag`'s install script and runs it to install that
/// release into `dir`. Returns what the script printed when `output` is
/// [`Output::Lines`]. A failed script's output is in the error.
pub fn install(tag: &str, dir: &Path, output: Output) -> Result<String, CapturedError> {
    let tmp = std::env::temp_dir().join(format!("tt-git-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| CapturedError::new(e.into()))?;
    let result = download_and_run(tag, dir, &tmp, output);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// An [`UpdateError`] with what the script printed before it failed.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct CapturedError {
    pub error: UpdateError,
    pub output: String,
}

impl CapturedError {
    fn new(error: UpdateError) -> Self {
        CapturedError {
            error,
            output: String::new(),
        }
    }
}

fn download_and_run(tag: &str, dir: &Path, tmp: &Path, output: Output) -> Result<String, CapturedError> {
    let script = tmp.join(script_name());
    let mut curl = Command::new("curl");
    curl.arg("-fsSL").arg("-o").arg(&script).arg(script_url(tag));
    let out = self::output(curl, "curl").map_err(CapturedError::new)?;
    if !out.status.success() {
        return Err(CapturedError::new(UpdateError::Download {
            tag: tag.to_string(),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }));
    }

    let mut cmd = installer_command(&script, tag, dir);
    let (status, text) = match output {
        Output::Inherit => {
            let status = cmd.status().map_err(|source| {
                CapturedError::new(UpdateError::Spawn {
                    program: shell(),
                    source,
                })
            })?;
            (status, String::new())
        }
        Output::Lines(on_line) => {
            let (status, text) = run_by_line(cmd, on_line).map_err(|source| {
                CapturedError::new(UpdateError::Spawn {
                    program: shell(),
                    source,
                })
            })?;
            (status, text.trim().to_string())
        }
    };
    if status.success() {
        Ok(text)
    } else {
        Err(CapturedError {
            error: UpdateError::Install { code: status.code() },
            output: text,
        })
    }
}

fn shell() -> &'static str {
    if cfg!(windows) { "powershell" } else { "sh" }
}

/// The command that runs the downloaded install `script` for release `tag`
/// into `dir`.
fn installer_command(script: &Path, tag: &str, dir: &Path) -> Command {
    let mut cmd = Command::new(shell());
    if cfg!(windows) {
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]);
        // Started from PowerShell 7, this holds its module paths, and Windows
        // PowerShell can then fail to load its own built-in modules.
        cmd.env_remove("PSModulePath");
    }
    cmd.arg(script)
        .env("TTGIT_VERSION", tag)
        .env("TTGIT_INSTALL_DIR", dir)
        .stdin(Stdio::null());
    cmd
}

/// Runs `cmd` with no console window, passing each line of its standard
/// output to `on_line` as it comes. Returns its exit status and everything
/// it printed, standard error last.
fn run_by_line(mut cmd: Command, on_line: &mut dyn FnMut(&str)) -> std::io::Result<(ExitStatus, String)> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::git::hide_console_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let mut stderr = child.stderr.take().expect("piped");
    // Read alongside stdout, so a full stderr pipe can't stall the script.
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut text = String::new();
    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        on_line(line.trim_end());
        text.push_str(&line);
        text.push('\n');
    }
    let status = child.wait()?;
    text.push_str(&errors.join().unwrap_or_default());
    Ok((status, text))
}

/// Runs `cmd` to completion with its output collected and no console window.
fn output(mut cmd: Command, program: &'static str) -> Result<std::process::Output, UpdateError> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::git::hide_console_window(&mut cmd);
    cmd.output().map_err(|source| UpdateError::Spawn { program, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numbers_and_pre_releases() {
        assert!(is_newer("v0.4.0", "0.3.0"));
        assert!(is_newer("v0.10.0", "0.9.1"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.9", "0.3.0"));
        assert!(is_newer("v0.4.0", "0.4.0-rc.1"), "a release is newer than its rc");
        assert!(!is_newer("v0.4.0-rc.1", "0.4.0"));
        assert!(is_newer("v0.4.0-rc.1", "0.3.0"));
        assert!(!is_newer("latest", "0.3.0"), "unparsable is never newer");
        assert!(!is_newer("v1.2", "0.3.0"));
        assert!(!is_newer("v1.2.3.4", "0.3.0"));
    }

    #[test]
    fn tags_are_normalized() {
        assert_eq!(normalize_tag("0.2.0"), "v0.2.0");
        assert_eq!(normalize_tag(" v0.2.0 "), "v0.2.0");
    }

    #[test]
    fn the_latest_tag_is_the_end_of_the_redirect() {
        let url = "https://github.com/ThirtyTech/tt-git/releases/tag/v0.4.0";
        assert_eq!(tag_from_url(url).as_deref(), Some("v0.4.0"));
        assert_eq!(tag_from_url(&format!("{url}/\n")).as_deref(), Some("v0.4.0"));
        // No release yet: GitHub leaves you on /releases.
        assert_eq!(tag_from_url("https://github.com/ThirtyTech/tt-git/releases"), None);
    }

    #[test]
    fn the_script_comes_from_the_release() {
        let url = script_url("v0.4.0");
        assert!(url.starts_with("https://github.com/ThirtyTech/tt-git/releases/download/v0.4.0/install."));
    }

    #[test]
    fn installs_next_to_the_running_binary() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(if cfg!(windows) { "tt-git.exe" } else { "tt-git" });
        assert_eq!(install_dir(&exe).unwrap(), dir.path());
    }

    #[test]
    fn refuses_a_cargo_build_and_other_names() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("git-tui.exe");
        assert!(matches!(install_dir(&other), Err(UpdateError::NotTtGit(_))));

        std::fs::create_dir(dir.path().join(".fingerprint")).unwrap();
        let exe = dir.path().join("tt-git");
        let err = install_dir(&exe).unwrap_err();
        assert!(matches!(err, UpdateError::CargoBuild(_)));
        assert!(err.to_string().contains("cargo build directory"), "{err}");
    }

    #[test]
    fn the_installer_gets_the_release_and_folder() {
        let cmd = installer_command(Path::new("install.sh"), "v0.4.0", Path::new("bin"));
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&("TTGIT_VERSION".as_ref(), Some("v0.4.0".as_ref()))));
        assert!(envs.contains(&("TTGIT_INSTALL_DIR".as_ref(), Some("bin".as_ref()))));
        assert_eq!(cmd.get_args().last(), Some("install.sh".as_ref()));
    }
}
