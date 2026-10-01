fn main() {
    println!("cargo:rerun-if-changed=assets/AppIcon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/AppIcon.ico");
        resource.set_manifest_file("assets/app.manifest");
        resource.set("FileDescription", "QuickADB 安装抽屉");
        resource.set("ProductName", "QuickADB");
        resource
            .compile()
            .expect("Windows application resource compilation failed");
    }
}
