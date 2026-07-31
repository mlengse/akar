@echo off
echo ===== Akar Rust Environment Check =====
echo.
echo --- Rust ---
where rustc 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo rustc NOT FOUND — install Rust from https://rustup.rs
) else (
    rustc --version
)
where cargo 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo cargo NOT FOUND
) else (
    cargo --version
)
echo.
echo --- MinGW (for x86_64-pc-windows-gnu target) ---
where gcc 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo gcc NOT FOUND — needed for MinGW target
) else (
    gcc --version | findstr "^gcc"
)
echo.
echo --- Build Commands ---
echo   cargo build --target x86_64-pc-windows-gnu
echo   cargo test --target x86_64-pc-windows-gnu --workspace
echo   cargo run --bin akar-cli
echo.
echo --- Rust Targets ---
rustup target list --installed
echo.
echo === Selesai ===
