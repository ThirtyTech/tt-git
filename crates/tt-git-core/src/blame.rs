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

/// How a committed version of a file stands against `HEAD`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recency {
    /// The same as in `HEAD`: no commit since has changed it.
    Latest,
    /// This many commits on the way to `HEAD` have changed it since.
    Behind(usize),
    /// Different from `HEAD`'s, and not in its history: from another branch.
    Elsewhere,
}

/// The most edits [`follow_line`] looks through before it gives up on
/// diffing and keeps the line's place instead.
const FOLLOW_MAX_EDITS: usize = 1000;

/// Where line `line` (from 0) of `from` is in `to`, two versions of a file:
/// following the line by its text as lines come and go around it. A line
/// both versions share maps to itself; a changed line maps to the most like
/// it among the lines that replaced it, and an added or removed one to
/// where it was. Returns 0 when `to` is empty.
pub fn follow_line(from: &[&str], to: &[&str], line: usize) -> usize {
    if from.is_empty() {
        return 0;
    }
    let last = to.len().saturating_sub(1);
    let line = line.min(from.len() - 1);
    let prefix = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    let suffix = from[prefix..]
        .iter()
        .rev()
        .zip(to[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if line < prefix {
        return line;
    }
    if line >= from.len() - suffix {
        return (to.len() + line).saturating_sub(from.len()).min(last);
    }
    let a = &from[prefix..from.len() - suffix];
    let b = &to[prefix..to.len() - suffix];
    let l = line - prefix;
    let Some(matches) = matching_lines(a, b, FOLLOW_MAX_EDITS) else {
        // Too different to diff: keep its place, scaled to the new length.
        return (prefix + l * b.len() / a.len().max(1)).min(last);
    };
    // The lines both sides keep around it, with the ends standing in for
    // matches before the first line and after the last.
    let after = matches.partition_point(|&(x, _)| x < l);
    if let Some(&(x, y)) = matches.get(after)
        && x == l
    {
        return prefix + y;
    }
    let (from_start, to_start) = after
        .checked_sub(1)
        .map_or((0, 0), |i| (matches[i].0 + 1, matches[i].1 + 1));
    let (from_end, to_end) = matches.get(after).copied().unwrap_or((a.len(), b.len()));
    if to_start == to_end {
        // Gone: the line that came after it.
        return (prefix + to_start).min(last);
    }
    // Replaced: the most like it, nearest its place among the replacements.
    let place = to_start + (l - from_start) * (to_end - to_start) / (from_end - from_start);
    let text = a[l].trim();
    let best = (to_start..to_end)
        .map(|j| (similarity(text, b[j].trim()), std::cmp::Reverse(j.abs_diff(place)), j))
        .max_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)))
        .map_or(place, |(score, _, j)| if score > 0.0 { j } else { place });
    prefix + best
}

/// The lines `a` and `b` have in common, as `(a index, b index)` in order:
/// a Myers diff, or `None` past `max_edits` insertions and deletions.
fn matching_lines(a: &[&str], b: &[&str], max_edits: usize) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let limit = max_edits.min(a.len() + b.len()) as isize;
    // v[k] is the furthest x on diagonal k = x - y, offset to index from 0.
    let off = limit + 1;
    let mut v = vec![0isize; 2 * off as usize + 1];
    let mut trace = Vec::new();
    for d in 0..=limit {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let i = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[i - 1] < v[i + 1]) {
                v[i + 1]
            } else {
                v[i - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[i] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m, off));
            }
        }
    }
    None
}

/// Walks a Myers trace back from the end, collecting the diagonal moves.
fn backtrack(trace: &[Vec<isize>], n: isize, m: isize, off: isize) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let (mut x, mut y) = (n, m);
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let i = (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[i - 1] < v[i + 1]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + off) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            matches.push((x as usize, y as usize));
        }
        x = prev_x;
        y = prev_y;
    }
    matches.reverse();
    matches
}

/// How alike two lines are, from 0 to 1: the share of character pairs they
/// have in common (single characters for a line shorter than two).
fn similarity(a: &str, b: &str) -> f64 {
    fn pairs(s: &str) -> Vec<(char, char)> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() == 1 {
            return vec![(chars[0], '\0')];
        }
        let mut pairs: Vec<_> = chars.windows(2).map(|w| (w[0], w[1])).collect();
        pairs.sort_unstable();
        pairs
    }
    if a == b {
        return 1.0;
    }
    let (pa, pb) = (pairs(a), pairs(b));
    if pa.is_empty() || pb.is_empty() {
        return 0.0;
    }
    // Both sorted: count the pairs they share, each at most once.
    let (mut i, mut j, mut shared) = (0, 0, 0);
    while i < pa.len() && j < pb.len() {
        match pa[i].cmp(&pb[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * f64::from(shared) / (pa.len() + pb.len()) as f64
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
             author Mario\n\
             author-mail <mario@lumen.dev>\n\
             author-time 1700000000\n\
             author-tz +0000\n\
             committer Mario\n\
             summary Add the client\n\
             boundary\n\
             filename src/old.ts\n\
             \tfirst\n\
             {A} 2 2\n\
             \tsecond\n\
             {B} 3 3 1\n\
             author Cow\n\
             author-mail <cow@lumen.dev>\n\
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
        assert_eq!(a.author, "Mario");
        assert_eq!(a.author_mail, "mario@lumen.dev");
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

    fn lines(s: &str) -> Vec<&str> {
        s.split(' ').filter(|l| !l.is_empty()).collect()
    }

    #[test]
    fn follow_line_keeps_a_line_as_lines_come_and_go_around_it() {
        let old = lines("a b c d e");
        // Two lines added above "d", one removed below it.
        let new = lines("x y a b c d f");
        assert_eq!(follow_line(&old, &new, 3), 5);
        assert_eq!(follow_line(&new, &old, 5), 3);
        // Shared start and end.
        assert_eq!(follow_line(&old, &new, 0), 2);
        assert_eq!(follow_line(&new, &old, 0), 0);
        // "e" was replaced by "f".
        assert_eq!(follow_line(&old, &new, 4), 6);
    }

    #[test]
    fn follow_line_picks_the_most_like_a_changed_line() {
        let old = ["fn main() {", "    let a = 1;", "    let total = a + 2;", "}"];
        let new = [
            "fn main() {",
            "    // Start.",
            "    let a = 1;",
            "    log(a);",
            "    let total = a + 3;",
            "    log(total);",
            "}",
        ];
        assert_eq!(follow_line(&new, &old, 4), 2);
        assert_eq!(follow_line(&old, &new, 2), 4);
        // An added line goes to where it was added.
        assert_eq!(follow_line(&new, &old, 1), 1);
    }

    #[test]
    fn follow_line_handles_empty_and_too_different_versions() {
        assert_eq!(follow_line(&lines("a b"), &[], 1), 0);
        assert_eq!(follow_line(&[], &lines("a b"), 0), 0);
        let old: Vec<String> = (0..3000).map(|i| format!("old {i}")).collect();
        let new: Vec<String> = (0..1500).map(|i| format!("new {i}")).collect();
        let old: Vec<&str> = old.iter().map(String::as_str).collect();
        let new: Vec<&str> = new.iter().map(String::as_str).collect();
        // Past the diff's limit: the same place, scaled.
        assert_eq!(follow_line(&old, &new, 2000), 1000);
    }

    #[test]
    fn empty_output_is_an_empty_blame() {
        let blame = Blame::parse_porcelain(b"");
        assert!(blame.lines.is_empty());
        assert_eq!(blame.path_of(0), None);
    }
}
