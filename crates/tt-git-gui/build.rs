//! Embeds the app icon in the Windows executable: GPUI gives each window
//! the executable's icon resource 1. Other platforms get no resource.

fn main() {
    println!("cargo:rerun-if-changed=assets/tt-git.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/tt-git.rc", embed_resource::ParamsIncludeDirs(["assets"]))
            .manifest_optional()
            .expect("embedding the icon resource");
    }
}
