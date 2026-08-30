fn main() {
    println!("cargo:rerun-if-changed=ui/app.slint");
    slint_build::compile("ui/app.slint").expect("failed to compile Slint UI");
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    println!("cargo:rerun-if-changed=img/Icons/logo.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("img/Icons/logo.ico");
    resource.compile().expect("failed to embed Windows icon");
}

#[cfg(not(windows))]
fn embed_windows_icon() {}
