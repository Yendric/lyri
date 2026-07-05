fn main() {
    // The build-dependencies only exist on Windows hosts; keep the build
    // script compilable when the workspace is checked on other platforms.
    #[cfg(windows)]
    {
        windows_reactor_setup::as_framework_dependent();

        winresource::WindowsResource::new()
            .set_icon("./Assets/app.ico")
            .compile()
            .expect("failed to embed Windows executable icon");
    }
}
