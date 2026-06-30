use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let netd_manifest = manifest.join("..").join("chimera-netd").join("Cargo.toml");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let netd_target = out.join("netd-build");

    // Build chimera-netd (release) into a dedicated target dir to avoid lock
    // contention with the outer build.
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "build",
            "--release",
            "--manifest-path",
            netd_manifest.to_str().unwrap(),
            "--target-dir",
            netd_target.to_str().unwrap(),
        ])
        .status()
        .expect("run cargo build for chimera-netd");
    assert!(status.success(), "failed to build chimera-netd");

    let bin = netd_target.join("release").join("chimera-netd");
    println!("cargo:rustc-env=CHIMERA_NETD_BIN={}", bin.display());
    println!("cargo:rerun-if-changed=../chimera-netd/src");
    println!("cargo:rerun-if-changed=../chimera-netd/Cargo.toml");
}
