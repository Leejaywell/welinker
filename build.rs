use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let web_dir = manifest_dir.join("web");

    watch_web_inputs(&web_dir);
    build_web(&web_dir);
    write_web_assets(&manifest_dir, &web_dir);
}

fn watch_web_inputs(web_dir: &Path) {
    println!(
        "cargo:rerun-if-changed={}",
        web_dir.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web_dir.join("package-lock.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web_dir.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web_dir.join("vite.config.js").display()
    );
    let src_dir = web_dir.join("src");
    if let Ok(files) = collect_files(&src_dir) {
        for file in files {
            println!("cargo:rerun-if-changed={}", file.display());
        }
    }
}

fn build_web(web_dir: &Path) {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if !web_dir.join("node_modules").exists() {
        let status = Command::new(npm)
            .arg("install")
            .current_dir(web_dir)
            .status()
            .expect("failed to run npm install for web UI");
        assert!(status.success(), "npm install failed for web UI");
    }

    let status = Command::new(npm)
        .args(["run", "build"])
        .current_dir(web_dir)
        .status()
        .expect("failed to run npm run build for web UI");
    assert!(status.success(), "npm run build failed for web UI");
}

fn write_web_assets(manifest_dir: &Path, web_dir: &Path) {
    let dist_dir = web_dir.join("dist");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_file = out_dir.join("web_assets.rs");
    let mut files = collect_files(&dist_dir).expect("read web/dist files");
    files.sort();

    let mut generated = String::from(
        "pub struct WebAsset {\n    pub content_type: &'static str,\n    pub bytes: &'static [u8],\n}\n\n",
    );
    generated.push_str(&format!(
        "pub const WEB_INDEX: &str = include_str!(r#\"{}\"#);\n\n",
        dist_dir.join("index.html").display()
    ));
    let mut asset_paths = Vec::new();
    generated.push_str("pub fn web_asset(path: &str) -> Option<WebAsset> {\n    match path {\n");

    for file in files {
        if file.file_name().and_then(|name| name.to_str()) == Some("index.html") {
            continue;
        }
        let rel = file
            .strip_prefix(&dist_dir)
            .expect("dist child")
            .to_string_lossy()
            .replace('\\', "/");
        asset_paths.push(rel.clone());
        let content_type = content_type(&file);
        generated.push_str(&format!(
            "        r#\"{}\"# => Some(WebAsset {{ content_type: r#\"{}\"#, bytes: include_bytes!(r#\"{}\"#) }}),\n",
            rel,
            content_type,
            file.display()
        ));
    }

    generated.push_str("        _ => None,\n    }\n}\n\n");
    generated.push_str("#[cfg(test)]\npub const WEB_ASSET_PATHS: &[&str] = &[\n");
    for rel in asset_paths {
        generated.push_str(&format!("    r#\"{}\"#,\n", rel));
    }
    generated.push_str("];\n");
    fs::write(&out_file, generated).expect("write generated web assets");

    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("build.rs").display()
    );
}

fn collect_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    collect_files_inner(dir, &mut files)?;
    Ok(files)
}

fn collect_files_inner(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_inner(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
