//! MSVC links the main thread with a 1 MiB stack, which start-up overruns, so
//! MSVC builds ask for 8 MiB instead.

const MAIN_STACK_BYTES: u32 = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_STACK_BYTES}"),
        _ => {}
    }
}
