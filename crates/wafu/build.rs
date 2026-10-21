fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config)
        .expect("failed to compile ui/app.slint");
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/wafu.ico")
            .compile()
            .expect("failed to embed the Windows icon");
    }
}

#[cfg(not(windows))]
fn embed_windows_icon() {}
