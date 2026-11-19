fn main() {
    // winresource only exists on Windows hosts; keep the build script
    // compilable when the workspace is checked on other platforms.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("./Assets/app.ico")
        .compile()
        .expect("failed to embed Windows executable icon");
}
