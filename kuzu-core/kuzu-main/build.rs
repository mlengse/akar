fn main() {
    // Allow the `kuzu_wasm` cfg flag (set via .cargo/config.toml
    // for wasm32-unknown-unknown target).
    println!("cargo::rustc-check-cfg=cfg(kuzu_wasm)");
}
