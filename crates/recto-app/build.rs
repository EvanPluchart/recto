fn main() {
    // On Windows, the executable's icon: resource 1, which GPUI uses for the window and the
    // taskbar. The icon is a copy of `assets/brand/recto.ico`.
    println!("cargo:rerun-if-changed=assets/recto.ico");
    println!("cargo:rerun-if-changed=app.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("app.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
