//! Syntax highlighting for the code in blame and diffs, with syntect and
//! bat's syntaxes (via two-face). A line comes back as byte ranges of a few
//! classes (comment, string, keyword…); the theme styles each class.
//!
//! Highlighting is slow next to drawing (a few hundred lines per
//! millisecond), so the loaders do it alongside the git command and the
//! panes keep the result.

use std::ops::Range;
use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use tt_git_core::patch::{FileDiff, LineKind};

use crate::widgets::expand_tabs;

/// Files with more lines than this aren't highlighted.
const MAX_LINES: usize = 20_000;
/// Lines longer than this (minified code, data) aren't highlighted.
const MAX_LINE_BYTES: usize = 2_000;

/// What a piece of code is, as far as its color goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Comment,
    String,
    Keyword,
    Operator,
    Type,
    Function,
    Number,
    /// `true`, `None`, escapes in strings, constants.
    Constant,
    Macro,
    /// An HTML tag name, a YAML key.
    Tag,
    /// An HTML attribute, a CSS property, a JSON key.
    Attribute,
    /// Markdown.
    Heading,
    Link,
    Strong,
    Emphasis,
}

/// One line's highlighted byte ranges, in order. The gaps are plain text.
pub type Tokens = Vec<(Range<usize>, Class)>;

/// Each line's tokens, `None` for a line that isn't code. Empty when
/// nothing is highlighted.
pub type LineTokens = Vec<Option<Tokens>>;

/// A diff's tokens, hunk by hunk. Empty when nothing is highlighted.
pub type HunkTokens = Vec<LineTokens>;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// TextMate scope prefixes and their class. For each scope, innermost
/// first, the first rule that matches wins, so specific rules come before
/// the general ones they'd otherwise fall under.
fn rules() -> &'static [(Scope, Class)] {
    static RULES: OnceLock<Vec<(Scope, Class)>> = OnceLock::new();
    RULES.get_or_init(|| {
        use Class::*;
        [
            ("comment", Comment),
            ("punctuation.definition.comment", Comment),
            ("string", String),
            ("punctuation.definition.string", String),
            ("constant.character.escape", Constant),
            ("constant.character", String),
            ("constant.numeric", Number),
            ("constant", Constant),
            ("support.constant", Constant),
            ("variable.language", Keyword),
            ("keyword.operator", Operator),
            ("keyword", Keyword),
            ("storage", Keyword),
            ("support.type.property-name", Attribute),
            ("entity.other.attribute-name", Attribute),
            ("entity.name.tag", Tag),
            ("entity.name.function", Function),
            ("support.function", Function),
            ("variable.function", Function),
            ("support.macro", Macro),
            ("entity.name.macro", Macro),
            ("entity.name.section", Heading),
            ("markup.heading", Heading),
            ("entity.name", Type),
            ("entity.other.inherited-class", Type),
            ("support.type", Type),
            ("support.class", Type),
            ("markup.underline.link", Link),
            ("markup.raw", String),
            ("markup.bold", Strong),
            ("markup.italic", Emphasis),
            ("markup.quote", Comment),
        ]
        .into_iter()
        .map(|(scope, class)| (Scope::new(scope).expect("a valid scope"), class))
        .collect()
    })
}

fn classify(stack: &ScopeStack) -> Option<Class> {
    static JSON_KEY: OnceLock<Scope> = OnceLock::new();
    let json_key = *JSON_KEY.get_or_init(|| Scope::new("meta.mapping.key.json").expect("a valid scope"));
    let rules = rules();
    let class = stack.as_slice().iter().rev().find_map(|scope| {
        rules
            .iter()
            .find(|(rule, _)| rule.is_prefix_of(*scope))
            .map(|(_, class)| *class)
    });
    // A JSON key is a string, but reads better as a key, as in YAML.
    if class == Some(Class::String) && stack.as_slice().contains(&json_key) {
        return Some(Class::Attribute);
    }
    class
}

/// The language of the file at `path`, by its name or extension, then by
/// `first_line` (a shebang). `None` when it's one we don't know.
fn language(path: &str, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, ext)| ext);
    set.find_syntax_by_extension(name)
        .or_else(|| ext.and_then(|e| set.find_syntax_by_extension(e)))
        .or_else(|| first_line.and_then(|l| set.find_syntax_by_first_line(l)))
        .filter(|s| s.name != "Plain Text")
}

/// Highlights one file's lines in order, carrying what's open (a block
/// comment, a string) from each line to the next.
struct Highlighter {
    language: &'static SyntaxReference,
    state: ParseState,
    stack: ScopeStack,
    buf: String,
    /// syntect gave up on this file: the rest stays plain.
    failed: bool,
}

impl Highlighter {
    fn new(language: &'static SyntaxReference) -> Self {
        Self {
            language,
            state: ParseState::new(language),
            stack: ScopeStack::new(),
            buf: String::new(),
            failed: false,
        }
    }

    /// Starts over as if at the top of the file.
    fn reset(&mut self) {
        self.state = ParseState::new(self.language);
        self.stack = ScopeStack::new();
    }

    fn line(&mut self, text: &str) -> Option<Tokens> {
        if self.failed || text.len() > MAX_LINE_BYTES {
            return None;
        }
        self.buf.clear();
        self.buf.push_str(text);
        self.buf.push('\n');
        let Ok(ops) = self.state.parse_line(&self.buf, syntaxes()) else {
            self.failed = true;
            return None;
        };
        let mut tokens = Tokens::new();
        let mut at = 0;
        let mut class = classify(&self.stack);
        for (pos, op) in ops {
            let pos = pos.min(text.len());
            push(&mut tokens, at..pos, class);
            if self.stack.apply(&op).is_err() {
                self.failed = true;
                return None;
            }
            class = classify(&self.stack);
            at = pos;
        }
        push(&mut tokens, at..text.len(), class);
        Some(tokens)
    }
}

/// Adds `range` to `tokens`, joined to the last one if it carries on with
/// the same class.
fn push(tokens: &mut Tokens, range: Range<usize>, class: Option<Class>) {
    let Some(class) = class else { return };
    if range.is_empty() {
        return;
    }
    match tokens.last_mut() {
        Some((last, c)) if *c == class && last.end == range.start => last.end = range.end,
        _ => tokens.push((range, class)),
    }
}

/// Highlights a whole file, `path` being its name, `lines` its lines in
/// order (tabs already expanded, as they're shown).
pub fn file<S: AsRef<str>>(path: &str, lines: &[S]) -> LineTokens {
    if lines.len() > MAX_LINES {
        return Vec::new();
    }
    let Some(language) = language(path, lines.first().map(AsRef::as_ref)) else {
        return Vec::new();
    };
    let mut h = Highlighter::new(language);
    lines.iter().map(|l| h.line(l.as_ref())).collect()
}

/// A diff's two sides, each highlighted on its own: the old one from its
/// context and deleted lines, the new one from its context and added
/// lines. A comment opened on a deleted line doesn't swallow the added
/// lines after it. Each hunk starts afresh, since what's between hunks
/// isn't known.
struct DiffHighlighter {
    old: Highlighter,
    new: Highlighter,
}

impl DiffHighlighter {
    fn new(language: &'static SyntaxReference) -> Self {
        Self {
            old: Highlighter::new(language),
            new: Highlighter::new(language),
        }
    }

    fn hunk(&mut self) {
        self.old.reset();
        self.new.reset();
    }

    fn line(&mut self, kind: LineKind, text: &str) -> Option<Tokens> {
        match kind {
            LineKind::Context => {
                self.old.line(text);
                self.new.line(text)
            }
            LineKind::Add => self.new.line(text),
            LineKind::Del => self.old.line(text),
        }
    }
}

/// Highlights the hunks in `git diff` or `git show` output, one file or
/// several, picking each file's language from its `+++` (or `---`) path.
/// A hunk line's tokens cover its text after the `+`, `-` or space.
pub fn diff_text<S: AsRef<str>>(lines: &[S]) -> LineTokens {
    let mut out = Vec::with_capacity(lines.len());
    let mut file: Option<DiffHighlighter> = None;
    let mut old_path = None;
    let mut in_hunk = false;
    for line in lines {
        let line = line.as_ref();
        if in_hunk && let Some(h) = &mut file {
            let kind = match line.as_bytes().first() {
                Some(b' ') => Some(LineKind::Context),
                Some(b'+') => Some(LineKind::Add),
                Some(b'-') => Some(LineKind::Del),
                _ => None,
            };
            if let Some(kind) = kind {
                out.push(h.line(kind, &line[1..]));
                continue;
            }
            // `\ No newline at end of file` and the hunk carries on.
            if line.starts_with('\\') {
                out.push(None);
                continue;
            }
        }
        in_hunk = false;
        if line.starts_with("diff ") {
            file = None;
            old_path = None;
        } else if let Some(path) = line.strip_prefix("--- ") {
            old_path = Some(path);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            let path = if path == "/dev/null" { old_path } else { Some(path) };
            file = path
                .and_then(|p| language(p.trim_end_matches(['\t', '"']), None))
                .map(DiffHighlighter::new);
        } else if line.starts_with("@@ ")
            && let Some(h) = &mut file
        {
            h.hunk();
            in_hunk = true;
        }
        out.push(None);
    }
    if out.iter().all(Option::is_none) {
        out.clear();
    }
    out
}

/// Highlights `path`'s diff hunk by hunk, both sides as in [`diff_text`]:
/// for each hunk, its lines' tokens over [`shown`] text.
pub fn hunks(path: &str, diff: &FileDiff) -> HunkTokens {
    let Some(language) = language(path, None) else {
        return Vec::new();
    };
    let mut h = DiffHighlighter::new(language);
    diff.hunks
        .iter()
        .map(|hunk| {
            h.hunk();
            hunk.lines.iter().map(|l| h.line(l.kind, &shown(&l.text))).collect()
        })
        .collect()
}

/// A diff line's text as the staging view shows it: no CR, tabs expanded.
pub fn shown(text: &str) -> std::borrow::Cow<'_, str> {
    expand_tabs(text.strip_suffix('\r').unwrap_or(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each token in `line` of `path`, with its class.
    fn classes(path: &str, text: &str) -> Vec<Vec<(String, Class)>> {
        let lines: Vec<&str> = text.lines().collect();
        let tokens = file(path, &lines);
        assert_eq!(tokens.len(), lines.len(), "{path} isn't highlighted");
        lines
            .iter()
            .zip(tokens)
            .map(|(line, tokens)| {
                tokens
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(r, c)| (line[r].to_string(), c))
                    .collect()
            })
            .collect()
    }

    fn has(tokens: &[Vec<(String, Class)>], text: &str, class: Class) -> bool {
        tokens.iter().flatten().any(|(t, c)| t.trim() == text && *c == class)
    }

    /// A file name, its text, and tokens it must have with their class.
    type Case = (&'static str, &'static str, &'static [(&'static str, Class)]);

    /// Each primary language: a keyword, a string and a comment, at least.
    #[test]
    fn highlights_the_primary_languages() {
        use Class::*;
        let cases: &[Case] = &[
            (
                "src/main.rs",
                "// hi\nfn main() { println!(\"x\"); let n = 42; }",
                &[
                    ("// hi", Comment),
                    ("fn", Keyword),
                    ("main", Function),
                    ("\"x\"", String),
                    ("42", Number),
                ],
            ),
            (
                "app.ts",
                "// hi\nexport const x: number = 1;\nfunction f() { return 'y'; }",
                &[
                    ("// hi", Comment),
                    ("export", Keyword),
                    ("'y'", String),
                    ("f", Function),
                ],
            ),
            (
                "app.tsx",
                "const a = <div className=\"x\">hi</div>; // c",
                &[("div", Tag), ("className", Attribute), ("// c", Comment)],
            ),
            (
                "main.go",
                "package main\n// hi\nfunc main() { s := \"x\" }",
                &[
                    ("package", Keyword),
                    ("// hi", Comment),
                    ("main", Function),
                    ("\"x\"", String),
                ],
            ),
            (
                "README.md",
                "# Title\n\nSome **bold** and `code`.",
                &[("# Title", Heading), ("**bold**", Strong), ("`code`", String)],
            ),
            (
                "site.css",
                "/* hi */\n.a { color: red; margin: 4px; }",
                &[("/* hi */", Comment), ("color", Attribute), ("4", Number)],
            ),
            (
                "index.html",
                "<!-- hi -->\n<a href=\"x\">link</a>",
                &[
                    ("<!-- hi -->", Comment),
                    ("a", Tag),
                    ("href", Attribute),
                    ("\"x\"", String),
                ],
            ),
            (
                "Program.cs",
                "// hi\npublic class Foo { string s = \"x\"; }",
                &[
                    ("// hi", Comment),
                    ("public", Keyword),
                    ("Foo", Type),
                    ("\"x\"", String),
                ],
            ),
            (
                "main.dart",
                "// hi\nvoid main() { final s = 'x'; }",
                &[("// hi", Comment), ("final", Keyword), ("'x'", String)],
            ),
            (
                "ci.yml",
                "# hi\nname: build\non: [push]\ncount: 3",
                &[("# hi", Comment), ("name", Tag), ("build", String), ("3", Number)],
            ),
            (
                "package.json",
                "{\"name\": \"x\", \"n\": 1, \"ok\": true}",
                &[
                    ("\"name\"", Attribute),
                    ("\"x\"", String),
                    ("1", Number),
                    ("true", Constant),
                ],
            ),
        ];
        for (path, text, want) in cases {
            let got = classes(path, text);
            for (token, class) in *want {
                assert!(has(&got, token, *class), "{path}: no {token:?} as {class:?} in {got:?}");
            }
        }
    }

    #[test]
    fn carries_block_comments_across_lines() {
        let got = classes("a.rs", "/* one\ntwo */ fn x() {}");
        assert_eq!(got[1][0], ("two */".to_string(), Class::Comment));
        assert!(has(&got, "fn", Class::Keyword));
    }

    #[test]
    fn leaves_unknown_files_and_huge_lines_plain() {
        assert!(file("notes.unknownext", &["fn main() {}"]).is_empty());
        let long = format!("let s = \"{}\";", "x".repeat(MAX_LINE_BYTES));
        assert_eq!(file("a.rs", &[long.as_str(), "fn x() {}"])[0], None);
        let many = vec!["x"; MAX_LINES + 1];
        assert!(file("a.rs", &many).is_empty());
    }

    #[test]
    fn picks_a_language_by_shebang() {
        assert!(!file("bin/run", &["#!/usr/bin/env bash", "echo hi"]).is_empty());
    }

    #[test]
    fn diff_sides_are_highlighted_on_their_own() {
        let text = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
-/* old comment
+fn new() {}
 let x = 1;
\\ No newline at end of file
";
        let lines: Vec<&str> = text.lines().collect();
        let tokens = diff_text(&lines);
        assert_eq!(tokens.len(), lines.len());
        assert!(tokens[..4].iter().all(Option::is_none));
        // The deleted line opens a comment on the old side only.
        assert_eq!(tokens[4], Some(vec![(0..14, Class::Comment)]));
        assert_eq!(tokens[5].as_ref().unwrap()[0], (0..2, Class::Keyword));
        // Context goes on both sides; the new side's is shown.
        assert_eq!(tokens[6].as_ref().unwrap()[0], (0..3, Class::Keyword));
        assert_eq!(tokens[7], None);
    }

    #[test]
    fn diff_text_follows_each_files_language() {
        let text = "\
commit abc
    fn not_code() {}

 a.rs | 1 +
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
+fn plain() {}
diff --git a/b.rs b/b.rs
deleted file mode 100644
--- a/b.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn gone() {}
";
        let lines: Vec<&str> = text.lines().collect();
        let tokens = diff_text(&lines);
        let highlighted: Vec<usize> = (0..lines.len()).filter(|&i| tokens[i].is_some()).collect();
        assert_eq!(highlighted, [14]);
        assert!(diff_text(&["no diff here"]).is_empty());
    }
}
