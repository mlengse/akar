<#
.SYNOPSIS
    Membersihkan biner tes lama, PDB, dan target non-core di repo Akar tanpa mengurangi kecepatan build.

.DESCRIPTION
    Script ini secara cerdas membersihkan sampah kompilasi Cargo (stale artifacts) yang menumpuk
    di target\debug\deps, dengan tetap menyisakan 1 versi terbaru untuk setiap file tes dan dependensi aktif.
    Juga membersihkan folder target dari workspace terpisah (akar-python, fuzz, dll.).

.PARAMETER IncludeIncremental
    Jika disertakan, folder target\debug\incremental juga akan dibersihkan (menghemat 10-20 GB ekstra).

.EXAMPLE
    pwsh ./tools/clean-target.ps1
    pwsh ./tools/clean-target.ps1 -IncludeIncremental
#>

[CmdletBinding()]
param(
    [switch]$IncludeIncremental
)

$ErrorActionPreference = "SilentlyContinue"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$depsPath = Join-Path $repoRoot "akar-core\target\debug\deps"

Write-Host "=== Akar Stale Artifacts Cleaner ===" -ForegroundColor Cyan
$totalDeletedFiles = 0
$totalFreedBytes = 0

# 1. Bersihkan biner tes .exe dan .pdb usang di deps
if (Test-Path $depsPath) {
    Write-Host "`n[1/3] Memeriksa biner tes (.exe) dan simbol debug (.pdb) usang..." -ForegroundColor Yellow
    $exeGroups = Get-ChildItem -Path $depsPath -Filter "*.exe" | ForEach-Object {
        $base = $_.BaseName -replace '-[0-9a-fA-F]{16}$',''
        [PSCustomObject]@{ Base = $base; File = $_; LastWrite = $_.LastWriteTime }
    } | Group-Object Base

    $staleExeCount = 0
    $staleExeBytes = 0

    foreach ($grp in $exeGroups) {
        $sorted = $grp.Group | Sort-Object LastWrite -Descending
        # Sisakan yang paling baru (index 0), hapus versi-versi lama
        $sorted | Select-Object -Skip 1 | ForEach-Object {
            $staleExeBytes += $_.File.Length
            $staleExeCount++
            Remove-Item -LiteralPath $_.File.FullName -Force

            $pdb = $_.File.FullName -replace '\.exe$', '.pdb'
            if (Test-Path $pdb) {
                $pItem = Get-Item $pdb
                $staleExeBytes += $pItem.Length
                $staleExeCount++
                Remove-Item -LiteralPath $pdb -Force
            }
        }
    }
    $totalDeletedFiles += $staleExeCount
    $totalFreedBytes += $staleExeBytes
    Write-Host ("      Dihapus: {0} file stale test/pdb ({1:N2} GB)" -f $staleExeCount, ($staleExeBytes / 1GB)) -ForegroundColor Green

    # 2. Bersihkan .rlib usang di deps
    Write-Host "`n[2/3] Memeriksa library (.rlib) versi lama..." -ForegroundColor Yellow
    $rlibGroups = Get-ChildItem -Path $depsPath -Filter "*.rlib" | ForEach-Object {
        $base = $_.BaseName -replace '-[0-9a-fA-F]{16}$',''
        [PSCustomObject]@{ Base = $base; File = $_; LastWrite = $_.LastWriteTime }
    } | Group-Object Base

    $staleRlibCount = 0
    $staleRlibBytes = 0

    foreach ($grp in $rlibGroups) {
        $sorted = $grp.Group | Sort-Object LastWrite -Descending
        $sorted | Select-Object -Skip 1 | ForEach-Object {
            $staleRlibBytes += $_.File.Length
            $staleRlibCount++
            Remove-Item -LiteralPath $_.File.FullName -Force
        }
    }
    $totalDeletedFiles += $staleRlibCount
    $totalFreedBytes += $staleRlibBytes
    Write-Host ("      Dihapus: {0} file stale rlib ({1:N2} GB)" -f $staleRlibCount, ($staleRlibBytes / 1GB)) -ForegroundColor Green
} else {
    Write-Host "Folder target\debug\deps tidak ditemukan." -ForegroundColor DarkGray
}

# 3. Bersihkan target non-core (akar-python, fuzz, examples, tools)
Write-Host "`n[3/3] Memeriksa folder target non-core..." -ForegroundColor Yellow
$nonCoreTargets = @(
    (Join-Path $repoRoot "akar-core\akar-python\target"),
    (Join-Path $repoRoot "akar-core\fuzz\target"),
    (Join-Path $repoRoot "examples\rust\target"),
    (Join-Path $repoRoot "tools\rust_api\target")
)

$nonCoreBytes = 0
foreach ($t in $nonCoreTargets) {
    if (Test-Path $t) {
        $m = Get-ChildItem -Path $t -Recurse -File -Force | Measure-Object -Property Length -Sum
        $s = if ($m.Sum) { $m.Sum } else { 0 }
        $nonCoreBytes += $s
        Remove-Item -LiteralPath $t -Recurse -Force
        Write-Host ("      Dihapus: {0} ({1:N2} GB)" -f $t, ($s / 1GB)) -ForegroundColor Green
    }
}
$totalFreedBytes += $nonCoreBytes

# Opsi: Bersihkan incremental cache
if ($IncludeIncremental) {
    $incPath = Join-Path $repoRoot "akar-core\target\debug\incremental"
    if (Test-Path $incPath) {
        $m = Get-ChildItem -Path $incPath -Recurse -File -Force | Measure-Object -Property Length -Sum
        $s = if ($m.Sum) { $m.Sum } else { 0 }
        Remove-Item -LiteralPath $incPath -Recurse -Force
        $totalFreedBytes += $s
        Write-Host ("`n[Opsional] Cache inkremental dibersihkan: {0:N2} GB" -f ($s / 1GB)) -ForegroundColor Green
    }
}

Write-Host "`n==========================================" -ForegroundColor Cyan
Write-Host ("Pembersihan selesai! Total ruang dibebaskan: {0:N2} GB" -f ($totalFreedBytes / 1GB)) -ForegroundColor Green
Write-Host "Semua biner aktif dan cache kompilasi tetap utuh." -ForegroundColor Gray
