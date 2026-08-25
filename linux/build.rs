//! Stamps the real build version into the binary: git-described for
//! local builds (v0.0.6-3-gabc123, with -dirty when the tree is), the
//! tag name in CI when git cannot describe a shallow checkout, the
//! crate version as the last resort.
fn main() {
    let described = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|version| !version.is_empty());
    let version = described
        .or_else(|| {
            std::env::var("GITHUB_REF_NAME")
                .ok()
                .filter(|name| name.starts_with('v'))
        })
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    println!("cargo:rustc-env=TEXTCHUM_BUILD_VERSION={version}");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
}
