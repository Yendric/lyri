fn main() {
    windows_reactor_setup::as_framework_dependent();

    winresource::WindowsResource::new()
        .set_icon("./Assets/app.ico")
        .compile()
        .expect("failed to embed Windows executable icon");
}
