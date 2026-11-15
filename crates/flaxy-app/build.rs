fn main() {
    // Ícone e metadados no executável do Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "FLAXY")
            .set("FileDescription", "FLAXY")
            .set("CompanyName", "FLAXY")
            .set("LegalCopyright", "FLAXY");
        if let Err(e) = res.compile() {
            println!("cargo:warning=ícone não embutido: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
