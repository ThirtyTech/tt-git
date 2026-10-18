//! Every TOML example in docs/configuration.md must be a valid config.

use tt_git_tui::Config;

#[test]
fn configuration_doc_examples_parse() {
    let doc = include_str!("../../../docs/configuration.md");
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in doc.lines() {
        match (&mut current, line.trim_start()) {
            (None, l) if l.starts_with("```toml") => current = Some(String::new()),
            (Some(_), l) if l.starts_with("```") => blocks.push(current.take().unwrap()),
            (Some(buf), _) => {
                buf.push_str(line);
                buf.push('\n');
            }
            _ => {}
        }
    }
    assert!(blocks.len() >= 3, "found {} toml blocks", blocks.len());
    for block in blocks {
        let cfg = Config::parse(&block).unwrap_or_else(|e| panic!("{e}\n---\n{block}"));
        cfg.keymap().unwrap_or_else(|e| panic!("{e}\n---\n{block}"));
    }
}
