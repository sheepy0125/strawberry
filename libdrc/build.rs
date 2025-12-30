use std::env;
use std::path::PathBuf;

fn main() {

    let bindings = bindgen::Builder::default()
        .header("src/wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .clang_args(["-x", "c++", "-I/home/ruben/libdrc/include"])
        .allowlist_file(".+/drc/c/[^/]*")
        .generate()
        .expect("bindgen");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("failed to write bindings");
    println!("cargo:rerun-if-changed=/home/ruben/libdrc/libdrc.so");
    std::fs::copy("/home/ruben/libdrc/libdrc.so", out_path.join("libdrc.so")).expect("copy lib");
    println!("cargo:rustc-link-search={}", out_path.display());
    println!("cargo:rustc-link-lib=drc");
}
