//! Embeds the app icon and a version resource in the Windows executable:
//! GPUI gives each window the executable's icon resource 1, and Explorer's
//! Properties show the version. Other platforms get no resource.

#[path = "assets/resource.rs"]
mod resource;

fn main() {
    println!("cargo:rerun-if-changed=assets/tt-git.rc");
    println!("cargo:rerun-if-changed=assets/version.rc");
    println!("cargo:rerun-if-changed=assets/resource.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out = resource::write_version_header("tt-git-gui.exe", "tt-git desktop app");
        embed_resource::compile(
            "assets/tt-git.rc",
            embed_resource::ParamsIncludeDirs(["assets".into(), out]),
        )
        .manifest_optional()
        .expect("embedding the icon and version resource");
    }
}
