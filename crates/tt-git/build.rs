//! Embeds the desktop app's icon in the Windows executable, so Explorer and
//! the taskbar show tt-git.exe with it too. Other platforms get no resource.

fn main() {
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/tt-git.rc");
    println!("cargo:rerun-if-changed=../tt-git-gui/assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile(
            "../tt-git-gui/assets/tt-git.rc",
            embed_resource::ParamsIncludeDirs(["../tt-git-gui/assets"]),
        )
        .manifest_optional()
        .expect("embedding the icon resource");
    }
}
