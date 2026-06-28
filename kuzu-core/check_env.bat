@echo off
echo Memeriksa lingkungan build...
where cl.exe 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo cl.exe tidak ditemukan. Mencari VS Build Tools...
    if exist "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" (
        echo VS Build Tools ditemukan, menjalankan vcvars64...
        call "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
    ) else (
        echo VS Build Tools tidak ditemukan. Menginstal...
        echo Proses instalasi bisa memakan waktu 5-10 menit...
    )
) else (
    echo cl.exe ditemukan
)
echo.
where rustc 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo rustc tidak ditemukan
) else (
    rustc --version
)
echo.
where cargo 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo cargo tidak ditemukan
) else (
    cargo --version
)
echo.
echo === Selesai ===
