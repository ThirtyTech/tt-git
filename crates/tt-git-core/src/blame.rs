//! `git blame`: which commit last changed each line of a file, parsed from
//! `--porcelain` output.

/// A commit that last changed some of the blamed lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlameCommit {
    pub oid: String,
    pub author: String,
    /// Without the angle brackets.
    pub author_mail: String,
    /// Unix seconds.
    pub author_time: i64,
    pub summary: String,
    /// The commit before this one that touched the file, and the file's path
    /// there: where to look to see a line as it was before this commit.
    /// `None` when this commit added the file, or for a boundary commit.
    pub previous: Option<(String, String)>,
    /// The oldest commit blame looked at, standing in for everything before it.
    pub boundary: bool,
}

impl BlameCommit {
    /// The lines changed in the working tree, not yet committed (git reports
    /// them as the all-zero commit).
    pub fn is_uncommitted(&self) -> bool {
        !self.oid.is_empty() && self.oid.bytes().all(|b| b == b'0')
    }

    pub fn short(&self) -> &str {
        &self.oid[..self.oid.len().min(7)]
    }
}

/// One line of the blamed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    /// Index into [`Blame::commits`].
    pub commit: usize,
    /// The line's number in its commit's version of the file.
    pub orig_line: u32,
    /// Index into [`Blame::paths`]: the file's path in that commit (it can
    /// differ from the blamed path after a rename, or with `-C` for a line
    /// that came from another file).
    pub path: usize,
    pub text: String,
}

/// Every line of a file with the commit that last changed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub paths: Vec<String>,
    pub lines: Vec<BlameLine>,
}

impl Blame {
    /// Parses `git blame --porcelain` output. A commit's details come the
    /// first time it appears; later groups of its lines repeat only its id.
    pub fn parse_porcelain(out: &[u8]) -> Blame {
        let mut blame = Blame::default();
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        // Each commit's last `filename`, as an index into `paths`.
        let mut commit_path: Vec<usize> = Vec::new();
        let mut current: Option<(usize, u32)> = None;
        for raw in out.split(|&b| b == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if let Some(text) = raw.strip_prefix(b"\t") {
                let Some((commit, orig_line)) = current.take() else {
                    continue;
                };
                blame.lines.push(BlameLine {
                    commit,
                    orig_line,
                    path: commit_path[commit],
                    text: String::from_utf8_lossy(text).into_owned(),
                });
                continue;
            }
            let line = String::from_utf8_lossy(raw);
            let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
            if is_oid(key) {
                let orig_line = value.split(' ').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                let commit = *index.entry(key.to_string()).or_insert_with(|| {
                    blame.commits.push(BlameCommit {
                        oid: key.to_string(),
                        ..BlameCommit::default()
                    });
                    commit_path.push(0);
                    blame.commits.len() - 1
                });
                current = Some((commit, orig_line));
                continue;
            }
            let Some((commit, _)) = current else { continue };
            let c = &mut blame.commits[commit];
            match key {
                "author" => c.author = value.to_string(),
                "author-mail" => {
                    c.author_mail = value.trim_start_matches('<').trim_end_matches('>').to_string();
                }
                "author-time" => c.author_time = value.parse().unwrap_or(0),
                "summary" => c.summary = value.to_string(),
                "boundary" => c.boundary = true,
                "previous" => {
                    c.previous = value
                        .split_once(' ')
                        .map(|(oid, path)| (oid.to_string(), crate::patch::unquote_c(path)));
                }
                "filename" => {
                    let path = crate::patch::unquote_c(value);
                    commit_path[commit] = match blame.paths.iter().position(|p| *p == path) {
                        Some(i) => i,
                        None => {
                            blame.paths.push(path);
                            blame.paths.len() - 1
                        }
                    };
                }
                _ => {}
            }
        }
        if blame.paths.is_empty() {
            blame.paths.push(String::new());
        }
        blame
    }

    pub fn commit_of(&self, line: usize) -> Option<&BlameCommit> {
        self.lines.get(line).map(|l| &self.commits[l.commit])
    }

    /// The path line `line` had in its commit.
    pub fn path_of(&self, line: usize) -> Option<&str> {
        self.lines.get(line).map(|l| self.paths[l.path].as_str())
    }

    /// How many lines came from commit `commit`.
    pub fn lines_from(&self, commit: usize) -> usize {
        self.lines.iter().filter(|l| l.commit == commit).count()
    }
}

/// A full 40- or 64-character hex object id.
fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "a996cfb511fd22f2f2cb0e5be7915fef8b51ae60";
    const B: &str = "75de15bce03d6469241b3bf1fe63d4acb2968f84";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    fn porcelain() -> String {
        format!(
            "{A} 1 1 2\n\
             author Ada Park\n\
             author-mail <ada@lumen.dev>\n\
             author-time 1700000000\n\
             author-tz +0000\n\
             committer Ada Park\n\
             summary Add the client\n\
             boundary\n\
             filename src/old.ts\n\
             \tfirst\n\
             {A} 2 2\n\
             \tsecond\n\
             {B} 3 3 1\n\
             author Sam Rivera\n\
             author-mail <sam@lumen.dev>\n\
             author-time 1700001000\n\
             summary Retry on 5xx\n\
             previous {A} src/old.ts\n\
             filename src/client.ts\n\
             \tthird\twith a tab\n\
             {A} 4 4 1\n\
             \tfourth\n\
             {ZERO} 5 5 1\n\
             author Not Committed Yet\n\
             author-mail <not.committed.yet>\n\
             author-time 1700002000\n\
             summary Version of src/client.ts from src/client.ts\n\
             filename src/client.ts\n\
             \tfifth\n"
        )
    }

    #[test]
    fn parses_lines_and_commits() {
        let blame = Blame::parse_porcelain(porcelain().as_bytes());
        let texts: Vec<_> = blame.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["first", "second", "third\twith a tab", "fourth", "fifth"]);
        assert_eq!(blame.commits.len(), 3);

        let a = blame.commit_of(0).unwrap();
        assert_eq!(a.oid, A);
        assert_eq!(a.short(), "a996cfb");
        assert_eq!(a.author, "Ada Park");
        assert_eq!(a.author_mail, "ada@lumen.dev");
        assert_eq!(a.author_time, 1_700_000_000);
        assert_eq!(a.summary, "Add the client");
        assert!(a.boundary);
        assert_eq!(a.previous, None);
        // A later group of a known commit has only its id; it keeps the
        // commit's details and path.
        assert_eq!(blame.commit_of(3).unwrap().oid, A);
        assert_eq!(blame.path_of(3), Some("src/old.ts"));
        assert_eq!(blame.lines[3].orig_line, 4);

        let b = blame.commit_of(2).unwrap();
        assert_eq!(b.previous, Some((A.to_string(), "src/old.ts".to_string())));
        assert!(!b.boundary);
        assert_eq!(blame.path_of(2), Some("src/client.ts"));

        assert!(blame.commit_of(4).unwrap().is_uncommitted());
        assert!(!a.is_uncommitted());
        assert_eq!(blame.lines_from(0), 3);
    }

    #[test]
    fn empty_output_is_an_empty_blame() {
        let blame = Blame::parse_porcelain(b"");
        assert!(blame.lines.is_empty());
        assert_eq!(blame.path_of(0), None);
    }
}
