use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // If LOGOS_CORE_LIB_DIR is already set (pointing to real SDK or pre-built stub),
    // skip building the stub — just let the transport/storage build.rs handle linking.
    if env::var("LOGOS_CORE_LIB_DIR").is_ok() {
        return;
    }

    // Build the stub liblogos_core.so from C source
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let stub_dir = manifest_dir.join("stub");
    let stub_src = stub_dir.join("logos_core_stub.c");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let stub_so = out_dir.join("liblogos_core.so");

    let status = Command::new("cc")
        .args([
            "-shared",
            "-fPIC",
            "-o",
            stub_so.to_str().unwrap(),
            stub_src.to_str().unwrap(),
            "-lpthread",
        ])
        .status()
        .expect("failed to compile stub liblogos_core.so — is cc installed?");

    assert!(status.success(), "stub compilation failed");

    // Tell cargo (and the transport/storage build.rs) where to find the stub
    println!("cargo:rustc-link-search={}", out_dir.display());

    // Re-run if stub source changes
    println!("cargo:rerun-if-changed=stub/logos_core_stub.c");
}
