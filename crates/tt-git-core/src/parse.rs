//! Parsers for machine-readable git output. Each takes the raw bytes git
//! printed so they can be unit tested on fixture strings.

use std::collections::HashSet;
use std::sync::Arc;

use crate::git::GitError;
use crate::model::{
    BranchStatus, Checkout, Commit, CommitFile, FileState, GitRef, Oid, RefKind, Stash, Status, StatusEntry, Worktree,
};
use crate::undo::ReflogEntry;

/// Field separator inside a record (ASCII unit separator).
pub const FS: char = '\u{1f}';

/// `git log` arguments producing records [`parse_commit`] understands.
/// Records are NUL-terminated (`-z`), fields separated by [`FS`].
pub const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%aN%x1f%aE%x1f%at%x1f%ct%x1f%s";

/// Parses one record of [`LOG_FORMAT`].
pub fn parse_commit(record: &[u8]) -> Result<Commit, GitError> {
    parse_commit_interned(record, &mut Idents::default())
}

/// Author names and emails seen so far in one log, so that commits by the
/// same author share one allocation for each.
#[derive(Debug, Default)]
pub struct Idents(HashSet<Arc<str>>);

impl Idents {
    pub fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(known) = self.0.get(s) {
            return known.clone();
        }
        let new: Arc<str> = s.into();
        self.0.insert(new.clone());
        new
    }
}

/// [`parse_commit`], sharing names and emails through `idents`.
pub fn parse_commit_interned(record: &[u8], idents: &mut Idents) -> Result<Commit, GitError> {
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
    let oid = next("hash")?;
    let parents = next("parents")?;
    let author_name = idents.intern(next("author name")?);
    let author_email = idents.intern(next("author email")?);
    let author_time = parse_i64(next("author time")?)?;
    let commit_time = parse_i64(next("commit time")?)?;
    let summary = next("subject")?.trim_end_matches(['\n', '\r']).into();
    let oid = Oid::from_hex(oid)
        .filter(|o| o.hex_len() >= 40)
        .ok_or_else(|| GitError::Parse(format!("bad commit hash {oid:?}")))?;
    let parents = parents
        .split_whitespace()
        .map(|p| Oid::from_hex(p).ok_or_else(|| GitError::Parse(format!("bad parent hash {p:?}"))))
        .collect::<Result<_, _>>()?;
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
pub const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(creatordate:unix)%00%(upstream)%00%(upstream:track,nobracket)%00%(symref)%00%(HEAD)%00%(authorname)%00%(authoremail)%00%(subject)";

/// Parses `git for-each-ref` output in [`REF_FORMAT`]. `remotes` is the list
/// of configured remote names, used to split `refs/remotes/<remote>/<branch>`
/// correctly when remote names contain slashes.
pub fn parse_refs(out: &[u8], remotes: &[String]) -> Result<Vec<GitRef>, GitError> {
    parse_refs_with_authors(out, remotes).map(|(refs, _)| refs)
}

/// Full ref names, each with the author of the commit it points at as
/// `Name <email>`.
pub type RefAuthors = Vec<(String, String)>;

/// [`parse_refs`], plus each ref's full name with the author of the commit
/// it points at as `Name <email>` (before `.mailmap`).
pub fn parse_refs_with_authors(out: &[u8], remotes: &[String]) -> Result<(Vec<GitRef>, RefAuthors), GitError> {
    let text = String::from_utf8_lossy(out);
    let mut refs = Vec::new();
    let mut authors = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 11 {
            return Err(GitError::Parse(format!("ref line has {} fields: {line:?}", f.len())));
        }
        let full_name = f[0].to_string();
        let (kind, short_name) = classify_ref(&full_name, remotes);
        let target = if f[2].is_empty() { f[1] } else { f[2] }.to_string();
        let (ahead, behind, upstream_gone) = parse_track(f[5]);
        authors.push((full_name.clone(), contact(f[8], f[9])));
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
            subject: f[10..].join("\0"),
        });
    }
    Ok((refs, authors))
}

/// Parses `git config -z --get-regexp` output for `remote.*` and
/// `tt-git.basebranch`: the remote names as `git remote` lists them (every
/// `<name>` with a `remote.<name>.<key>`, sorted, except git's invalid ones
/// starting with `/`), and the base branch setting as `git config --get`
/// gives it (the last value, trimmed; none when empty).
pub fn parse_remote_config(out: &[u8]) -> (Vec<String>, Option<String>) {
    let mut remotes = Vec::new();
    let mut base = None;
    for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let record = String::from_utf8_lossy(record);
        // Key, then a newline and the value unless it has none.
        let (key, value) = match record.split_once('\n') {
            Some((key, value)) => (key, Some(value)),
            None => (&*record, None),
        };
        if key == "tt-git.basebranch" {
            base = value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        } else if let Some((name, _)) = key.strip_prefix("remote.").and_then(|k| k.rsplit_once('.'))
            && !name.is_empty()
            && !name.starts_with('/')
        {
            remotes.push(name.to_string());
        }
    }
    remotes.sort();
    remotes.dedup();
    (remotes, base)
}

/// `Name <email>` from `%(authorname)` and `%(authoremail)` (which git
/// prints with or without the brackets).
pub fn contact(name: &str, email: &str) -> String {
    if email.starts_with('<') {
        format!("{name} {email}")
    } else {
        format!("{name} <{email}>")
    }
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
/// Checkouts made by undo count too.
pub fn parse_checkouts(out: &[u8]) -> Vec<Checkout> {
    String::from_utf8_lossy(out)
        .lines()
        .filter_map(|line| {
            let (selector, subject) = line.split_once(FS)?;
            let subject = subject
                .strip_prefix(crate::undo::UNDO_MARK)
                .and_then(|s| s.strip_prefix(' '))
                .unwrap_or(subject);
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

/// `git log -g` format for [`parse_reflog`]: the commit and the subject.
pub const UNDO_LOG_FORMAT: &str = "--format=%H%x1f%gs";

/// Parses HEAD's reflog in [`UNDO_LOG_FORMAT`], newest first.
pub fn parse_reflog(out: &[u8]) -> Vec<ReflogEntry> {
    String::from_utf8_lossy(out)
        .lines()
        .filter_map(|line| {
            let (oid, subject) = line.split_once(FS)?;
            Some(ReflogEntry {
                oid: oid.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// `git stash list` arguments producing records [`parse_stashes`] understands.
pub const STASH_FORMAT: &str = "--format=%gd%x1f%H%x1f%P%x1f%ct%x1f%gs";

/// Parses `git stash list` output in [`STASH_FORMAT`].
pub fn parse_stashes(out: &[u8]) -> Vec<Stash> {
    String::from_utf8_lossy(out)
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(5, FS);
            let (selector, oid, parents, date, message) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
            let index = selector.strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
            Some(Stash {
                index,
                oid: oid.to_string(),
                untracked: parents.split(' ').nth(2).map(str::to_string),
                date: date.parse().ok()?,
                message: message.to_string(),
            })
        })
        .collect()
}

/// Parses `git merge-tree --write-tree --stdin --name-only -z --no-messages`:
/// for each merge in input order, `1` (clean) or `0` (conflicts), the tree,
/// the conflicted paths, then an empty field. Each merge's conflicted
/// paths, empty when it's clean; `None` if the output doesn't read that way.
pub fn parse_merge_tree_stdin(out: &[u8]) -> Option<Vec<Vec<String>>> {
    let mut fields = out.split(|&b| b == 0);
    let mut merges = Vec::new();
    loop {
        let clean = match fields.next()? {
            b"" => return Some(merges),
            b"1" => true,
            b"0" => false,
            _ => return None,
        };
        fields.next().filter(|tree| !tree.is_empty())?;
        let mut paths = Vec::new();
        loop {
            match fields.next()? {
                b"" => break,
                path => paths.push(String::from_utf8_lossy(path).into_owned()),
            }
        }
        if clean != paths.is_empty() {
            return None;
        }
        merges.push(paths);
    }
}

/// Parses `git worktree list --porcelain`: one block of `key value` lines
/// per working tree, blank-line separated, the main one first. `current`
/// is left `false`.
pub fn parse_worktrees(out: &[u8]) -> Vec<Worktree> {
    let text = String::from_utf8_lossy(out);
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        if line.is_empty() {
            open = false;
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "worktree" {
            worktrees.push(Worktree {
                path: value.into(),
                head: None,
                branch: None,
                main: worktrees.is_empty(),
                bare: false,
                locked: None,
                prunable: None,
                current: false,
            });
            open = true;
            continue;
        }
        let Some(wt) = worktrees.last_mut().filter(|_| open) else {
            continue;
        };
        match key {
            // All zeros on an unborn branch.
            "HEAD" if !value.bytes().all(|b| b == b'0') => wt.head = Some(value.to_string()),
            "branch" => wt.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
            "bare" => wt.bare = true,
            "locked" => wt.locked = Some(value.to_string()),
            "prunable" => wt.prunable = Some(value.to_string()),
            _ => {}
        }
    }
    worktrees
}

/// Parses `git show --name-status -z`: a status token (`M`, `A`, `R100`, …)
/// then one path, or two (source, destination) for renames and copies.
pub fn parse_name_status(out: &[u8]) -> Vec<CommitFile> {
    let text = String::from_utf8_lossy(out);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(token) = fields.next() {
        let state = token
            .bytes()
            .next()
            .and_then(FileState::from_code)
            .unwrap_or(FileState::Modified);
        let two_paths = matches!(state, FileState::Renamed | FileState::Copied);
        let Some(first) = fields.next() else { break };
        let (orig_path, path) = if two_paths {
            match fields.next() {
                Some(dest) => (Some(first.to_string()), dest.to_string()),
                None => break,
            }
        } else {
            (None, first.to_string())
        };
        files.push(CommitFile { state, path, orig_path });
    }
    files
}

/// Timestamp of the last entry of a raw reflog file whose message `keep`
/// accepts (`<old> <new> <name> <<email>> <time> <tz>\t<message>` per line).
pub fn last_reflog_time(contents: &[u8], keep: impl Fn(&str) -> bool) -> Option<i64> {
    let text = String::from_utf8_lossy(contents);
    reflog_line_time(last_kept_line(&text, &keep)?)
}

/// [`last_reflog_time`] of a whole reflog of `len` bytes, reading only as
/// much of its end as it takes to hold the last kept line: `window` bytes at
/// first, doubling until a whole kept line fits. Gives up with `None` once
/// `limit` bytes hold none.
pub fn tail_reflog_time<R: std::io::Read + std::io::Seek>(
    file: &mut R,
    len: u64,
    window: u64,
    limit: u64,
    keep: impl Fn(&str) -> bool,
) -> std::io::Result<Option<i64>> {
    let mut window = window.max(1);
    loop {
        let start = len.saturating_sub(window);
        file.seek(std::io::SeekFrom::Start(start))?;
        let mut buf = vec![0; (len - start) as usize];
        file.read_exact(&mut buf)?;
        if start == 0 {
            return Ok(last_reflog_time(&buf, keep));
        }
        // The window may start mid-line; the lines after its first newline
        // are whole, and are the file's last lines. (A newline byte is never
        // part of a multi-byte character, so decoding from there matches
        // decoding the whole file.)
        if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let text = String::from_utf8_lossy(&buf[nl + 1..]);
            if let Some(line) = last_kept_line(&text, &keep) {
                return Ok(reflog_line_time(line));
            }
        }
        if window >= limit {
            return Ok(None);
        }
        window = window.saturating_mul(2);
    }
}

fn last_kept_line<'a>(text: &'a str, keep: &impl Fn(&str) -> bool) -> Option<&'a str> {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty() && keep(l.split_once('\t').map_or("", |(_, msg)| msg)))
}

fn reflog_line_time(line: &str) -> Option<i64> {
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
    fn parses_merge_tree_stdin() {
        let out = format!("1\0{A}\0\x000\0{B}\0f.txt\0sub dir/n m.txt\0\x001\0{C}\0\0");
        assert_eq!(
            parse_merge_tree_stdin(out.as_bytes()),
            Some(vec![vec![], vec!["f.txt".into(), "sub dir/n m.txt".into()], vec![]])
        );
        assert_eq!(parse_merge_tree_stdin(b""), Some(vec![]));
        // Cut off mid-merge, or clean with paths.
        assert_eq!(parse_merge_tree_stdin(format!("0\0{B}\0f.txt\0").as_bytes()), None);
        assert_eq!(parse_merge_tree_stdin(format!("1\0{B}\0f.txt\0\0").as_bytes()), None);
    }

    #[test]
    fn parses_name_status() {
        let out = b"M\0src/a.rs\0R087\0old name.txt\0new name.txt\0A\0b.txt\0D\0c.txt\0";
        let files = parse_name_status(out);
        let summary: Vec<_> = files
            .iter()
            .map(|f| (f.state.code(), f.orig_path.as_deref(), f.path.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ('M', None, "src/a.rs"),
                ('R', Some("old name.txt"), "new name.txt"),
                ('A', None, "b.txt"),
                ('D', None, "c.txt"),
            ]
        );
        assert!(parse_name_status(b"").is_empty());
    }

    #[test]
    fn parses_commits() {
        let rec = log_record(A, &format!("{B} {C}"), "Merge branch 'x' into main");
        let c = parse_commit(rec.as_bytes()).unwrap();
        assert_eq!(c.oid, A);
        assert_eq!(*c.parents, [B, C]);
        assert_eq!(&*c.author_name, "Ann Author");
        assert_eq!(&*c.author_email, "ann@example.com");
        assert_eq!((c.author_time, c.commit_time), (100, 200));
        assert_eq!(&*c.summary, "Merge branch 'x' into main");
        assert!(c.is_merge());
        assert_eq!(c.short_oid(), "1111111");
    }

    #[test]
    fn parses_root_commit_and_odd_summaries() {
        let rec = format!("\n{}", log_record(A, "", "fix: ünïcode — and a \u{1f}separator"));
        let c = parse_commit(rec.as_bytes()).unwrap();
        assert!(c.parents.is_empty());
        // The subject is the last field, so a stray separator stays in it.
        assert_eq!(&*c.summary, "fix: ünïcode — and a \u{1f}separator");
        let empty = log_record(B, A, "");
        assert_eq!(&*parse_commit(empty.as_bytes()).unwrap().summary, "");
    }

    #[test]
    fn rejects_garbage_commit() {
        assert!(parse_commit(b"not a commit").is_err());
        assert!(parse_commit(log_record("abc", "", "x").as_bytes()).is_err());
        // git prints lowercase ids.
        assert!(parse_commit(log_record(&A.replace('1', "A"), "", "x").as_bytes()).is_err());
        assert!(parse_commit(log_record(A, "not-a-parent", "x").as_bytes()).is_err());
    }

    #[test]
    fn parses_sha256_commits() {
        let long = "a".repeat(64);
        let c = parse_commit(log_record(&long, &long, "x").as_bytes()).unwrap();
        assert_eq!(c.oid, long);
        assert_eq!(*c.parents, [long.as_str()]);
        assert_eq!(c.short_oid(), "aaaaaaa");
    }

    /// A [`REF_FORMAT`] line from its fields before the author's, the
    /// subject last; the author is "Ann Author <ann@example.com>".
    fn ref_line(fields: [&str; 9]) -> String {
        let mut all = fields.to_vec();
        all.splice(8..8, ["Ann Author", "<ann@example.com>"]);
        all.join("\0")
    }

    #[test]
    fn parses_ref_authors() {
        let line = |name: &str, author: &str, email: &str| {
            [name, A, "", "300", "", "", "", " ", author, email, "subject"].join("\0")
        };
        let out = [
            line("refs/heads/main", "Ann Author", "<ann@example.com>"),
            line("refs/heads/bare", "Bob", "bob@example.com"),
            line("refs/tags/v1", "", ""),
        ]
        .join("\n");
        let (refs, authors) = parse_refs_with_authors(out.as_bytes(), &[]).unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].subject, "subject");
        assert_eq!(
            authors,
            [
                (
                    "refs/heads/main".to_string(),
                    "Ann Author <ann@example.com>".to_string()
                ),
                ("refs/heads/bare".to_string(), "Bob <bob@example.com>".to_string()),
                ("refs/tags/v1".to_string(), " <>".to_string()),
            ]
        );
    }

    #[test]
    fn parses_remote_config() {
        let out = [
            "remote.origin.url\nhttps://example.com/a.git",
            "remote.origin.fetch\n+refs/heads/*:refs/remotes/origin/*",
            "remote.dotted.name.url\nx",
            "remote.a/b.fetch\ny",
            "remote.pushdefault\norigin",
            "remote./bad.url\nz",
            "remote..url\nz",
            "remote.flag.skipdefaultupdate",
            "tt-git.basebranch\n  first  ",
            "tt-git.basebranch\nmain",
        ]
        .join("\0");
        let (remotes, base) = parse_remote_config(format!("{out}\0").as_bytes());
        assert_eq!(remotes, ["a/b", "dotted.name", "flag", "origin"]);
        assert_eq!(base.as_deref(), Some("main"));
        // The last value counts, even when it's empty or missing.
        assert_eq!(
            parse_remote_config(b"tt-git.basebranch\nmain\0tt-git.basebranch\0").1,
            None
        );
        assert_eq!(
            parse_remote_config(b"tt-git.basebranch\n  x \n").1.as_deref(),
            Some("x")
        );
        assert_eq!(parse_remote_config(b""), (Vec::new(), None));
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
             HEAD@{{300}}{FS}checkout: moving from {A} to main\n\
             HEAD@{{200}}{FS}[tt-git undo] checkout: moving from main to old\n\
             HEAD@{{100}}{FS}[tt-git undo]: updating HEAD\n"
        );
        let cs = parse_checkouts(out.as_bytes());
        assert_eq!(cs.len(), 3);
        assert_eq!((cs[2].to.as_str(), cs[2].time), ("old", 200));
        assert_eq!(
            (cs[0].from.as_str(), cs[0].to.as_str(), cs[0].time),
            ("main", "feature/to-x", 500)
        );
        assert_eq!((cs[1].to.as_str(), cs[1].time), ("main", 300));
    }

    #[test]
    fn parses_reflog() {
        let out = format!("{A}{FS}commit: two\n{B}{FS}checkout: moving from a to b\n");
        let entries = parse_reflog(out.as_bytes());
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].oid.as_str(), entries[0].subject.as_str()),
            (A, "commit: two")
        );
        assert_eq!(entries[1].subject, "checkout: moving from a to b");
    }

    #[test]
    fn reads_last_reflog_time() {
        let contents = format!(
            "{A} {B} Test User <t@e.com> 1700000000 +0000\tcommit (initial): x\n\
             {B} {C} Test User <t@e.com> 1700000123 -0500\tfetch: fast-forward\n"
        );
        assert_eq!(last_reflog_time(contents.as_bytes(), |_| true), Some(1_700_000_123));
        assert_eq!(
            last_reflog_time(contents.as_bytes(), |m| !m.starts_with("fetch")),
            Some(1_700_000_000)
        );
        assert_eq!(last_reflog_time(contents.as_bytes(), |_| false), None);
        assert_eq!(last_reflog_time(b"", |_| true), None);
    }

    #[test]
    fn tail_reflog_time_gives_up_at_the_limit() {
        let line = |t: u64, msg: &str| format!("{A} {B} Test User <t@e.com> {t} +0000\t{msg}\n");
        let mut contents = line(1_700_000_000, "update by push");
        for _ in 0..100 {
            contents.push_str(&line(1_700_000_500, "fetch: fast-forward"));
        }
        let len = contents.len() as u64;
        let push = |m: &str| m.starts_with("update by push");
        let mut file = std::io::Cursor::new(contents.as_bytes());
        assert_eq!(tail_reflog_time(&mut file, len, 64, 1024, push).unwrap(), None);
        assert_eq!(
            tail_reflog_time(&mut file, len, 64, len, push).unwrap(),
            Some(1_700_000_000)
        );
    }

    #[test]
    fn tail_reflog_time_matches_reading_everything() {
        let line = |t: u64, msg: &str| format!("{A} {B} Test User <t@e.com> {t} +0000\t{msg}");
        let long = line(1_700_000_999, &"x".repeat(10_000));
        let two = format!("{}\n{}", line(1_700_000_000, "one"), line(1_700_000_123, "two"));
        let crlf = two.replace('\n', "\r\n");
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"\n".to_vec(),
            b"\n\n  \r\n\t\n".to_vec(),
            line(1_700_000_000, "single").into_bytes(),
            format!("{}\n", line(1_700_000_000, "single")).into_bytes(),
            format!("{two}\n").into_bytes(),
            two.clone().into_bytes(),
            format!("{two}\n\n   \n").into_bytes(),
            crlf.clone().into_bytes(),
            format!("{crlf}\r\n").into_bytes(),
            format!("{two}\r").into_bytes(),
            format!("{two}\n{long}\n").into_bytes(),
            format!("{two}\n{long}").into_bytes(),
            format!("{long}\n{two}\n").into_bytes(),
            format!("{two}\ngarbage without a time\n").into_bytes(),
            format!("{two}\n{A} {B} No Email 1700000500 +0000\tmsg\n").into_bytes(),
            // Invalid UTF-8 on both sides of a line break, multi-byte
            // characters a small window cuts through, and a last line that
            // is only Unicode whitespace (U+2028).
            [
                two.as_bytes(),
                b"\n\xff\xfe",
                line(1_700_000_777, "caf\u{e9} \u{1f600}").as_bytes(),
                b"\xc3\n",
            ]
            .concat(),
            [
                line(1_700_000_001, "\u{1f600}\u{1f600}").as_bytes(),
                b"\n\xe2\x80\xa8\n",
            ]
            .concat(),
        ];
        let keeps: [&dyn Fn(&str) -> bool; 2] = [&|_| true, &|m| m.starts_with("one")];
        for contents in cases {
            for keep in keeps {
                let want = last_reflog_time(&contents, keep);
                let len = contents.len() as u64;
                for window in (0..=len + 2).filter(|w| *w < 200 || w % 97 == 0 || *w + 3 > len) {
                    let mut file = std::io::Cursor::new(&contents);
                    let got = tail_reflog_time(&mut file, len, window, u64::MAX, keep).unwrap();
                    assert_eq!(got, want, "window {window} of {:?}", String::from_utf8_lossy(&contents));
                }
            }
        }
    }

    #[test]
    fn parses_worktree_list() {
        let out = b"worktree C:/r
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree C:/r/.worktrees/a b
HEAD 2222222222222222222222222222222222222222
detached
locked

worktree C:/r/.worktrees/gone
HEAD 3333333333333333333333333333333333333333
branch refs/heads/feat/x
locked on usb
prunable gitdir file points to non-existent location

worktree C:/r/.worktrees/new
HEAD 0000000000000000000000000000000000000000
branch refs/heads/new

";
        let wts = parse_worktrees(out);
        assert_eq!(wts.len(), 4);
        assert!(wts[0].main && !wts[1].main);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert_eq!(wts[1].path, std::path::PathBuf::from("C:/r/.worktrees/a b"));
        assert_eq!((wts[1].branch.as_deref(), wts[1].locked.as_deref()), (None, Some("")));
        assert_eq!(wts[1].head.as_deref(), Some("2222222222222222222222222222222222222222"));
        assert_eq!(wts[2].branch.as_deref(), Some("feat/x"));
        assert_eq!(wts[2].locked.as_deref(), Some("on usb"));
        assert!(wts[2].prunable.is_some());
        assert_eq!(wts[3].head, None, "unborn");

        let bare = parse_worktrees(
            b"worktree /srv/r.git
bare

worktree /srv/w
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main
",
        );
        assert!(bare[0].bare && bare[0].main && !bare[1].bare);
        assert!(parse_worktrees(b"").is_empty());
    }
}
