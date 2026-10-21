//! Opening a file in the user's editor: which editor, and the arguments
//! that put it on the right line. Starting it is the runner's job.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The `[editor]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorConfig {
    /// The editor command. `{file}` and `{line}` in it make it a template.
    pub command: Option<String>,
    /// The editor runs in this terminal (tt-git waits for it), rather than
    /// in a window of its own. Worked out from the program when unset.
    pub terminal: Option<bool>,
}

/// A file to open, relative to the repo root, and the line to open it at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditTarget {
    pub path: String,
    pub line: Option<u32>,
    /// The commit to take the file from when it isn't in the working tree.
    pub rev: Option<String>,
}

/// A read-only copy of a file as of a commit, written for the editor
/// because the file isn't in the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The file, relative to the repo root.
    pub path: String,
    /// The commit it came from, abbreviated.
    pub commit: String,
    /// Where the copy is.
    pub file: PathBuf,
    pub line: Option<u32>,
}

/// Writes `contents`, `path` as of `commit`, under `dir` and makes it
/// read-only. A copy already there is reused: it can't have changed.
pub fn write_snapshot(dir: &Path, commit: &str, path: &str, contents: &[u8]) -> std::io::Result<PathBuf> {
    let file = abs_path(&dir.join(commit), path);
    if file.exists() {
        return Ok(file);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file, contents)?;
    let mut perms = std::fs::metadata(&file)?.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms)?;
    Ok(file)
}

/// An editor ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorLaunch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Runs in this terminal: tt-git steps aside until it exits.
    pub terminal: bool,
    /// Where the command came from, for error messages.
    pub from: &'static str,
}

/// The editor command to use and where it came from: the config, then
/// `$VISUAL`, `$EDITOR`, git's `core.editor`, then a default.
pub fn choose(
    cfg: &EditorConfig,
    env: impl Fn(&str) -> Option<String>,
    core_editor: impl FnOnce() -> Option<String>,
) -> (String, &'static str) {
    let set = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    if let Some(cmd) = set(cfg.command.clone()) {
        return (cmd, "[editor] command");
    }
    if let Some(cmd) = set(env("VISUAL")) {
        return (cmd, "$VISUAL");
    }
    if let Some(cmd) = set(env("EDITOR")) {
        return (cmd, "$EDITOR");
    }
    if let Some(cmd) = set(core_editor()) {
        return (cmd, "core.editor");
    }
    let default = if cfg!(windows) { "notepad" } else { "vi" };
    (default.to_string(), "the default editor")
}

/// The launch that opens `file` at `line` with editor `command`, run from
/// the repo at `root`.
pub fn launch(
    cfg: &EditorConfig,
    command: &str,
    from: &'static str,
    root: &Path,
    file: &Path,
    line: Option<u32>,
) -> Result<EditorLaunch, String> {
    let mut words = split_command(command, cfg!(windows));
    if words.is_empty() {
        return Err(format!("the editor command from {from} is empty"));
    }
    let program = words.remove(0);
    let file = file.display().to_string();
    let stem = stem(&program);
    if words.iter().any(|w| w.contains("{file}")) {
        let line = line.unwrap_or(1).to_string();
        for w in &mut words {
            *w = w.replace("{file}", &file).replace("{line}", &line);
        }
    } else {
        words.extend(line_args(&stem, &file, line));
    }
    Ok(EditorLaunch {
        program,
        args: words,
        cwd: root.to_path_buf(),
        terminal: cfg.terminal.unwrap_or(!is_gui(&stem)),
        from,
    })
}

/// `root` joined with a `/`-separated repo path, one component at a time
/// so Windows paths don't end up with mixed separators.
pub fn abs_path(root: &Path, path: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    p.extend(path.split('/').filter(|c| !c.is_empty()));
    p
}

/// Splits a command line into words. Double and single quotes group;
/// outside Windows a backslash escapes the next character (on Windows it's
/// a path separator).
pub fn split_command(s: &str, windows: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => word.push(c),
            (_, '\\') if !windows => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                in_word = true;
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// The program's file name, lowercased, without a Windows executable
/// extension: `C:\Tools\Code.cmd` → `code`.
fn stem(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(s) = name.strip_suffix(ext) {
            return s.to_string();
        }
    }
    name
}

const JETBRAINS: &[&str] = &[
    "idea",
    "idea64",
    "pycharm",
    "pycharm64",
    "goland",
    "goland64",
    "rustrover",
    "rustrover64",
    "webstorm",
    "webstorm64",
    "clion",
    "clion64",
    "rider",
    "rider64",
    "phpstorm",
    "phpstorm64",
];

/// The arguments that open `file` at `line` in the editor named `stem`.
fn line_args(stem: &str, file: &str, line: Option<u32>) -> Vec<String> {
    let Some(line) = line else {
        return vec![file.to_string()];
    };
    match stem {
        "vi" | "vim" | "nvim" | "gvim" | "nano" | "micro" | "emacs" | "emacsclient" | "kak" | "joe" | "mg" | "ne" => {
            vec![format!("+{line}"), file.to_string()]
        }
        "code" | "code-insiders" | "codium" | "cursor" | "windsurf" => {
            vec!["--goto".into(), format!("{file}:{line}")]
        }
        "hx" | "helix" | "zed" | "subl" | "sublime_text" => vec![format!("{file}:{line}")],
        "notepad++" => vec![format!("-n{line}"), file.to_string()],
        s if JETBRAINS.contains(&s) => vec!["--line".into(), line.to_string(), file.to_string()],
        _ => vec![file.to_string()],
    }
}

/// Editors that open a window of their own rather than running in the
/// terminal.
fn is_gui(stem: &str) -> bool {
    matches!(
        stem,
        "code"
            | "code-insiders"
            | "codium"
            | "cursor"
            | "windsurf"
            | "zed"
            | "subl"
            | "sublime_text"
            | "notepad"
            | "notepad++"
            | "gedit"
            | "kate"
            | "mate"
    ) || JETBRAINS.contains(&stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_path() -> PathBuf {
        abs_path(Path::new("/repo"), "src/main.rs")
    }

    fn run(command: &str, line: Option<u32>) -> EditorLaunch {
        launch(
            &EditorConfig::default(),
            command,
            "test",
            Path::new("/repo"),
            &file_path(),
            line,
        )
        .unwrap()
    }

    fn file() -> String {
        file_path().display().to_string()
    }

    #[test]
    fn chooses_config_then_env_then_git() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        let cfg = EditorConfig {
            command: Some("hx".into()),
            terminal: None,
        };
        let both = env(&[("VISUAL", "nvim"), ("EDITOR", "nano")]);
        assert_eq!(choose(&cfg, both, || None), ("hx".into(), "[editor] command"));
        let none = EditorConfig::default();
        assert_eq!(choose(&none, both, || None), ("nvim".into(), "$VISUAL"));
        let editor = env(&[("VISUAL", " "), ("EDITOR", "nano")]);
        assert_eq!(choose(&none, editor, || None), ("nano".into(), "$EDITOR"));
        assert_eq!(
            choose(&none, env(&[]), || Some("code --wait".into())),
            ("code --wait".into(), "core.editor")
        );
        let (cmd, from) = choose(&none, env(&[]), || None);
        assert_eq!(from, "the default editor");
        assert_eq!(cmd, if cfg!(windows) { "notepad" } else { "vi" });
    }

    #[test]
    fn splits_commands() {
        assert_eq!(split_command("  code   --wait ", false), ["code", "--wait"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst"#, true),
            [r"C:\Program Files\Notepad++\notepad++.exe", "-multiInst"]
        );
        assert_eq!(split_command(r"C:\tools\vim.exe", true), [r"C:\tools\vim.exe"]);
        assert_eq!(split_command(r"my\ editor 'a b' x", false), ["my editor", "a b", "x"]);
        assert_eq!(split_command(r#"ed "" x"#, false), ["ed", "", "x"]);
        assert!(split_command("   ", false).is_empty());
    }

    #[test]
    fn puts_the_line_where_each_editor_wants_it() {
        let f = file();
        assert_eq!(run("nvim", Some(7)).args, ["+7".to_string(), f.clone()]);
        assert_eq!(
            run("/usr/bin/vim -p", Some(7)).args,
            ["-p".to_string(), "+7".into(), f.clone()]
        );
        assert_eq!(
            run("code --wait", Some(7)).args,
            ["--wait".to_string(), "--goto".into(), format!("{f}:7")]
        );
        assert_eq!(
            run("C:/bin/Code.cmd", Some(7)).args,
            ["--goto".to_string(), format!("{f}:7")]
        );
        assert_eq!(run("hx", Some(7)).args, [format!("{f}:7")]);
        assert_eq!(run("notepad++", Some(7)).args, ["-n7".to_string(), f.clone()]);
        assert_eq!(
            run("idea64.exe", Some(7)).args,
            ["--line".to_string(), "7".into(), f.clone()]
        );
        assert_eq!(run("notepad", Some(7)).args, std::slice::from_ref(&f));
        assert_eq!(run("nvim", None).args, std::slice::from_ref(&f));
    }

    #[test]
    fn templates_take_file_and_line() {
        let f = file();
        let l = run("ed {file} +{line}", Some(3));
        assert_eq!(
            (l.program.as_str(), l.args.clone()),
            ("ed", vec![f.clone(), "+3".into()])
        );
        assert_eq!(run("ed {file}:{line}", None).args, [format!("{f}:1")]);
        assert_eq!(run("hx {file}", Some(3)).args, [f]);
    }

    #[test]
    fn terminal_or_window() {
        assert!(run("nvim", None).terminal);
        assert!(run("some-editor", None).terminal);
        assert!(!run("code", None).terminal);
        assert!(!run(r#""C:/Program Files/Notepad++/notepad++.exe""#, None).terminal);
        let cfg = EditorConfig {
            command: None,
            terminal: Some(false),
        };
        let l = launch(&cfg, "nvim", "test", Path::new("/repo"), &file_path(), None).unwrap();
        assert!(!l.terminal);
    }

    #[test]
    fn empty_command_is_an_error() {
        let err = launch(
            &EditorConfig::default(),
            "  ",
            "$EDITOR",
            Path::new("/r"),
            Path::new("/r/f"),
            None,
        )
        .unwrap_err();
        assert!(err.contains("$EDITOR"), "{err}");
    }

    #[test]
    fn snapshots_are_read_only_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_snapshot(
            dir.path(),
            "abc1234",
            "src/a.rs",
            b"one
",
        )
        .unwrap();
        assert_eq!(file, abs_path(&dir.path().join("abc1234"), "src/a.rs"));
        assert!(std::fs::metadata(&file).unwrap().permissions().readonly());
        let again = write_snapshot(
            dir.path(),
            "abc1234",
            "src/a.rs",
            b"one
",
        )
        .unwrap();
        assert_eq!(again, file);
        assert_eq!(
            std::fs::read(&file).unwrap(),
            b"one
"
        );
    }
}
