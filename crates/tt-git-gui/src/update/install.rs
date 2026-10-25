//! Which of a release's downloads updates this copy of the app, and putting
//! it in place.
//!
//! The release workflow publishes the app as `tt-git-gui-<tag>-<platform>`
//! with one extension per way it's installed (see `release.yml` and
//! `packaging/`). [`detect`] works out which one the running copy came
//! from, [`prepare`] downloads it, checks it against the release's
//! `SHA256SUMS.txt` and swaps the new files in where it can do that while
//! the app runs. Only the Windows installer waits until every instance has
//! quit (it's run by the [waiter](super::instances::finish)).
//!
//! Nothing here looks at the running binary or goes online by itself: the
//! paths come from the caller and downloads go through [`Fetch`], so tests
//! work in a temporary folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

/// The OS a copy runs on, as [`detect`] needs it. A parameter rather than
/// `cfg!`, so every case is tested on every OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    pub const CURRENT: Os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    };
}

/// How this copy of the app was installed, which says what updates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// Unpacked from the Windows zip: `tt-git-gui.exe` and `tt-git.exe` in
    /// `dir`.
    WindowsFolder { dir: PathBuf },
    /// Installed by the Windows installer (Inno Setup), whose uninstaller is
    /// in `dir`. The new installer runs over it, so Settings > Apps shows
    /// the new version.
    WindowsInstaller { dir: PathBuf },
    /// `tt-git.app`, from the macOS .dmg.
    AppBundle { app: PathBuf },
    /// A Linux AppImage.
    AppImage { file: PathBuf },
    /// The Linux tarball unpacked into a prefix (`~/.local`, `/usr/local`):
    /// `bin` holds `tt-git-gui` and `tt-git`.
    Prefix { bin: PathBuf },
}

/// Why this copy can't update itself.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("This copy was built with cargo ({}); build it again instead.", .0.display())]
    CargoBuild(PathBuf),
    #[error("It was installed by your package manager ({}); update it there.", .0.display())]
    Package(PathBuf),
    #[error(
        "This copy ({}) isn't laid out as a release download installs it, so it can't replace itself.",
        .0.display()
    )]
    Unpackaged(PathBuf),
    #[error("There's no desktop app download for this platform.")]
    NoBuild,
    #[error("Can't find the running app: {0}")]
    NoExe(String),
}

/// The executable's file name.
fn exe_name(os: Os, stem: &str) -> String {
    match os {
        Os::Windows => format!("{stem}.exe"),
        _ => stem.to_string(),
    }
}

/// How the app at `exe` was installed. `appimage` is `$APPIMAGE`, which an
/// AppImage's runtime sets to the AppImage's own path.
pub fn detect(exe: &Path, os: Os, appimage: Option<&Path>) -> Result<Install, Refusal> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    // Every cargo profile directory (target/debug, target/release) has one.
    if dir.join(".fingerprint").is_dir() {
        return Err(Refusal::CargoBuild(dir.to_path_buf()));
    }
    match os {
        Os::Windows if dir.join("unins000.exe").is_file() => Ok(Install::WindowsInstaller { dir: dir.to_path_buf() }),
        Os::Windows => Ok(Install::WindowsFolder { dir: dir.to_path_buf() }),
        Os::MacOs => app_bundle(dir)
            .map(|app| Install::AppBundle { app: app.to_path_buf() })
            .ok_or_else(|| Refusal::Unpackaged(dir.to_path_buf())),
        Os::Linux => {
            if let Some(file) = appimage {
                return Ok(Install::AppImage {
                    file: file.to_path_buf(),
                });
            }
            // Where deb, rpm and Arch packages put it.
            if [Path::new("/usr/bin"), Path::new("/bin"), Path::new("/usr/sbin")].contains(&dir) {
                return Err(Refusal::Package(dir.to_path_buf()));
            }
            if dir.file_name().is_some_and(|n| n == "bin") {
                Ok(Install::Prefix { bin: dir.to_path_buf() })
            } else {
                Err(Refusal::Unpackaged(dir.to_path_buf()))
            }
        }
    }
}

/// The `.app` bundle `dir` is the executables folder of, if it is one.
fn app_bundle(dir: &Path) -> Option<&Path> {
    let contents = dir.ends_with("Contents/MacOS").then(|| dir.parent()?.parent())??;
    contents.extension().is_some_and(|e| e == "app").then_some(contents)
}

/// The release's name for this build's platform, as in its downloads
/// (`windows-x64`), or `None` where no desktop app is published. Built
/// for x64 and run on an Arm machine, it's still the x64 download.
pub fn platform() -> Option<&'static str> {
    let os = Os::CURRENT;
    let arm = cfg!(target_arch = "aarch64");
    let x64 = cfg!(target_arch = "x86_64");
    match (os, x64, arm) {
        (Os::Windows, true, _) => Some("windows-x64"),
        (Os::Linux, true, _) => Some("linux-x64"),
        (Os::Linux, _, true) => Some("linux-arm64"),
        (Os::MacOs, _, true) => Some("macos-arm64"),
        _ => None,
    }
}

impl Install {
    /// The release download that updates this install.
    pub fn asset(&self, tag: &str, platform: &str) -> String {
        let name = format!("tt-git-gui-{tag}-{platform}");
        match self {
            Install::WindowsFolder { .. } => format!("{name}.zip"),
            Install::WindowsInstaller { .. } => format!("{name}-setup.exe"),
            Install::AppBundle { .. } => format!("{name}.dmg"),
            Install::AppImage { .. } => format!("{name}.AppImage"),
            Install::Prefix { .. } => format!("{name}.tar.gz"),
        }
    }

    /// What to start to run this install again: the executable, or the
    /// bundle or AppImage that holds it.
    pub fn launch(&self, os: Os) -> PathBuf {
        match self {
            Install::WindowsFolder { dir } | Install::WindowsInstaller { dir } => dir.join(exe_name(os, "tt-git-gui")),
            Install::AppBundle { app } => app.clone(),
            Install::AppImage { file } => file.clone(),
            Install::Prefix { bin } => bin.join(exe_name(os, "tt-git-gui")),
        }
    }
}

/// Where release `tag`'s download `file` is.
pub fn download_url(tag: &str, file: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{tag}/{file}",
        tt_git_core::update::REPO
    )
}

/// The release page of `tag`, for downloading it by hand.
pub fn release_page(tag: &str) -> String {
    format!("https://github.com/{}/releases/tag/{tag}", tt_git_core::update::REPO)
}

/// Downloads a file. [`Curl`] in the app; tests copy local files.
pub trait Fetch: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), String>;
}

/// Downloads with `curl`, which every supported OS ships, as
/// `tt-git update` does.
pub struct Curl;

impl Fetch for Curl {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), String> {
        let mut cmd = Command::new("curl");
        cmd.arg("-fsSL").arg("-o").arg(dest).arg(url);
        let out = quiet_output(cmd).map_err(|e| format!("couldn't run curl: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "couldn't download {url}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

/// Runs `cmd` to completion with its output collected and no console window.
fn quiet_output(mut cmd: Command) -> std::io::Result<std::process::Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    tt_git_core::git::hide_console_window(&mut cmd);
    cmd.output()
}

/// Runs `cmd`, turning a failure into what it printed.
fn run(cmd: Command, what: &str) -> Result<(), String> {
    let out = quiet_output(cmd).map_err(|e| format!("couldn't run {what}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// What [`prepare`] left to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// The new files are in place; restarting runs them.
    Replaced,
    /// This installer has to run once every instance has quit.
    Installer(PathBuf),
}

/// Downloads release `tag` for `install` into `work` (an empty folder that
/// outlives this call when an installer is staged there), checks it, and
/// swaps it in. `progress` hears each step.
pub fn prepare(
    install: &Install,
    tag: &str,
    platform: &str,
    fetch: &dyn Fetch,
    work: &Path,
    os: Os,
    progress: &mut dyn FnMut(&str),
) -> Result<Staged, String> {
    std::fs::create_dir_all(work).map_err(|e| format!("couldn't make {}: {e}", work.display()))?;
    let asset = install.asset(tag, platform);
    let file = work.join(&asset);
    progress(&format!("downloading {asset}"));
    fetch.fetch(&download_url(tag, &asset), &file)?;
    let sums = work.join("SHA256SUMS.txt");
    fetch.fetch(&download_url(tag, "SHA256SUMS.txt"), &sums)?;
    let sums = std::fs::read_to_string(&sums).map_err(|e| format!("couldn't read SHA256SUMS.txt: {e}"))?;
    verify(&file, &asset, &sums)?;

    progress("installing");
    let name = format!("tt-git-gui-{tag}-{platform}");
    match install {
        Install::WindowsInstaller { .. } => return Ok(Staged::Installer(file)),
        Install::WindowsFolder { dir } => {
            let unpacked = work.join("unpacked");
            unpack(&file, &unpacked, os)?;
            replace_files(&both_exes(&unpacked.join(&name), dir, os), os)?;
        }
        Install::Prefix { bin } => {
            let unpacked = work.join("unpacked");
            unpack(&file, &unpacked, os)?;
            replace_files(&both_exes(&unpacked.join(&name).join("bin"), bin, os), os)?;
        }
        Install::AppImage { file: target } => {
            replace_files(&[(file.clone(), target.clone())], os)?;
        }
        Install::AppBundle { app } => {
            let new = sibling(app, ".new");
            copy_from_dmg(&file, work, &new)?;
            replace_dir(&new, app)?;
        }
    }
    Ok(Staged::Replaced)
}

/// `tt-git-gui` and `tt-git` in `from`, each paired with where it goes in
/// `to`. A package without `tt-git` (it carries it today) leaves it out.
fn both_exes(from: &Path, to: &Path, os: Os) -> Vec<(PathBuf, PathBuf)> {
    ["tt-git-gui", "tt-git"]
        .iter()
        .map(|stem| exe_name(os, stem))
        .map(|name| (from.join(&name), to.join(&name)))
        .filter(|(new, _)| new.is_file() || new.ends_with(exe_name(os, "tt-git-gui")))
        .collect()
}

/// Checks `file` against its line (`<sha256>  <asset>`) in a release's
/// `SHA256SUMS.txt`.
pub fn verify(file: &Path, asset: &str, sums: &str) -> Result<(), String> {
    let expected = sums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
        .map(|(sum, _)| sum.to_ascii_lowercase())
        .ok_or_else(|| format!("SHA256SUMS.txt has no checksum for {asset}"))?;
    let mut hasher = Sha256::new();
    let mut f = std::fs::File::open(file).map_err(|e| format!("couldn't read {asset}: {e}"))?;
    std::io::copy(&mut f, &mut hasher).map_err(|e| format!("couldn't read {asset}: {e}"))?;
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{asset} doesn't match its checksum; it may be damaged, try again"
        ))
    }
}

/// Unpacks a .zip or .tar.gz with the system's `tar` (on Windows the one in
/// System32, which reads zip; another `tar` on `PATH`, Git's say, may not).
pub fn unpack(archive: &Path, into: &Path, os: Os) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| format!("couldn't make {}: {e}", into.display()))?;
    let tar = match (os, std::env::var_os("SystemRoot")) {
        (Os::Windows, Some(root)) => PathBuf::from(root).join("System32").join("tar.exe"),
        _ => PathBuf::from("tar"),
    };
    let mut cmd = Command::new(tar);
    cmd.arg("-xf").arg(archive).arg("-C").arg(into);
    run(cmd, "tar")
}

/// `path` with `suffix` added to its file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Puts each new file where its target is.
///
/// On Windows a running .exe can't be overwritten or deleted, but it can
/// be renamed: the old one moves aside to `<name>.old` (removed at the next
/// start by [`remove_leftovers`]) and the new one is copied in. Elsewhere
/// the new file is copied next to the target and renamed over it, which
/// leaves a running copy running from the old file.
pub fn replace_files(pairs: &[(PathBuf, PathBuf)], os: Os) -> Result<(), String> {
    for (new, target) in pairs {
        let failed = |e: std::io::Error| format!("couldn't replace {}: {e}", target.display());
        if os == Os::Windows {
            let aside = target.exists().then(|| set_aside(target)).transpose().map_err(failed)?;
            if let Err(e) = std::fs::copy(new, target) {
                if let Some(aside) = aside {
                    let _ = std::fs::rename(aside, target);
                }
                return Err(failed(e));
            }
        } else {
            let tmp = sibling(&target.with_file_name(format!(".{}", file_name(target))), ".new");
            std::fs::copy(new, &tmp).map_err(failed)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(failed)?;
            }
            if let Err(e) = std::fs::rename(&tmp, target) {
                let _ = std::fs::remove_file(&tmp);
                return Err(failed(e));
            }
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Renames `target` to `<target>.old`, or `<target>.old-<n>` while an
/// earlier `.old` is still running and can't be removed.
fn set_aside(target: &Path) -> std::io::Result<PathBuf> {
    let old = sibling(target, ".old");
    let aside = if !old.exists() || std::fs::remove_file(&old).is_ok() {
        old
    } else {
        (1..)
            .map(|n| sibling(target, &format!(".old-{n}")))
            .find(|p| !p.exists())
            .expect("a free name")
    };
    std::fs::rename(target, &aside)?;
    Ok(aside)
}

/// Removes what [`replace_files`] and [`replace_dir`] set aside in `dir`,
/// skipping any copy that's still running.
pub fn remove_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = ["tt-git-gui", "tt-git.exe", "tt-git.app"]
            .iter()
            .any(|p| name.starts_with(p));
        if ours && (name.ends_with(".old") || name.contains(".old-")) {
            let path = entry.path();
            let removed = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match removed {
                Ok(()) => tracing::info!("removed {} from an earlier update", path.display()),
                Err(e) => tracing::debug!("kept {}: {e}", path.display()),
            }
        }
    }
}

/// Swaps the folder `new` in for `target` (the app bundle): `target` moves
/// aside, `new` takes its name, and the old one is removed. A running app
/// keeps running from the files it has open.
pub fn replace_dir(new: &Path, target: &Path) -> Result<(), String> {
    let old = sibling(target, ".old");
    if old.exists() {
        let _ = std::fs::remove_dir_all(&old);
    }
    std::fs::rename(target, &old).map_err(|e| format!("couldn't move {} aside: {e}", target.display()))?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(format!("couldn't replace {}: {e}", target.display()));
    }
    if let Err(e) = std::fs::remove_dir_all(&old) {
        tracing::warn!("couldn't remove {}: {e}", old.display());
    }
    Ok(())
}

/// Copies `tt-git.app` out of the .dmg `dmg` to `to`.
fn copy_from_dmg(dmg: &Path, work: &Path, to: &Path) -> Result<(), String> {
    let mount = work.join("mnt");
    let mut attach = Command::new("hdiutil");
    attach
        .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
        .arg(&mount)
        .arg(dmg);
    run(attach, "hdiutil attach")?;
    if to.exists() {
        let _ = std::fs::remove_dir_all(to);
    }
    let mut ditto = Command::new("ditto");
    ditto.arg(mount.join("tt-git.app")).arg(to);
    let copied = run(ditto, "ditto");
    let mut detach = Command::new("hdiutil");
    detach.args(["detach", "-quiet"]).arg(&mount);
    if let Err(e) = run(detach, "hdiutil detach") {
        tracing::warn!("{e}");
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn detects_each_way_the_app_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let zip = root.join("zip");
        touch(&zip.join("tt-git-gui.exe"), "");
        let exe = zip.join("tt-git-gui.exe");
        assert_eq!(
            detect(&exe, Os::Windows, None),
            Ok(Install::WindowsFolder { dir: zip.clone() })
        );
        touch(&zip.join("unins000.exe"), "");
        assert_eq!(
            detect(&exe, Os::Windows, None),
            Ok(Install::WindowsInstaller { dir: zip.clone() })
        );

        let app = root.join("Applications/tt-git.app");
        let exe = app.join("Contents/MacOS/tt-git-gui");
        assert_eq!(
            detect(&exe, Os::MacOs, None),
            Ok(Install::AppBundle { app: app.clone() })
        );
        assert!(matches!(
            detect(&root.join("tt-git-gui"), Os::MacOs, None),
            Err(Refusal::Unpackaged(_))
        ));

        let local = root.join(".local/bin");
        let exe = local.join("tt-git-gui");
        assert_eq!(
            detect(&exe, Os::Linux, None),
            Ok(Install::Prefix { bin: local.clone() })
        );
        let image = root.join("tt-git.AppImage");
        assert_eq!(
            detect(Path::new("/tmp/.mount_x/usr/bin/tt-git-gui"), Os::Linux, Some(&image)),
            Ok(Install::AppImage { file: image.clone() })
        );
        assert_eq!(
            detect(Path::new("/usr/bin/tt-git-gui"), Os::Linux, None),
            Err(Refusal::Package(PathBuf::from("/usr/bin")))
        );
        assert!(matches!(
            detect(&root.join("tt-git-gui"), Os::Linux, None),
            Err(Refusal::Unpackaged(_))
        ));
    }

    #[test]
    fn a_cargo_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".fingerprint")).unwrap();
        let exe = dir.path().join("tt-git-gui.exe");
        for os in [Os::Windows, Os::MacOs, Os::Linux] {
            assert_eq!(
                detect(&exe, os, None),
                Err(Refusal::CargoBuild(dir.path().to_path_buf()))
            );
        }
    }

    #[test]
    fn each_install_takes_its_own_download() {
        let tag = "v0.9.0";
        let dir = PathBuf::from("d");
        let cases = [
            (
                Install::WindowsFolder { dir: dir.clone() },
                "windows-x64",
                "tt-git-gui-v0.9.0-windows-x64.zip",
            ),
            (
                Install::WindowsInstaller { dir: dir.clone() },
                "windows-x64",
                "tt-git-gui-v0.9.0-windows-x64-setup.exe",
            ),
            (
                Install::AppBundle { app: dir.clone() },
                "macos-arm64",
                "tt-git-gui-v0.9.0-macos-arm64.dmg",
            ),
            (
                Install::AppImage { file: dir.clone() },
                "linux-x64",
                "tt-git-gui-v0.9.0-linux-x64.AppImage",
            ),
            (
                Install::Prefix { bin: dir.clone() },
                "linux-arm64",
                "tt-git-gui-v0.9.0-linux-arm64.tar.gz",
            ),
        ];
        for (install, platform, asset) in cases {
            assert_eq!(install.asset(tag, platform), asset);
        }
        assert_eq!(
            download_url(tag, "SHA256SUMS.txt"),
            "https://github.com/ThirtyTech/tt-git/releases/download/v0.9.0/SHA256SUMS.txt"
        );
    }

    #[test]
    fn relaunches_the_bundle_or_appimage_rather_than_the_binary_inside() {
        let app = PathBuf::from("/Applications/tt-git.app");
        assert_eq!(Install::AppBundle { app: app.clone() }.launch(Os::MacOs), app);
        let dir = PathBuf::from("C:/tt-git");
        assert_eq!(
            Install::WindowsInstaller { dir: dir.clone() }.launch(Os::Windows),
            dir.join("tt-git-gui.exe")
        );
    }

    #[test]
    fn checksums_must_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, "hello").unwrap();
        let sum = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let sums = format!("0000  other.zip\n{sum}  a.zip\n");
        assert_eq!(verify(&file, "a.zip", &sums), Ok(()));
        assert_eq!(verify(&file, "a.zip", &format!("{sum} *a.zip\n")), Ok(()));
        let wrong = verify(&file, "a.zip", &sums.replace("2cf2", "ffff")).unwrap_err();
        assert!(wrong.contains("doesn't match"), "{wrong}");
        let missing = verify(&file, "b.zip", &sums).unwrap_err();
        assert!(missing.contains("no checksum for b.zip"), "{missing}");
    }

    /// Opens `path` as a running exe is: on Windows it then can't be
    /// deleted.
    fn hold(path: &Path) -> std::fs::File {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 1;
            options.share_mode(FILE_SHARE_READ);
        }
        options.open(path).unwrap()
    }

    #[test]
    fn windows_sets_the_running_exe_aside() {
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("new/tt-git-gui.exe"), dir.path().join("tt-git-gui.exe"));
        touch(&new, "v2");
        touch(&target, "v1");
        // Still running from an earlier update.
        let old = dir.path().join("tt-git-gui.exe.old");
        touch(&old, "v0");
        let held = hold(&old);

        replace_files(&[(new.clone(), target.clone())], Os::Windows).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "v2");
        let aside: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".old"))
            .collect();
        if cfg!(windows) {
            // The open .old couldn't be removed, so v1 went to .old-1.
            assert_eq!(
                std::fs::read_to_string(dir.path().join("tt-git-gui.exe.old-1")).unwrap(),
                "v1"
            );
        } else {
            assert_eq!(std::fs::read_to_string(&old).unwrap(), "v1", "{aside:?}");
        }
        drop(held);

        remove_leftovers(dir.path());
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!left.iter().any(|n| n.contains(".old")), "{left:?}");
        assert!(left.contains(&"tt-git-gui.exe".to_string()));
    }

    #[test]
    fn elsewhere_the_new_file_is_renamed_over_the_old() {
        let dir = tempfile::tempdir().unwrap();
        let (new, target) = (dir.path().join("new/tt-git-gui"), dir.path().join("bin/tt-git-gui"));
        touch(&new, "v2");
        touch(&target, "v1");
        replace_files(&[(new, target.clone())], Os::Linux).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "v2");
        let names: Vec<_> = std::fs::read_dir(dir.path().join("bin")).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no temporary file left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn the_bundle_is_swapped_whole() {
        let dir = tempfile::tempdir().unwrap();
        let (new, app) = (dir.path().join("tt-git.app.new"), dir.path().join("tt-git.app"));
        touch(&new.join("Contents/MacOS/tt-git-gui"), "v2");
        touch(&app.join("Contents/MacOS/tt-git-gui"), "v1");
        touch(&app.join("Contents/MacOS/gone"), "");
        replace_dir(&new, &app).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/MacOS/tt-git-gui")).unwrap(),
            "v2"
        );
        assert!(!app.join("Contents/MacOS/gone").exists());
        assert!(!new.exists() && !dir.path().join("tt-git.app.old").exists());
    }

    /// Serves files from a folder, by the URL's last part.
    struct Local(PathBuf);

    impl Fetch for Local {
        fn fetch(&self, url: &str, dest: &Path) -> Result<(), String> {
            let name = url.rsplit('/').next().unwrap();
            std::fs::copy(self.0.join(name), dest)
                .map(|_| ())
                .map_err(|e| format!("{name}: {e}"))
        }
    }

    fn sha256(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap();
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A release folder holding `asset`, built by `tar` from `tree`, and
    /// its SHA256SUMS.txt.
    fn release(root: &Path, asset: &str, tree: &Path, tar_args: &[&str]) -> PathBuf {
        let rel = root.join("release");
        std::fs::create_dir_all(&rel).unwrap();
        let out = std::process::Command::new(if cfg!(windows) {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/tar.exe")
        } else {
            PathBuf::from("tar")
        })
        .args(tar_args)
        .arg(rel.join(asset))
        .arg("-C")
        .arg(tree)
        .arg(".")
        .output()
        .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let sums = format!("{}  {asset}\n", sha256(&rel.join(asset)));
        std::fs::write(rel.join("SHA256SUMS.txt"), sums).unwrap();
        rel
    }

    #[test]
    fn prepare_downloads_checks_unpacks_and_swaps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (tag, platform) = ("v9.9.9", "test-x64");
        let name = format!("tt-git-gui-{tag}-{platform}");
        let (install, os, layout, tar_args): (Install, Os, PathBuf, &[&str]) = if cfg!(windows) {
            (
                Install::WindowsFolder { dir: root.join("app") },
                Os::Windows,
                PathBuf::from(&name),
                &["-a", "-cf"],
            )
        } else {
            (
                Install::Prefix {
                    bin: root.join("app/bin"),
                },
                Os::Linux,
                PathBuf::from(&name).join("bin"),
                &["-czf"],
            )
        };
        let app_dir = install.launch(os).parent().unwrap().to_path_buf();
        let tree = root.join("tree");
        for stem in ["tt-git-gui", "tt-git"] {
            touch(&tree.join(&layout).join(exe_name(os, stem)), &format!("new {stem}"));
            touch(&app_dir.join(exe_name(os, stem)), &format!("old {stem}"));
        }
        let rel = release(root, &install.asset(tag, platform), &tree, tar_args);

        let mut steps = Vec::new();
        let staged = prepare(&install, tag, platform, &Local(rel), &root.join("work"), os, &mut |s| {
            steps.push(s.to_string())
        })
        .unwrap();
        assert_eq!(staged, Staged::Replaced);
        assert_eq!(steps[0], format!("downloading {}", install.asset(tag, platform)));
        for stem in ["tt-git-gui", "tt-git"] {
            assert_eq!(
                std::fs::read_to_string(app_dir.join(exe_name(os, stem))).unwrap(),
                format!("new {stem}")
            );
        }
    }

    #[test]
    fn prepare_stages_the_installer_and_checks_it() {
        let dir = tempfile::tempdir().unwrap();
        let rel = dir.path().join("release");
        let install = Install::WindowsInstaller {
            dir: dir.path().join("app"),
        };
        let asset = install.asset("v9.9.9", "windows-x64");
        touch(&rel.join(&asset), "setup");
        std::fs::write(
            rel.join("SHA256SUMS.txt"),
            format!("{}  {asset}\n", sha256(&rel.join(&asset))),
        )
        .unwrap();
        let work = dir.path().join("work");
        let staged = prepare(
            &install,
            "v9.9.9",
            "windows-x64",
            &Local(rel.clone()),
            &work,
            Os::Windows,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(staged, Staged::Installer(work.join(&asset)));
        assert!(!dir.path().join("app").exists(), "nothing installed yet");

        // A damaged download stops before anything is replaced.
        std::fs::write(rel.join(&asset), "tampered").unwrap();
        let err = prepare(
            &install,
            "v9.9.9",
            "windows-x64",
            &Local(rel),
            &dir.path().join("w2"),
            Os::Windows,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.contains("doesn't match"), "{err}");
    }
}
