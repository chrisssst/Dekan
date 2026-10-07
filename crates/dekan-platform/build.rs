fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_debug_info(std::env::var("PROFILE").as_deref() == Ok("debug"))
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("the Slint interface does not compile: {e}");
    }
}
