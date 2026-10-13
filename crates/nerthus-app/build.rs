//! Embeds the interface icons: every SVG in assets/icons, listed in
//! `icon-files.rs` for src/icons.rs to include; the example projects with
//! their pictures (assets/examples), listed in `example-files.rs`; and the
//! documentation's pages (assets/docs and its folders), listed in
//! `doc-files.rs`.
//!
//! Puts the app icon in the Windows executable, where Explorer, shortcuts and
//! pinned taskbar buttons find it: assets/nerthus.svg drawn at the shell's
//! icon sizes into an .ico, named by a resource script that embed-resource
//! compiles (rc.exe, windres or llvm-rc) and links. The window icons are set
//! at run time (src/app_icon.rs).

use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=assets/nerthus.svg");
    // A directory: Cargo reruns when any file in it is added, removed or
    // changed.
    println!("cargo::rerun-if-changed=assets/icons");
    println!("cargo::rerun-if-changed=assets/examples");
    println!("cargo::rerun-if-changed=assets/docs");

    write_file_table("icons", &["svg"], "icon-files.rs");
    write_file_table("examples", &["terrain", "jpg"], "example-files.rs");
    write_page_table("docs", "doc-files.rs");

    // The build dependencies are only there on Windows hosts, where Windows
    // builds are made.
    #[cfg(windows)]
    windows_icon::embed();
}

/// Writes `table` to OUT_DIR: an array of the files in assets/`folder`
/// with these extensions as (file name, bytes), sorted by file name.
fn write_file_table(folder: &str, extensions: &[&str], table: &str) {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let directory = Path::new(&manifest_dir).join("assets").join(folder);
    let entries = std::fs::read_dir(&directory).unwrap_or_else(|_| panic!("cannot read assets/{folder}"));

    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.unwrap_or_else(|_| panic!("cannot read assets/{folder}")).path();
        let wanted = path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extensions.contains(&extension));
        if wanted {
            paths.push(path);
        }
    }
    paths.sort();

    // Debug-formatted strings are Rust string literals, backslashes in
    // Windows paths included.
    let mut contents = String::from("&[\n");
    for path in &paths {
        let file_name = path.file_name().unwrap().to_str().expect("asset file names are UTF-8");
        let full_path = path.to_str().expect("the asset paths are UTF-8");
        let entry = format!("    ({file_name:?}, include_bytes!({full_path:?}).as_slice()),\n");
        contents.push_str(&entry);
    }
    contents.push_str("]\n");

    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let table_path = Path::new(&out_dir).join(table);
    std::fs::write(&table_path, contents).unwrap_or_else(|_| panic!("cannot write {table}"));
}

/// Writes `table` to OUT_DIR: an array of the Markdown pages under
/// assets/`folder` and its folders as (path from there with `/` between
/// folders, text), sorted by path.
fn write_page_table(folder: &str, table: &str) {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest_dir).join("assets").join(folder);

    let mut pages = Vec::new();
    let mut folders = vec![root.clone()];
    while let Some(directory) = folders.pop() {
        let entries = std::fs::read_dir(&directory).unwrap_or_else(|_| panic!("cannot read {}", directory.display()));
        for entry in entries {
            let path = entry.unwrap_or_else(|_| panic!("cannot read {}", directory.display())).path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let is_page = path.extension().is_some_and(|extension| extension == "md");
            if is_page {
                pages.push(path);
            }
        }
    }

    let mut entries = Vec::new();
    for path in &pages {
        let relative = path.strip_prefix(&root).unwrap();
        let mut parts = Vec::new();
        for part in relative.components() {
            parts.push(part.as_os_str().to_str().expect("the page paths are UTF-8"));
        }
        let page = parts.join("/");
        let full_path = path.to_str().expect("the page paths are UTF-8");
        entries.push((page, full_path.to_string()));
    }
    entries.sort();

    let mut contents = String::from("&[\n");
    for (page, full_path) in &entries {
        let entry = format!("    ({page:?}, include_str!({full_path:?})),\n");
        contents.push_str(&entry);
    }
    contents.push_str("]\n");

    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let table_path = Path::new(&out_dir).join(table);
    std::fs::write(&table_path, contents).unwrap_or_else(|_| panic!("cannot write {table}"));
}

#[cfg(windows)]
mod windows_icon {
    use std::path::Path;

    use resvg::{tiny_skia, usvg};

    /// The shell's small (16–40) and large (32–64) icons at 100–250 % display
    /// scale, and the 256 of its largest views.
    const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

    pub fn embed() {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        if target_os != "windows" {
            return;
        }
        // Build scripts run in their package's directory.
        let svg = std::fs::read("assets/nerthus.svg").expect("cannot read assets/nerthus.svg");
        let options = usvg::Options::default();
        let tree = usvg::Tree::from_data(&svg, &options).expect("cannot parse assets/nerthus.svg");

        let mut images = Vec::new();
        for size in ICO_SIZES {
            images.push((size, render_png(&tree, size)));
        }
        let ico = ico_file(&images);
        let out_dir = std::env::var_os("OUT_DIR").unwrap();
        let ico_path = Path::new(&out_dir).join("nerthus.ico");
        std::fs::write(&ico_path, ico).expect("cannot write nerthus.ico");

        // Forward slashes, because resource scripts read backslashes in
        // strings as escapes; code page 65001 lets the path be UTF-8.
        let ico_reference = ico_path.display().to_string().replace('\\', "/");
        let script = format!("#pragma code_page(65001)\n1 ICON \"{ico_reference}\"\n");
        let script_path = Path::new(&out_dir).join("app-icon.rc");
        std::fs::write(&script_path, script).expect("cannot write app-icon.rc");

        // Without a resource compiler the executable just has no icon.
        let compiled = embed_resource::compile(&script_path, embed_resource::NONE);
        compiled.manifest_optional().expect("cannot compile app-icon.rc");
    }

    /// The logo filling a `size`² square, as PNG.
    fn render_png(tree: &usvg::Tree, size: u32) -> Vec<u8> {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();

        let logo_size = tree.size();
        let scale = size as f32 / logo_size.width().max(logo_size.height());
        let left = (size as f32 - logo_size.width() * scale) / 2.0;
        let top = (size as f32 - logo_size.height() * scale) / 2.0;
        let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, left, top);

        resvg::render(tree, transform, &mut pixmap.as_mut());
        pixmap.encode_png().expect("cannot encode the icon")
    }

    /// An .ico of PNG images (read by Windows Vista and later at any size): a
    /// 6-byte header, a 16-byte entry per image, then the images.
    fn ico_file(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut ico = Vec::new();
        ico.extend_from_slice(&0u16.to_le_bytes());
        // Type 1: icon.
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&(images.len() as u16).to_le_bytes());

        let mut offset = 6 + 16 * images.len();
        for (size, png) in images {
            // Width and height, with 256 written as 0.
            let dimension = if *size >= 256 { 0 } else { *size as u8 };
            ico.push(dimension);
            ico.push(dimension);
            // No palette, reserved.
            ico.push(0);
            ico.push(0);
            // One colour plane, 32 bits per pixel.
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += png.len();
        }
        for (_, png) in images {
            ico.extend_from_slice(png);
        }
        ico
    }
}
