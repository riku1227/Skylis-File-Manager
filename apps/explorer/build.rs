fn main() {
    println!("cargo:rerun-if-changed=src/shell_handoff/windows.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cc::Build::new()
            .cpp(true)
            .file("src/shell_handoff/windows.cpp")
            .flag_if_supported("/std:c++17")
            .define("UNICODE", None)
            .define("_UNICODE", None)
            .compile("skylis_shell_handoff");
        for library in [
            "ole32",
            "oleaut32",
            "shell32",
            "shlwapi",
            "uuid",
            "uiautomationcore",
        ] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
    println!("cargo:rerun-if-changed=src/media/quicklook.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/media/quicklook.m")
            .flag("-fobjc-arc")
            .compile("skylis_quicklook");
        for framework in ["AppKit", "Foundation", "Quartz"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
