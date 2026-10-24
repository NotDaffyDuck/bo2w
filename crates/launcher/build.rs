//! MSVC's default 1 MiB main-thread stack is too small for the app's
//! start-up; ask the linker for 8 MiB.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if msvc {
        println!("cargo::rustc-link-arg-bins=/STACK:8388608");
    }
}
