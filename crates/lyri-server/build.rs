use std::env;
use std::fs;
use std::path::Path;

/// Served when `apps/lyri-web` has not been built (`pnpm -C apps/lyri-web build`).
const PLACEHOLDER: &str = "<!doctype html>\n<html>\n<head><meta charset=\"utf-8\"><title>Lyri</title></head>\n<body style=\"font-family: system-ui; display: grid; place-items: center; height: 100vh; margin: 0\">\n<p>This Lyri build does not include the web view. Build it with <code>pnpm -C apps/lyri-web build</code> and rebuild Lyri.</p>\n</body>\n</html>\n";

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/lyri-web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());

    let dest = Path::new(&env::var("OUT_DIR").expect("OUT_DIR set by cargo")).join("webview");
    let _ = fs::remove_dir_all(&dest);
    fs::create_dir_all(&dest).expect("create embedded webview directory");

    if dist.join("index.html").is_file() {
        copy_dir(&dist, &dest);
    } else {
        fs::write(dest.join("index.html"), PLACEHOLDER).expect("write placeholder index.html");
    }
}

fn copy_dir(src: &Path, dest: &Path) {
    for entry in fs::read_dir(src).expect("read web dist directory").flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target).expect("create embedded asset directory");
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy web asset");
        }
    }
}
