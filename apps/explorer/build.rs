fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/branding/skylis.ico");
    println!("cargo:rerun-if-changed=src/shell_handoff/windows.cpp");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version_defines = [
            ("CARGO_PKG_VERSION_MAJOR", "SKYLIS_VERSION_MAJOR", false),
            ("CARGO_PKG_VERSION_MINOR", "SKYLIS_VERSION_MINOR", false),
            ("CARGO_PKG_VERSION_PATCH", "SKYLIS_VERSION_PATCH", false),
            ("CARGO_PKG_VERSION", "SKYLIS_VERSION_STRING", true),
        ]
        .map(|(env, name, quoted)| {
            println!("cargo:rerun-if-env-changed={env}");
            let value = std::env::var(env).expect("Cargo package version");
            if quoted {
                format!("{name}=\"{value}\"")
            } else {
                format!("{name}={value}")
            }
        });
        embed_resource::compile("app.rc", &version_defines)
            .manifest_required()
            .expect("failed to embed Skylis application resources");
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
    println!("cargo:rerun-if-changed=src/shell_icons_macos.m");
    println!("cargo:rerun-if-changed=src/file_operations/macos_trash.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/media/quicklook.m")
            .file("src/shell_icons_macos.m")
            .file("src/file_operations/macos_trash.m")
            .flag("-fobjc-arc")
            .compile("skylis_quicklook");
        for framework in ["AppKit", "Foundation", "Quartz", "UniformTypeIdentifiers"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
