//! Build script of the taskbar helper library.

fn main() {
    // The two COM entry points are exported with `#[no_mangle]`; the MSVC linker warns
    // (LNK4104) that such entry points should be marked PRIVATE in a module-definition
    // file, which only matters for import libraries (nobody links against this library:
    // the XAML host finds the entry points by name). The warning is silenced, not hidden
    // from anything that matters.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-cdylib-link-arg=/IGNORE:4104");
    }
}
