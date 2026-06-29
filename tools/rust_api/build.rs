fn main() {
    // Pure Rust — kuzu-main provides all functionality.
    // No C++ compilation or cmake needed.
    println!("cargo:rustc-cfg=native");
}
