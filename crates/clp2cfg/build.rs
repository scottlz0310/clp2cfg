//! Windows のアイコンとファイルバージョン。
//!
//! リリースの版は `clp2cfg-cli` の `Cargo.toml` だけが持つ（release-automate が直接の
//! `[package].version` を 1 つだけ書き換えるため）。この crate の版は使えないので、そこから読む。

#[cfg(windows)]
const VERSION_MANIFEST: &str = "../clp2cfg-cli/Cargo.toml";

/// `[package]` の `version = "x.y.z"` を読む
#[cfg(windows)]
fn package_version(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            let value = line
                .strip_prefix("version")
                .and_then(|rest| rest.trim_start().strip_prefix('='));
            if let Some(value) = value {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// `major.minor.patch` を、Windows のバージョン情報の 64 ビット値（各 16 ビット）にする
#[cfg(windows)]
fn packed_version(version: &str) -> Option<u64> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    Some((major << 48) | (minor << 32) | (patch << 16))
}

#[cfg(windows)]
fn main() {
    println!("cargo:rerun-if-changed={VERSION_MANIFEST}");
    let manifest = std::fs::read_to_string(VERSION_MANIFEST)
        .unwrap_or_else(|e| panic!("failed to read {VERSION_MANIFEST}: {e}"));
    let version = package_version(&manifest)
        .unwrap_or_else(|| panic!("no [package].version in {VERSION_MANIFEST}"));
    let packed = packed_version(&version)
        .unwrap_or_else(|| panic!("version is not major.minor.patch: {version}"));

    let mut res = winres::WindowsResource::new();
    res.set_icon("../../assets/icon.ico");
    res.set("FileVersion", &version);
    res.set("ProductVersion", &version);
    res.set_version_info(winres::VersionInfo::FILEVERSION, packed);
    res.set_version_info(winres::VersionInfo::PRODUCTVERSION, packed);
    if let Err(e) = res.compile() {
        eprintln!("Failed to compile Windows resource: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}
