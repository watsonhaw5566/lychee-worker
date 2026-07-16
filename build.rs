//! lychee-worker build script.
//!
//! On macOS, the system linker by default refuses to produce a `.dylib` when
//! any referenced symbol cannot be resolved at link-time. PHP extensions,
//! however, intentionally rely on symbols exported by the PHP binary at
//! runtime, so we must explicitly tell the linker to allow this.

fn main() {
    // Tell cargo to re-run this script when the target OS changes.
    println!("cargo:rerun-if-changed=build.rs");

    // Only macOS needs the extra linker flag; Linux/Windows handle undefined
    // symbols in shared libraries natively.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-undefined");
        println!("cargo:rustc-link-arg=dynamic_lookup");
    }
}
