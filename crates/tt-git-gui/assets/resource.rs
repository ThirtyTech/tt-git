//! Shared by the build scripts of tt-git and tt-git-gui, which both embed
//! tt-git.rc. Writes the version.h that version.rc reads.

use std::path::{Path, PathBuf};

/// Writes version.h into OUT_DIR for an exe described as `description`, and
/// returns the directory to add to the resource compiler's include dirs.
pub fn write_version_header(exe: &str, description: &str) -> PathBuf {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("version.h"), version_header(exe, description)).expect("writing version.h");
    out
}

fn version_header(exe: &str, description: &str) -> String {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
    // Windows file versions are four numbers, so a pre-release suffix only
    // shows in the FileVersion and ProductVersion strings.
    let numbers = format!(
        "{},{},{},0",
        var("CARGO_PKG_VERSION_MAJOR"),
        var("CARGO_PKG_VERSION_MINOR"),
        var("CARGO_PKG_VERSION_PATCH")
    );
    let prerelease = !var("CARGO_PKG_VERSION_PRE").is_empty();
    let name = Path::new(exe).file_stem().unwrap().to_str().unwrap();
    format!(
        "#define TT_VERSION_NUMBERS {numbers}\n\
         #define TT_VERSION \"{}\"\n\
         #define TT_FILE_FLAGS {}\n\
         #define TT_FILE_DESCRIPTION \"{description}\"\n\
         #define TT_INTERNAL_NAME \"{name}\"\n\
         #define TT_ORIGINAL_FILENAME \"{exe}\"\n",
        var("CARGO_PKG_VERSION"),
        if prerelease { "VS_FF_PRERELEASE" } else { "0" },
    )
}
