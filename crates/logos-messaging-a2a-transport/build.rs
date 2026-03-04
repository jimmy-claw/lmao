fn main() {
    #[cfg(feature = "logos-core")]
    {
        println!("cargo:rustc-link-lib=logos_core");
        if let Ok(dir) = std::env::var("LOGOS_CORE_LIB_DIR") {
            println!("cargo:rustc-link-search={}", dir);
        }
    }

    // When native-waku feature is enabled, compile RLN stub symbols.
    // libwaku (Nim) references RLN functions even when RLN is not used.
    // These stubs satisfy the linker without pulling in the full zerokit build.
    #[cfg(feature = "native-waku")]
    {
        cc::Build::new()
            .file("stubs/rln_stubs.c")
            .compile("rln_stubs");
    }
}
