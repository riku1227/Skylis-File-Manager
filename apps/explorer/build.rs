fn main() {
    println!("cargo:rerun-if-changed=src/media/quicklook.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new().file("src/media/quicklook.m").flag("-fobjc-arc").compile("skylis_quicklook");
        for framework in ["AppKit", "Foundation", "Quartz"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
