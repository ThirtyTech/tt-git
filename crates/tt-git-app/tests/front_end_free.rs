//! tt-git-app is shared by every front end, so it mustn't depend on the
//! TUI's terminal libraries, even indirectly.

use std::process::Command;

#[test]
fn depends_on_no_terminal_library() {
    let out = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--package",
            "tt-git-app",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree runs");
    assert!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8_lossy(&out.stdout);
    assert!(tree.lines().any(|l| l.starts_with("tt-git-core ")), "{tree}");
    for banned in ["ratatui", "crossterm"] {
        let found: Vec<&str> = tree
            .lines()
            .filter(|l| l.split(' ').next().is_some_and(|name| name.starts_with(banned)))
            .collect();
        assert!(found.is_empty(), "tt-git-app depends on {banned}: {found:?}");
    }
}
