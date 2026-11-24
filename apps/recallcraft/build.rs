//! Windows only: embed RecallCraft's icon and version info into `recallcraft.exe`, so it shows in
//! Explorer, the taskbar and the Start menu. A missing resource compiler is a warning unless
//! `RECALLCRAFT_REQUIRE_WINRES=1` (release builds) makes it an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/recallcraft.ico");
    println!("cargo:rerun-if-env-changed=RECALLCRAFT_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=RECALLCRAFT_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/recallcraft.ico")
        .set("ProductName", "RecallCraft")
        .set("FileDescription", "RecallCraft")
        .set("CompanyName", "RecallCraft contributors")
        .set("LegalCopyright", "Copyright (c) the RecallCraft authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "recallcraft.exe")
        .set("InternalName", "recallcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("RECALLCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=recallcraft.exe built without icon/version resources: {e}");
    }
}
