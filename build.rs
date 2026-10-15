fn main() {
    // Use Slint's built-in cupertino style (macOS look, follows system light/dark mode).
    // Note: Slint's "native" style depends on Qt, so we use the pure-Rust cupertino style instead.
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino".into());
    slint_build::compile_with_config("ui/app-window.slint", config).expect("Slint build failed");
}
