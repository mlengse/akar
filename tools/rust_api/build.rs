fn main() {
    // Check if native mode is enabled
    let native = std::env::var("CARGO_FEATURE_NATIVE").is_ok();
    if native {
        // Native mode: no C++ compilation needed
        println!("cargo:rustc-cfg=native");
        return;
    }

    // Legacy C++ FFI mode — only compiled when native feature is disabled
    #[cfg(not(feature = "native"))]
    legacy_main();
}

// ==================== Legacy C++ FFI build ====================

#[cfg(not(feature = "native"))]
fn legacy_main() {
    if std::env::var("DOCS_RS").is_ok() {
        return;
    }

    let mut bundled = false;
    let mut include_paths = vec![
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("include"),
    ];

    if let (Ok(kuzu_lib_dir), Ok(kuzu_include)) =
        (std::env::var("KUZU_LIBRARY_DIR"), std::env::var("KUZU_INCLUDE_DIR"))
    {
        println!("cargo:rustc-link-search=native={kuzu_lib_dir}");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{kuzu_lib_dir}");
        include_paths.push(std::path::Path::new(&kuzu_include).to_path_buf());
    } else {
        include_paths.extend(build_bundled_cmake());
        bundled = true;
    }
    if link_mode() == "static" {
        link_libraries();
    }
    build_ffi(
        "src/ffi.rs",
        "kuzu_rs",
        "src/kuzu_rs.cpp",
        bundled,
        &include_paths,
    );

    if cfg!(feature = "arrow") {
        build_ffi(
            "src/ffi/arrow.rs",
            "kuzu_arrow_rs",
            "src/kuzu_arrow.cpp",
            bundled,
            &include_paths,
        );
    }
    if link_mode() == "dylib" {
        link_libraries();
    }
}

#[cfg(not(feature = "native"))]
fn get_target() -> String {
    std::env::var("PROFILE").unwrap()
}

#[cfg(not(feature = "native"))]
fn link_mode() -> &'static str {
    if std::env::var("KUZU_SHARED").is_ok() {
        "dylib"
    } else {
        "static"
    }
}

#[cfg(not(feature = "native"))]
fn link_libraries() {
    use std::env;
    if !cfg!(windows) && link_mode() == "static" {
        println!("cargo:rustc-link-arg=-rdynamic");
    }
    if cfg!(windows) && link_mode() == "dylib" {
        println!("cargo:rustc-link-lib=dylib=kuzu_shared");
    } else if link_mode() == "dylib" {
        println!("cargo:rustc-link-lib={}=kuzu", link_mode());
    } else {
        println!("cargo:rustc-link-lib=static:+whole-archive=kuzu");
    }
    if link_mode() == "static" {
        if cfg!(windows) {
            println!("cargo:rustc-link-lib=dylib=msvcrt");
            println!("cargo:rustc-link-lib=dylib=shell32");
            println!("cargo:rustc-link-lib=dylib=ole32");
        } else if cfg!(target_os = "macos") {
            println!("cargo:rustc-link-lib=dylib=c++");
        } else {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
        for lib in [
            "utf8proc", "antlr4_cypher", "antlr4_runtime", "re2",
            "fastpfor", "parquet", "thrift", "snappy", "zstd",
            "miniz", "mbedtls", "brotlidec", "brotlicommon",
            "lz4", "roaring_bitmap", "simsimd",
        ] {
            println!("cargo:rustc-link-lib=static:+whole-archive={lib}");
        }
    }
}

#[cfg(not(feature = "native"))]
fn build_bundled_cmake() -> Vec<std::path::PathBuf> {
    use std::path::Path;
    let kuzu_root = {
        let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("kuzu-src");
        if root.is_symlink() || root.is_dir() {
            root
        } else {
            Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..")
        }
    };

    let mut build = cmake::Config::new(&kuzu_root);
    build
        .no_build_target(true)
        .define("BUILD_SHELL", "OFF")
        .define("BUILD_SINGLE_FILE_HEADER", "OFF")
        .define("AUTO_UPDATE_GRAMMAR", "OFF");
    if cfg!(windows) {
        build.generator("Ninja");
        build.cxxflag("/EHsc");
        build.define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL");
        build.define("CMAKE_POLICY_DEFAULT_CMP0091", "NEW");
    }
    if let Ok(jobs) = std::env::var("NUM_JOBS") {
        std::env::set_var("CMAKE_BUILD_PARALLEL_LEVEL", jobs);
    }
    let build_dir = build.build();
    let kuzu_lib_path = build_dir.join("build").join("src");
    println!("cargo:rustc-link-search=native={}", kuzu_lib_path.display());

    for dir in [
        "utf8proc", "antlr4_cypher", "antlr4_runtime", "re2",
        "brotli", "alp", "fastpfor", "parquet", "thrift",
        "snappy", "zstd", "miniz", "mbedtls", "lz4",
        "roaring_bitmap", "simsimd",
    ] {
        let lib_path = build_dir
            .join("build")
            .join("third_party")
            .join(dir)
            .canonicalize()
            .unwrap_or_else(|_| {
                panic!("Could not find {}/build/third_party/{}", build_dir.display(), dir)
            });
        println!("cargo:rustc-link-search=native={}", lib_path.display());
    }

    vec![
        kuzu_root.join("src/include"),
        build_dir.join("build/src"),
        build_dir.join("build/src/include"),
        kuzu_root.join("third_party/nlohmann_json"),
        kuzu_root.join("third_party/fastpfor"),
        kuzu_root.join("third_party/alp/include"),
    ]
}

#[cfg(not(feature = "native"))]
fn build_ffi(
    bridge_file: &str,
    out_name: &str,
    source_file: &str,
    bundled: bool,
    include_paths: &[std::path::PathBuf],
) {
    let mut build = cxx_build::bridge(bridge_file);
    build.file(source_file);
    if bundled {
        build.define("KUZU_BUNDLED", None);
    }
    if get_target() == "debug" || get_target() == "relwithdebinfo" {
        build.define("ENABLE_RUNTIME_CHECKS", "1");
    }
    if link_mode() == "static" {
        build.define("KUZU_STATIC_DEFINE", None);
    }
    build.includes(include_paths);
    build.flag_if_supported("-std=c++2a");
    if cfg!(windows) {
        build.flag("/MD");
    }
    build.compile(out_name);
}
