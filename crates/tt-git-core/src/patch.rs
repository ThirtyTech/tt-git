//! One file's unified diff split into hunks, and partial patches built from
//! it so single hunks or lines can be staged, unstaged or discarded with
//! `git apply`.

/// What a line of a hunk does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The line without its `+`/`-`/space prefix. A CR before the newline
    /// is kept so patches reproduce CRLF files exactly.
    pub text: String,
    /// Followed by `\ No newline at end of file`.
    pub no_newline: bool,
}

impl DiffLine {
    pub fn is_change(&self) -> bool {
        self.kind != LineKind::Context
    }
}

/// One `@@` hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// What follows the second `@@` (git's function context), if anything.
    pub section: String,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// The `@@ -a,b +c,d @@ section` line.
    pub fn heading(&self) -> String {
        heading(
            self.old_start,
            self.old_count,
            self.new_start,
            self.new_count,
            &self.section,
        )
    }

    /// The new-side line number of the hunk's line `idx`. A deleted line
    /// has none of its own, so it gets the new-side line that follows it.
    pub fn new_line_of(&self, idx: usize) -> u32 {
        let before = self.lines[..idx.min(self.lines.len())]
            .iter()
            .filter(|l| l.kind != LineKind::Del)
            .count() as u32;
        // `git diff` numbers an empty range by the line before it.
        let start = if self.new_count == 0 {
            self.new_start + 1
        } else {
            self.new_start
        };
        (start + before).max(1)
    }

    /// The index of the hunk's first added or deleted line (0 if none).
    pub fn first_change(&self) -> usize {
        self.lines.iter().position(DiffLine::is_change).unwrap_or(0)
    }
}

/// A single file's diff: the lines before the first hunk, then its hunks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
}

/// How a partial patch will be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchDirection {
    /// Applied as is, e.g. staging part of the unstaged diff.
    Forward,
    /// Applied with `-R`, e.g. unstaging part of the staged diff or
    /// discarding part of the unstaged one.
    Reverse,
}

impl FileDiff {
    /// Parses `git diff` output for one file. Returns `None` for anything
    /// whose hunks can't be split up line by line: a binary file, a combined
    /// (conflict) diff, or more than one file.
    pub fn parse(text: &str) -> Option<Self> {
        let mut diff = FileDiff::default();
        let mut lines = text.split('\n').peekable();
        // `split` yields an empty last piece for text ending in a newline.
        let mut pieces: Vec<&str> = Vec::new();
        while let Some(l) = lines.next() {
            if lines.peek().is_some() || !l.is_empty() {
                pieces.push(l);
            }
        }
        let mut files = 0;
        for line in pieces {
            if line.starts_with("diff --git ") {
                files += 1;
                if files > 1 {
                    return None;
                }
            }
            if line.starts_with("diff --cc ") || line.starts_with("diff --combined ") || line.starts_with("@@@") {
                return None;
            }
            if let Some(hunk) = parse_heading(line) {
                diff.hunks.push(hunk);
                continue;
            }
            let Some(hunk) = diff.hunks.last_mut() else {
                if line.starts_with("Binary files ") || line == "GIT binary patch" {
                    return None;
                }
                diff.header.push(line.to_string());
                continue;
            };
            let (kind, text) = match line.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &line[1..]),
                Some(b'+') => (LineKind::Add, &line[1..]),
                Some(b'-') => (LineKind::Del, &line[1..]),
                Some(b'\\') => {
                    if let Some(last) = hunk.lines.last_mut() {
                        last.no_newline = true;
                    }
                    continue;
                }
                // An empty context line whose space was stripped (some
                // editors and mailers do this).
                None => (LineKind::Context, ""),
                Some(_) => return None,
            };
            hunk.lines.push(DiffLine {
                kind,
                text: text.to_string(),
                no_newline: false,
            });
        }
        Some(diff)
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// The new-side line of the first change, if there are hunks.
    pub fn first_change_line(&self) -> Option<u32> {
        let hunk = self.hunks.first()?;
        Some(hunk.new_line_of(hunk.first_change()))
    }

    /// Where old-side line `line` ends up on the new side, e.g. a line of
    /// the index in the working tree given the unstaged diff. A deleted
    /// line maps to the new-side line that follows it.
    pub fn old_to_new(&self, line: u32) -> u32 {
        let mut shift: i64 = 0;
        for hunk in &self.hunks {
            let first = if hunk.old_count == 0 {
                hunk.old_start + 1
            } else {
                hunk.old_start
            };
            if line < first {
                break;
            }
            let mut old = first;
            for (i, l) in hunk.lines.iter().enumerate() {
                if l.kind == LineKind::Add {
                    continue;
                }
                if old == line {
                    return hunk.new_line_of(i);
                }
                old += 1;
            }
            shift += i64::from(hunk.new_count) - i64::from(hunk.old_count);
        }
        (i64::from(line) + shift).max(1) as u32
    }

    /// A patch holding only the changed lines `selected(hunk, line)` picks,
    /// ready for `git apply` in `direction`, or `None` if it picks no
    /// change. Lines left out stay as they are on the side being patched:
    /// going forward an unpicked deletion becomes context and an unpicked
    /// addition is dropped; in reverse it's the other way round.
    ///
    /// When only part of a new or deleted file is picked, the file exists on
    /// both sides of the result, so the patch says so instead of creating or
    /// deleting it. `always_partial` forces that (for a file just added with
    /// `git add -N`).
    pub fn patch(
        &self,
        selected: impl Fn(usize, usize) -> bool,
        direction: PatchDirection,
        always_partial: bool,
    ) -> Option<String> {
        let forward = direction == PatchDirection::Forward;
        let mut body = String::new();
        let mut whole = !always_partial;
        // How far the side being rewritten has drifted from the original
        // by the hunks emitted so far.
        let mut shift: i64 = 0;
        for (h, hunk) in self.hunks.iter().enumerate() {
            let mut lines = String::new();
            let (mut old, mut new, mut picked) = (0u32, 0u32, false);
            for (l, line) in hunk.lines.iter().enumerate() {
                let pick = line.is_change() && selected(h, l);
                picked |= pick;
                whole &= pick || !line.is_change();
                let keep_as = match (line.kind, pick) {
                    (LineKind::Context, _) => Some(' '),
                    (LineKind::Add, true) => Some('+'),
                    (LineKind::Del, true) => Some('-'),
                    (LineKind::Add, false) => (!forward).then_some(' '),
                    (LineKind::Del, false) => forward.then_some(' '),
                };
                let Some(prefix) = keep_as else { continue };
                match prefix {
                    ' ' => {
                        old += 1;
                        new += 1;
                    }
                    '+' => new += 1,
                    _ => old += 1,
                }
                lines.push(prefix);
                lines.push_str(&line.text);
                lines.push('\n');
                if line.no_newline {
                    lines.push_str("\\ No newline at end of file\n");
                }
            }
            if !picked {
                continue;
            }
            // The side `git apply` matches against keeps its original
            // position; the other moves by what earlier hunks changed.
            let (old_start, new_start) = if forward {
                (hunk.old_start, moved(hunk.old_start, old, new, shift))
            } else {
                (moved(hunk.new_start, new, old, -shift), hunk.new_start)
            };
            shift += i64::from(new) - i64::from(old);
            body.push_str(&heading(old_start, old, new_start, new, &hunk.section));
            body.push('\n');
            body.push_str(&lines);
        }
        if body.is_empty() {
            return None;
        }
        let mut out = String::new();
        for line in self.header_for(whole) {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(&body);
        Some(out)
    }

    /// The header, rewritten for a partial patch of a new or deleted file to
    /// modify it in place instead.
    fn header_for(&self, whole: bool) -> Vec<String> {
        if whole {
            return self.header.clone();
        }
        let old_path = self.header.iter().find_map(|l| l.strip_prefix("--- "));
        let new_path = self.header.iter().find_map(|l| l.strip_prefix("+++ "));
        let swap = |path: &str, from: &str, to: &str| -> String {
            if let Some(rest) = path.strip_prefix(&format!("\"{from}")) {
                format!("\"{to}{rest}")
            } else if let Some(rest) = path.strip_prefix(from) {
                format!("{to}{rest}")
            } else {
                path.to_string()
            }
        };
        self.header
            .iter()
            .filter(|l| !l.starts_with("new file mode ") && !l.starts_with("deleted file mode "))
            .map(|l| match (l.as_str(), old_path, new_path) {
                ("--- /dev/null", _, Some(new)) => format!("--- {}", swap(new, "b/", "a/")),
                ("+++ /dev/null", Some(old), _) => format!("+++ {}", swap(old, "a/", "b/")),
                _ => l.clone(),
            })
            .collect()
    }
}

/// The start line on the side a patch doesn't match against, from the
/// matched side's `start`. `git diff` numbers an empty range by the line
/// before it, hence the adjustments when one side is empty.
fn moved(start: u32, count: u32, other_count: u32, shift: i64) -> u32 {
    let mut s = i64::from(start) + shift;
    if count == 0 && other_count > 0 {
        s += 1;
    } else if count > 0 && other_count == 0 {
        s -= 1;
    }
    s.max(0) as u32
}

fn heading(old_start: u32, old_count: u32, new_start: u32, new_count: u32, section: &str) -> String {
    let mut s = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@");
    if !section.is_empty() {
        s.push(' ');
        s.push_str(section);
    }
    s
}

/// Parses `@@ -a[,b] +c[,d] @@[ section]`.
fn parse_heading(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |r: &str| -> Option<(u32, u32)> {
        match r.split_once(',') {
            Some((s, c)) => Some((s.parse().ok()?, c.parse().ok()?)),
            None => Some((r.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = range(old)?;
    let (new_start, new_count) = range(new)?;
    Some(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: section.strip_prefix(' ').unwrap_or(section).to_string(),
        lines: Vec::new(),
    })
}

/// The file and line a row of diff text (`git show`, `git diff`) is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPosition {
    /// The file's path on the new side, when the text names it.
    pub path: Option<String>,
    /// The file is gone on the new side (`+++ /dev/null`); `path` is its
    /// old path.
    pub deleted: bool,
    /// The new-side line, when the file has hunks.
    pub line: Option<u32>,
}

/// Where row `row` of diff text is: the file whose `diff --git` section
/// holds it (or the first one below it, for a row in a commit's header) and
/// the new-side line. A row that isn't a hunk line (a heading, the file's
/// header) gets the first change of the hunk that follows. `None` when the
/// text has no file sections or hunks at all.
pub fn diff_text_position<S: AsRef<str>>(lines: &[S], row: usize) -> Option<DiffPosition> {
    let line = |i: usize| lines[i].as_ref();
    let is_file = |i: usize| line(i).starts_with("diff --git ");
    let row = row.min(lines.len().checked_sub(1)?);
    let start = (0..=row)
        .rev()
        .find(|&i| is_file(i))
        .or_else(|| (row..lines.len()).find(|&i| is_file(i)))
        .unwrap_or(0);
    let end = (start + 1..lines.len()).find(|&i| is_file(i)).unwrap_or(lines.len());

    // The new path from `+++`, else the old one from `---` for a deletion,
    // else `diff --git a/x b/x` (a binary file or a mode change).
    let mut path = None;
    let mut deleted = false;
    let mut old_path = None;
    for i in start..end {
        let l = line(i);
        if l.starts_with("@@ ") {
            break;
        }
        if let Some(p) = l.strip_prefix("--- ") {
            old_path = Some(diff_path(p, "a/"));
        } else if let Some(p) = l.strip_prefix("+++ ") {
            if p == "/dev/null" {
                deleted = true;
                path = old_path.take();
            } else {
                path = Some(diff_path(p, "b/"));
            }
        }
    }
    if path.is_none() && !deleted && is_file(start) {
        path = same_paths(&line(start)["diff --git ".len()..]);
    }

    let in_section = |i: usize| (start..end).contains(&i);
    let heading = (start..=row)
        .rev()
        .find(|&i| in_section(i) && line(i).starts_with("@@ "))
        .or_else(|| (start..end).find(|&i| line(i).starts_with("@@ ")));
    let mut found = None;
    if let Some(at) = heading
        && let Some(mut hunk) = parse_heading(line(at))
    {
        let mut target = None;
        for i in at + 1..end {
            let kind = match line(i).as_bytes().first() {
                Some(b' ') | None => LineKind::Context,
                Some(b'+') => LineKind::Add,
                Some(b'-') => LineKind::Del,
                Some(b'\\') => continue,
                Some(_) => break,
            };
            if row > at && i >= row && target.is_none() {
                target = Some(hunk.lines.len());
            }
            hunk.lines.push(DiffLine {
                kind,
                text: String::new(),
                no_newline: false,
            });
        }
        let idx = target.unwrap_or_else(|| hunk.first_change());
        found = Some(hunk.new_line_of(idx));
    }
    if path.is_none() && found.is_none() {
        return None;
    }
    Some(DiffPosition {
        path,
        deleted,
        line: found,
    })
}

/// The row of diff text that shows new-side line `line`: an added or
/// context line of the hunk that holds it. `None` when no hunk reaches it.
pub fn diff_text_row_of_line<S: AsRef<str>>(lines: &[S], line: u32) -> Option<usize> {
    let mut new_line = None;
    for (row, text) in lines.iter().enumerate() {
        let text = text.as_ref();
        if text.starts_with("diff --git ") {
            new_line = None;
        } else if text.starts_with("@@ ") {
            new_line = parse_heading(text).map(|h| h.new_start);
        } else if let Some(n) = new_line.as_mut() {
            match text.as_bytes().first() {
                Some(b' ' | b'+') | None => {
                    if *n == line {
                        return Some(row);
                    }
                    *n += 1;
                }
                Some(b'-' | b'\\') => {}
                Some(_) => new_line = None,
            }
        }
    }
    None
}

/// A path from a `---`/`+++` line, unquoted and without its `a/`/`b/` prefix.
fn diff_path(raw: &str, prefix: &str) -> String {
    let p = unquote_c(raw);
    match p.strip_prefix(prefix) {
        Some(rest) => rest.to_string(),
        None => p,
    }
}

/// The path from `a/<p> b/<p>`, when both sides are the same path.
fn same_paths(rest: &str) -> Option<String> {
    let inner = rest.strip_prefix("a/")?;
    let len = inner.len().checked_sub(3)? / 2;
    let (old, new) = (inner.get(..len)?, inner.get(len..)?);
    (new.strip_prefix(" b/")? == old).then(|| old.to_string())
}

/// Undoes git's C-style quoting of a path (`"a\tb\"c"`), leaving unquoted
/// text as it is.
pub fn unquote_c(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return s.to_string();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut it = inner.bytes().peekable();
    while let Some(b) = it.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match it.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'r') => bytes.push(b'\r'),
            Some(b'a') => bytes.push(7),
            Some(b'b') => bytes.push(8),
            Some(b'f') => bytes.push(12),
            Some(b'v') => bytes.push(11),
            Some(d @ b'0'..=b'7') => {
                let mut n = u32::from(d - b'0');
                for _ in 0..2 {
                    match it.peek() {
                        Some(&d @ b'0'..=b'7') => {
                            n = n * 8 + u32::from(d - b'0');
                            it.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(n as u8);
            }
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A diff that adds all of `content` as a new file at `path`, in the shape
/// `git diff` gives a file added with `git add -N`, minus the new-file mode
/// (so patches from it apply once the path is in the index). `None` for
/// binary content.
pub fn untracked_diff(path: &str, content: &[u8]) -> Option<String> {
    if content.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(content).ok()?;
    let mut lines: Vec<&str> = text.split('\n').collect();
    let trailing_newline = text.ends_with('\n');
    if trailing_newline || text.is_empty() {
        lines.pop();
    }
    let mut out = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    if lines.is_empty() {
        return Some(out);
    }
    out.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    for line in &lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if !trailing_newline {
        out.push_str("\\ No newline at end of file\n");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_HUNKS: &str = "\
diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,4 +1,4 @@ fn top
 a
-b
+B
 c
 d
@@ -10,3 +10,4 @@
 x
+y
 z
 w
";

    #[test]
    fn finds_the_row_of_a_new_line() {
        let lines: Vec<&str> = TWO_HUNKS.lines().collect();
        let row = |line| diff_text_row_of_line(&lines, line);
        assert_eq!(row(1), Some(5));
        // A deleted line has no new number: the added line after it is 2.
        assert_eq!(row(2), Some(7));
        assert_eq!(row(4), Some(9));
        assert_eq!(row(11), Some(12));
        assert_eq!(row(13), Some(14));
        // Between the hunks and past the end, no hunk shows it.
        assert_eq!(row(6), None);
        assert_eq!(row(14), None);
    }

    #[test]
    fn parses_header_and_hunks() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        assert_eq!(d.header.len(), 4);
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.hunks[0].section, "fn top");
        assert_eq!(d.hunks[0].heading(), "@@ -1,4 +1,4 @@ fn top");
        assert_eq!(d.hunks[1].lines.len(), 4);
        assert_eq!(d.hunks[1].lines[1].kind, LineKind::Add);
    }

    #[test]
    fn refuses_binary_and_combined_diffs() {
        assert!(FileDiff::parse("diff --git a/x b/x\nBinary files a/x and b/x differ\n").is_none());
        assert!(FileDiff::parse("diff --cc f\n@@@ -1,1 -1,1 +1,1 @@@\n").is_none());
    }

    #[test]
    fn second_hunk_alone_moves_its_new_start() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        let p = d.patch(|h, _| h == 1, PatchDirection::Forward, false).unwrap();
        assert!(p.contains("@@ -10,3 +10,4 @@\n x\n+y\n z\n w\n"), "{p}");
        assert!(!p.contains("+B"));
    }

    #[test]
    fn forward_drops_unpicked_additions_and_keeps_deletions_as_context() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        // Only `-b` of the first hunk.
        let p = d
            .patch(|h, l| h == 0 && l == 1, PatchDirection::Forward, false)
            .unwrap();
        assert!(p.ends_with("@@ -1,4 +1,3 @@ fn top\n a\n-b\n c\n d\n"), "{p}");
        // Only `+B`.
        let p = d
            .patch(|h, l| h == 0 && l == 2, PatchDirection::Forward, false)
            .unwrap();
        assert!(p.ends_with("@@ -1,4 +1,5 @@ fn top\n a\n b\n+B\n c\n d\n"), "{p}");
    }

    #[test]
    fn reverse_keeps_unpicked_additions_as_context() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        let p = d
            .patch(|h, l| h == 0 && l == 1, PatchDirection::Reverse, false)
            .unwrap();
        assert!(p.ends_with("@@ -1,5 +1,4 @@ fn top\n a\n-b\n B\n c\n d\n"), "{p}");
        let p = d
            .patch(|h, l| h == 0 && l == 2, PatchDirection::Reverse, false)
            .unwrap();
        assert!(p.ends_with("@@ -1,3 +1,4 @@ fn top\n a\n+B\n c\n d\n"), "{p}");
    }

    #[test]
    fn nothing_picked_is_no_patch() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        assert!(d.patch(|_, _| false, PatchDirection::Forward, false).is_none());
    }

    #[test]
    fn keeps_crlf_and_missing_newline_markers() {
        let text = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\r\n\\ No newline at end of file\n+b\r\n";
        let d = FileDiff::parse(text).unwrap();
        assert_eq!(d.hunks[0].lines[0].text, "a\r");
        assert!(d.hunks[0].lines[0].no_newline);
        let p = d.patch(|_, _| true, PatchDirection::Forward, false).unwrap();
        assert!(
            p.ends_with("@@ -1,1 +1,1 @@\n-a\r\n\\ No newline at end of file\n+b\r\n"),
            "{p:?}"
        );
    }

    #[test]
    fn partial_new_file_is_patched_in_place() {
        let text = "diff --git a/n b/n\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+one\n+two\n";
        let d = FileDiff::parse(text).unwrap();
        let whole = d.patch(|_, _| true, PatchDirection::Forward, false).unwrap();
        assert!(whole.contains("new file mode") && whole.contains("--- /dev/null"));
        let part = d.patch(|_, l| l == 0, PatchDirection::Forward, false).unwrap();
        assert!(!part.contains("new file mode"), "{part}");
        assert!(part.contains("--- a/n\n+++ b/n\n@@ -0,0 +1,1 @@\n+one\n"), "{part}");
        let forced = d.patch(|_, _| true, PatchDirection::Forward, true).unwrap();
        assert!(forced.contains("--- a/n\n"), "{forced}");
    }

    #[test]
    fn partial_deletion_keeps_the_file() {
        let text =
            "diff --git a/d b/d\ndeleted file mode 100644\n--- a/d\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-one\n-two\n";
        let d = FileDiff::parse(text).unwrap();
        let part = d.patch(|_, l| l == 1, PatchDirection::Forward, false).unwrap();
        assert!(
            part.contains("--- a/d\n+++ b/d\n@@ -1,2 +1,1 @@\n one\n-two\n"),
            "{part}"
        );
    }

    #[test]
    fn untracked_content_as_a_diff() {
        assert_eq!(
            untracked_diff("n", b"a\nb\n").unwrap(),
            "diff --git a/n b/n\n--- a/n\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n"
        );
        assert!(
            untracked_diff("n", b"a")
                .unwrap()
                .ends_with("+a\n\\ No newline at end of file\n")
        );
        assert!(untracked_diff("n", b"a\0b").is_none());
        let d = FileDiff::parse(&untracked_diff("n", b"").unwrap()).unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn new_side_lines_of_hunk_lines() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        let lines: Vec<u32> = (0..5).map(|i| d.hunks[0].new_line_of(i)).collect();
        // a, -b (takes B's line), +B, c, d
        assert_eq!(lines, [1, 2, 2, 3, 4]);
        assert_eq!(d.first_change_line(), Some(2));
        let gone = FileDiff::parse("--- a/f\n+++ b/f\n@@ -5,2 +4,0 @@\n-x\n-y\n").unwrap();
        assert_eq!(gone.first_change_line(), Some(5));
        let emptied = FileDiff::parse("--- a/f\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n").unwrap();
        assert_eq!(emptied.first_change_line(), Some(1));
    }

    #[test]
    fn old_lines_map_through_the_hunks() {
        let d = FileDiff::parse(TWO_HUNKS).unwrap();
        let mapped: Vec<u32> = [1, 2, 3, 5, 10, 11, 12, 20].iter().map(|&l| d.old_to_new(l)).collect();
        assert_eq!(mapped, [1, 2, 3, 5, 10, 12, 13, 21]);
        let inserted = FileDiff::parse("--- a/f\n+++ b/f\n@@ -3,0 +4,2 @@\n+p\n+q\n").unwrap();
        assert_eq!([3, 4].map(|l| inserted.old_to_new(l)), [3, 6]);
        let deleted = FileDiff::parse("--- a/f\n+++ b/f\n@@ -5,2 +4,0 @@\n-x\n-y\n").unwrap();
        assert_eq!([4, 5, 6, 7].map(|l| deleted.old_to_new(l)), [4, 5, 5, 5]);
    }

    const SHOW: &str = r#"commit abc
Author: me

    subject

diff --git a/one.txt b/one.txt
index 1111111..2222222 100644
--- a/one.txt
+++ b/one.txt
@@ -1,3 +1,3 @@
 a
-b
+B
 c
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-x
diff --git "a/sp\"q.txt" "b/sp\"q.txt"
--- "a/sp\"q.txt"
+++ "b/sp\"q.txt"
@@ -5,2 +5,3 @@
 m
+n
 o
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
"#;

    #[test]
    fn rows_of_diff_text_map_to_files_and_lines() {
        let lines: Vec<&str> = SHOW.lines().collect();
        let at = |row: usize| diff_text_position(&lines, row).unwrap();
        let pos = |path: &str, line: Option<u32>| DiffPosition {
            path: Some(path.into()),
            deleted: false,
            line,
        };
        // The commit header and a file's header go to its first change.
        assert_eq!(at(0), pos("one.txt", Some(2)));
        assert_eq!(at(7), pos("one.txt", Some(2)));
        assert_eq!(at(9), pos("one.txt", Some(2)));
        assert_eq!(at(10), pos("one.txt", Some(1)));
        assert_eq!(at(11), pos("one.txt", Some(2)));
        assert_eq!(at(13), pos("one.txt", Some(3)));
        assert_eq!(
            at(19),
            DiffPosition {
                path: Some("gone.txt".into()),
                deleted: true,
                line: Some(1),
            }
        );
        assert_eq!(at(25), pos("sp\"q.txt", Some(6)));
        assert_eq!(at(27), pos("img.png", None));
        assert_eq!(at(1000), pos("img.png", None));
        assert_eq!(diff_text_position(&["just text", "here"], 1), None);
        assert_eq!(diff_text_position::<&str>(&[], 0), None);
    }

    #[test]
    fn unquotes_c_style_paths() {
        assert_eq!(unquote_c("plain"), "plain");
        assert_eq!(unquote_c(r#""a\tb\"c\\d""#), "a\tb\"c\\d");
        assert_eq!(unquote_c(r#""\303\244.txt""#), "ä.txt");
    }
}
