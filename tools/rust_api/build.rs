fn main() {
    // Pure Rust — akar-main provides all functionality.
    // No C++ compilation or cmake needed.
    println!("cargo:rustc-cfg=native");
}
