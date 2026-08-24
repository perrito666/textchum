//! Regenerates the C header consumed by platform shells.
//!
//! The header is committed (shells build without a Rust toolchain present in
//! their editor tooling), but it is always regenerated from source here so it
//! can never drift silently: CI fails if a build leaves the tree dirty.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let header = crate_dir.join("../../macos/Sources/CTextchum/include/textchum.h");

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    cbindgen::generate(&crate_dir)
        .expect("cbindgen failed to generate textchum.h")
        .write_to_file(header);
}
