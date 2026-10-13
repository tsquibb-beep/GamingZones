// Shared build script for the gamingzones, ruler and gz-editor exes. Generates the Windows resource
// script (icon, PMv2 manifest, version info) so the exe's file properties always match
// version.txt. The icon is res/<package name>.ico.
use std::{env, fs, path::PathBuf};

fn main() {
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = crate_dir.join("../..").canonicalize().unwrap();
    let name = env::var("CARGO_PKG_NAME").unwrap();
    // What Task Manager shows as the process name.
    let description = match name.as_str() {
        "ruler" => "GamingZones Ruler",
        "gz-editor" => "GamingZones Editor",
        _ => "GamingZones",
    };
    let version = fs::read_to_string(root.join("version.txt")).unwrap();
    let version = version.trim();
    assert_eq!(
        version,
        env::var("CARGO_PKG_VERSION").unwrap(),
        "version.txt and the workspace Cargo.toml version differ"
    );
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));

    let res = root.join("res");
    let path = |file: &str| res.join(file).display().to_string().replace('\\', "/");
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{ico}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Tom Squibb"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{name}"
      VALUE "LegalCopyright", "Copyright (c) 2026 Tom Squibb. GPL-3.0."
      VALUE "OriginalFilename", "{name}.exe"
      VALUE "ProductName", "GamingZones"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
        ico = path(&format!("{name}.ico")),
        manifest = path("app.manifest"),
    );

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_required().unwrap();

    println!("cargo:rerun-if-changed={}", root.join("version.txt").display());
    println!("cargo:rerun-if-changed={}", res.display());
}
