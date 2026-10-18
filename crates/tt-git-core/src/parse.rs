//! Parsers for machine-readable git output. Each takes the raw bytes git
//! printed so they can be unit tested on fixture strings.

use crate::git::GitError;
use crate::model::{BranchStatus, Checkout, Commit, FileState, GitRef, RefKind, Status, StatusEntry};

/// Field separator inside a record (ASCII unit separator).
pub const FS: char = '\u{1f}';

/// `git log` arguments producing records [`parse_commit`] understands.
/// Records are NUL-terminated (`-z`), fields separated by [`FS`].
pub const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%ct%x1f%s";

/// Parses one record of [`LOG_FORMAT`].
pub fn parse_commit(record: &[u8]) -> Result<Commit, GitError> {
    let text = String::from_utf8_lossy(record);
    // git separates -z records with NUL but may also leave a newline from the
    // previous record's end; trim only leading line breaks.
    let text = text.trim_start_matches(['\n', '\r']);
    let mut fields = text.splitn(7, FS);
    let mut next = |name: &str| {
        fields
            .next()
            .ok_or_else(|| GitError::Parse(format!("commit record missing {name}: {text:?}")))
    };
    let oid = next("hash")?.to_string();
    let parents = next("parents")?.split_whitespace().map(str::to_string).collect();
    let author_name = next("author name")?.to_string();
    let author_email = next("author email")?.to_string();
    let author_time = parse_i64(next("author time")?)?;
    let commit_time = parse_i64(next("commit time")?)?;
    let summary = next("subject")?.trim_end_matches(['\n', '\r']).to_string();
    if oid.len() < 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(GitError::Parse(format!("bad commit hash {oid:?}")));
    }
    Ok(Commit {
        oid,
        parents,
        author_name,
        author_email,
        author_time,
        commit_time,
        summary,
    })
}

fn parse_i64(s: &str) -> Result<i64, GitError> {
    s.trim()
        .parse()
        .map_err(|_| GitError::Parse(format!("bad timestamp {s:?}")))
}

/// `for-each-ref` format for [`parse_refs`]: NUL-separated fields, one ref per line.
pub const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(creatordate:unix)%00%(upstream)%00%(upstream:track,nobracket)%00%(symref)%00%(HEAD)%00%(subject)";

/// Parses `git for-each-ref` output in [`REF_FORMAT`]. `remotes` is the list
/// of configured remote names, used to split `refs/remotes/<remote>/<branch>`
/// correctly when remote names contain slashes.
pub fn parse_refs(out: &[u8], remotes: &[String]) -> Result<Vec<GitRef>, GitError> {
    let text = String::from_utf8_lossy(out);
    let mut refs = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 9 {
            return Err(GitError::Parse(format!("ref line has {} fields: {line:?}", f.len())));
        }
        let full_name = f[0].to_string();
        let (kind, short_name) = classify_ref(&full_name, remotes);
        let target = if f[2].is_empty() { f[1] } else { f[2] }.to_string();
        let (ahead, behind, upstream_gone) = parse_track(f[5]);
        refs.push(GitRef {
            full_name,
            short_name,
            kind,
            target,
            date: f[3].trim().parse().unwrap_or(0),
            upstream: non_empty(f[4]),
            ahead,
            behind,
            upstream_gone,
            is_head: f[7] == "*",
            symref: non_empty(f[6]),
            subject: f[8..].join("\0"),
        });
    }
    Ok(refs)
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn classify_ref(full: &str, remotes: &[String]) -> (RefKind, String) {
    if let Some(name) = full.strip_prefix("refs/heads/") {
        return (RefKind::LocalBranch, name.to_string());
    }
    if let Some(rest) = full.strip_prefix("refs/remotes/") {
        // Longest configured remote that prefixes the rest wins.
        let remote = remotes
            .iter()
            .filter(|r| rest.len() > r.len() && rest.starts_with(r.as_str()) && rest.as_bytes()[r.len()] == b'/')
            .max_by_key(|r| r.len())
            .cloned()
            .unwrap_or_else(|| rest.split('/').next().unwrap_or(rest).to_string());
        return (RefKind::RemoteBranch { remote }, rest.to_string());
    }
    if let Some(name) = full.strip_prefix("refs/tags/") {
        return (RefKind::Tag, name.to_string());
    }
    (RefKind::Other, full.strip_prefix("refs/").unwrap_or(full).to_string())
}

/// Parses `%(upstream:track,nobracket)`: `ahead 1, behind 2`, `gone`, or empty.
fn parse_track(s: &str) -> (u32, u32, bool) {
    if s == "gone" {
        return (0, 0, true);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in s.split(',').map(str::trim) {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

/// Parses `git status --porcelain=v2 -z --branch` output.
pub fn parse_status(out: &[u8]) -> Result<Status, GitError> {
    let mut status = Status::default();
    let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(record) = fields.next() {
        let line = String::from_utf8_lossy(record);
        let bad = || GitError::Parse(format!("bad status record {line:?}"));
        let (tag, rest) = line.split_once(' ').ok_or_else(bad)?;
        match tag {
            "#" => parse_branch_header(rest, &mut status.branch),
            "1" => {
                // 1 XY sub mH mI mW hH hI path
                let parts: Vec<&str> = rest.splitn(8, ' ').collect();
                if parts.len() < 8 {
                    return Err(bad());
                }
                let (index, worktree) = parse_xy(parts[0]).ok_or_else(bad)?;
                status.entries.push(StatusEntry {
                    path: parts[7].to_string(),
                    orig_path: None,
                    index,
                    worktree,
                });
            }
            "2" => {
                // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
                let parts: Vec<&str> = rest.splitn(9, ' ').collect();
                if parts.len() < 9 {
                    return Err(bad());
                }
                let (index, worktree) = parse_xy(parts[0]).ok_or_else(bad)?;
                let orig = fields.next().ok_or_else(bad)?;
                status.entries.push(StatusEntry {
                    path: parts[8].to_string(),
                    orig_path: Some(String::from_utf8_lossy(orig).into_owned()),
                    index,
                    worktree,
                });
            }
            "u" => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                if parts.len() < 10 {
                    return Err(bad());
                }
                let (index, worktree) = parse_xy(parts[0]).ok_or_else(bad)?;
                // Normalise both-modified/added-by-us/... to "unmerged" on
                // the side git marks with U, keeping A/D for add/delete conflicts.
                let fix = |s: FileState| {
                    if s == FileState::Unmodified {
                        FileState::Unmerged
                    } else {
                        s
                    }
                };
                status.entries.push(StatusEntry {
                    path: parts[9].to_string(),
                    orig_path: None,
                    index: fix(index),
                    worktree: fix(worktree),
                });
            }
            "?" => status.entries.push(StatusEntry {
                path: rest.to_string(),
                orig_path: None,
                index: FileState::Untracked,
                worktree: FileState::Untracked,
            }),
            "!" => status.entries.push(StatusEntry {
                path: rest.to_string(),
                orig_path: None,
                index: FileState::Ignored,
                worktree: FileState::Ignored,
            }),
            _ => return Err(bad()),
        }
    }
    Ok(status)
}

fn parse_xy(xy: &str) -> Option<(FileState, FileState)> {
    let b = xy.as_bytes();
    if b.len() != 2 {
        return None;
    }
    Some((FileState::from_code(b[0])?, FileState::from_code(b[1])?))
}

fn parse_branch_header(rest: &str, branch: &mut BranchStatus) {
    let Some((key, value)) = rest.split_once(' ') else {
        return;
    };
    match key {
        "branch.oid" => branch.oid = (value != "(initial)").then(|| value.to_string()),
        "branch.head" => branch.head = (value != "(detached)").then(|| value.to_string()),
        "branch.upstream" => branch.upstream = Some(value.to_string()),
        "branch.ab" => {
            for part in value.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    branch.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    branch.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

/// `git log -g` format for [`parse_checkouts`]. With `--date=unix` the
/// selector `%gd` is `HEAD@{<unix time of the reflog entry>}`; `%ct` would be
/// the commit's time, which is wrong for checkouts of old commits.
pub const REFLOG_FORMAT: &str = "--format=%gd%x1f%gs";

/// Extracts `checkout: moving from A to B` entries (newest first, as git
/// prints them) from HEAD's reflog in [`REFLOG_FORMAT`], one per line.
pub fn parse_checkouts(out: &[u8]) -> Vec<Checkout> {
    String::from_utf8_lossy(out)
        .lines()
        .filter_map(|line| {
            let (selector, subject) = line.split_once(FS)?;
            let rest = subject.strip_prefix("checkout: moving from ")?;
            let (from, to) = rest.split_once(" to ")?;
            let time = selector.rsplit_once("@{")?.1.strip_suffix('}')?;
            Some(Checkout {
                from: from.to_string(),
                to: to.to_string(),
                time: time.trim().parse().ok()?,
            })
        })
        .collect()
}

/// Timestamp of the last entry of a raw reflog file
/// (`<old> <new> <name> <<email>> <time> <tz>\t<message>` per line).
pub fn last_reflog_time(contents: &[u8]) -> Option<i64> {
    let text = String::from_utf8_lossy(contents);
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    let ident = line.split('\t').next()?;
    // After the email's closing '>' come "<time> <tz>".
    let after_email = &ident[ident.rfind('>')? + 1..];
    after_email.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const C: &str = "3333333333333333333333333333333333333333";

    fn log_record(oid: &str, parents: &str, summary: &str) -> String {
        format!("{oid}{FS}{parents}{FS}Ann Author{FS}ann@example.com{FS}100{FS}200{FS}{summary}")
    }

    #[test]
    fn parses_commits() {
        let rec = log_record(A, &format!("{B} {C}"), "Merge branch 'x' into main");
        let c = parse_commit(rec.as_bytes()).unwrap();
        assert_eq!(c.oid, A);
        assert_eq!(c.parents, vec![B, C]);
        assert_eq!(c.author_name, "Ann Author");
        assert_eq!(c.author_email, "ann@example.com");
        assert_eq!((c.author_time, c.commit_time), (100, 200));
        assert_eq!(c.summary, "Merge branch 'x' into main");
        assert!(c.is_merge());
        assert_eq!(c.short_oid(), "1111111");
    }

    #[test]
    fn parses_root_commit_and_odd_summaries() {
        let rec = format!("\n{}", log_record(A, "", "fix: ünïcode — and a \u{1f}separator"));
        let c = parse_commit(rec.as_bytes()).unwrap();
        assert!(c.parents.is_empty());
        // The subject is the last field, so a stray separator stays in it.
        assert_eq!(c.summary, "fix: ünïcode — and a \u{1f}separator");
        let empty = log_record(B, A, "");
        assert_eq!(parse_commit(empty.as_bytes()).unwrap().summary, "");
    }

    #[test]
    fn rejects_garbage_commit() {
        assert!(parse_commit(b"not a commit").is_err());
        assert!(parse_commit(log_record("abc", "", "x").as_bytes()).is_err());
    }

    fn ref_line(fields: [&str; 9]) -> String {
        fields.join("\0")
    }

    #[test]
    fn parses_refs() {
        let out = [
            ref_line([
                "refs/heads/main",
                A,
                "",
                "300",
                "refs/remotes/origin/main",
                "ahead 2, behind 1",
                "",
                "*",
                "tip",
            ]),
            ref_line([
                "refs/heads/old",
                B,
                "",
                "100",
                "refs/remotes/origin/old",
                "gone",
                "",
                " ",
                "old work",
            ]),
            ref_line([
                "refs/remotes/origin/HEAD",
                A,
                "",
                "300",
                "",
                "",
                "refs/remotes/origin/main",
                " ",
                "tip",
            ]),
            ref_line(["refs/remotes/my/fork/feature/x", C, "", "250", "", "", "", " ", "feat"]),
            ref_line([
                "refs/tags/v1.0",
                "4444444444444444444444444444444444444444",
                B,
                "150",
                "",
                "",
                "",
                " ",
                "release",
            ]),
        ]
        .join("\n");
        let refs = parse_refs(out.as_bytes(), &["origin".into(), "my".into(), "my/fork".into()]).unwrap();
        assert_eq!(refs.len(), 5);

        let main = &refs[0];
        assert_eq!(main.kind, RefKind::LocalBranch);
        assert_eq!(main.short_name, "main");
        assert!(main.is_head);
        assert_eq!((main.ahead, main.behind, main.upstream_gone), (2, 1, false));
        assert_eq!(main.upstream.as_deref(), Some("refs/remotes/origin/main"));

        assert!(refs[1].upstream_gone);
        assert!(!refs[1].is_head);
        assert_eq!(refs[2].symref.as_deref(), Some("refs/remotes/origin/main"));

        let fork = &refs[3];
        assert_eq!(
            fork.kind,
            RefKind::RemoteBranch {
                remote: "my/fork".into()
            }
        );
        assert_eq!(fork.short_name, "my/fork/feature/x");
        assert_eq!(fork.branch_name(), "feature/x");

        let tag = &refs[4];
        assert_eq!(tag.kind, RefKind::Tag);
        assert_eq!(tag.target, B, "annotated tags are peeled");
    }

    #[test]
    fn parses_status() {
        let mut out = Vec::new();
        for rec in [
            "# branch.oid 1111111111111111111111111111111111111111",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +3 -1",
            "1 M. N... 100644 100644 100644 aaaa bbbb src/staged.rs",
            "1 .M N... 100644 100644 100644 aaaa aaaa dir with space/file.txt",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
            "1 A. N... 000000 100644 100644 0000 bbbb new.rs",
            "1 .D N... 100644 100644 000000 aaaa aaaa gone.rs",
            "2 R. N... 100644 100644 100644 aaaa aaaa R100 renamed.rs",
            "old name.rs",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflict.rs",
            "u AU N... 000000 100644 100644 100644 0000 bbbb cccc added-by-us.rs",
            "? untracked ünï.txt",
        ] {
            out.extend_from_slice(rec.as_bytes());
            out.push(0);
        }
        let st = parse_status(&out).unwrap();
        assert_eq!(st.branch.head.as_deref(), Some("main"));
        assert_eq!(st.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.branch.ahead, st.branch.behind), (3, 1));
        let codes: Vec<(String, &str)> = st.entries.iter().map(|e| (e.short_code(), e.path.as_str())).collect();
        assert_eq!(
            codes,
            vec![
                ("M ".into(), "src/staged.rs"),
                (" M".into(), "dir with space/file.txt"),
                ("MM".into(), "both.rs"),
                ("A ".into(), "new.rs"),
                (" D".into(), "gone.rs"),
                ("R ".into(), "renamed.rs"),
                ("UU".into(), "conflict.rs"),
                ("AU".into(), "added-by-us.rs"),
                ("??".into(), "untracked ünï.txt"),
            ]
        );
        assert_eq!(st.entries[5].orig_path.as_deref(), Some("old name.rs"));
        let both = &st.entries[2];
        assert!(both.has_staged() && both.has_unstaged());
        assert!(st.entries[6].is_conflicted() && !st.entries[6].has_staged());
        assert!(st.entries[8].is_untracked() && st.entries[8].has_unstaged());
    }

    #[test]
    fn parses_initial_and_detached_status() {
        let st = parse_status(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(st.branch.oid, None);
        assert_eq!(st.branch.head.as_deref(), Some("main"));
        assert!(st.is_clean());
        let st = parse_status(format!("# branch.oid {A}\0# branch.head (detached)\0").as_bytes()).unwrap();
        assert_eq!(st.branch.head, None);
    }

    #[test]
    fn parses_checkouts() {
        let out = format!(
            "HEAD@{{500}}{FS}checkout: moving from main to feature/to-x\n\
             HEAD@{{400}}{FS}commit: something\n\
             HEAD@{{300}}{FS}checkout: moving from {A} to main\n"
        );
        let cs = parse_checkouts(out.as_bytes());
        assert_eq!(cs.len(), 2);
        assert_eq!(
            (cs[0].from.as_str(), cs[0].to.as_str(), cs[0].time),
            ("main", "feature/to-x", 500)
        );
        assert_eq!((cs[1].to.as_str(), cs[1].time), ("main", 300));
    }

    #[test]
    fn reads_last_reflog_time() {
        let contents = format!(
            "{A} {B} Test User <t@e.com> 1700000000 +0000\tcommit (initial): x\n\
             {B} {C} Test User <t@e.com> 1700000123 -0500\tfetch: fast-forward\n"
        );
        assert_eq!(last_reflog_time(contents.as_bytes()), Some(1_700_000_123));
        assert_eq!(last_reflog_time(b""), None);
    }
}
