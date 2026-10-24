# Downloads third-party accuracy test data into .\test-data (gitignored).
#   scripts\fetch-test-data.ps1            test ROMs (~6 MB)
#   scripts\fetch-test-data.ps1 -CpuJson   also SingleStepTests nes6502 (~1.3 GB)
param([switch]$CpuJson)
$ErrorActionPreference = 'Stop'
$dest = Join-Path (Split-Path $PSScriptRoot -Parent) 'test-data'
New-Item -ItemType Directory -Force $dest | Out-Null

$roms = Join-Path $dest 'nes-test-roms'
if (-not (Test-Path $roms)) {
    git clone --depth 1 https://github.com/christopherpow/nes-test-roms.git $roms
} else {
    Write-Host 'test-data\nes-test-roms already present'
}

$json = Join-Path $dest '65x02'
if ($CpuJson -and -not (Test-Path $json)) {
    git clone --depth 1 --filter=blob:none --sparse https://github.com/SingleStepTests/65x02.git $json
    git -C $json sparse-checkout set nes6502
}
