fn main() {
    #[cfg(feature = "logos-core")]
    {
        println!("cargo:rustc-link-lib=logos_core");
        if let Ok(dir) = std::env::var("LOGOS_CORE_LIB_DIR") {
            println!("cargo:rustc-link-search={}", dir);
        }
    }
}
