fn main() {
    println!("cargo:rerun-if-changed=src/coreml_bridge.m");
    if std::env::var_os("CARGO_FEATURE_COREML").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
    {
        cc::Build::new()
            .file("src/coreml_bridge.m")
            .flag("-fobjc-arc")
            .flag("-mmacosx-version-min=15.0")
            .compile("craig_coreml");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreML");
    }
}
