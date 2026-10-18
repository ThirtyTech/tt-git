//! The example plugins shipped in `examples/plugins` must stay valid.

use std::path::Path;

#[test]
fn example_manifests_are_valid() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("examples/plugins exists") {
        let path = entry.unwrap().path();
        if !path.join(tt_git_plugin::MANIFEST_FILE).exists() {
            continue;
        }
        let loaded = tt_git_plugin::manifest::load(&path, &tt_git_plugin::host_version())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(loaded.warnings.is_empty(), "{}: {:?}", path.display(), loaded.warnings);
        assert!(loaded.manifest.id.starts_with("examples."));
        count += 1;
    }
    assert!(count >= 3, "expected the example plugins, found {count}");
}
