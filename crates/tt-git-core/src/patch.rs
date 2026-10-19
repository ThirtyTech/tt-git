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
}
