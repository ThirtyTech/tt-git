//! Embeds the desktop app's icon and a version resource in the Windows
//! executable, so Explorer and the taskbar show tt-git.exe with the icon and
//! its Properties show the version. Other platforms get no resource.

#[path = "../tt-git-gui/assets/resource.rs"]
mod resource;

fn main() {
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/tt-git.rc");
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/version.rc");
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/resource.rs");
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out = resource::write_version_header("tt-git.exe", "tt-git terminal app");
        embed_resource::compile(
            "../tt-git-gui/assets/tt-git.rc",
            embed_resource::ParamsIncludeDirs(["../tt-git-gui/assets".into(), out]),
        )
        .manifest_optional()
        .expect("embedding the icon and version resource");
    }
}
