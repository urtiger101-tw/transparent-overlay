fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app_icon.ico");
        resource.compile().expect("無法嵌入 Windows App 圖示");
    }
}
