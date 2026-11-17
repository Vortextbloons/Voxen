$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $PSScriptRoot "out"

Push-Location $root
try {
    cargo build --release --manifest-path src-tauri/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $version = (Select-String -Path src-tauri/Cargo.toml -Pattern '^version = "(.+)"' -List).Matches[0].Groups[1].Value
    $zip = "Voxen_${version}_x64.zip"

    if (Test-Path $out) { Remove-Item -Recurse -Force $out }
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    Copy-Item "src-tauri/target/release/voxen.exe" (Join-Path $out "Voxen.exe")
    Copy-Item "README.md" (Join-Path $out "README.md")
    Compress-Archive -Path (Join-Path $out "Voxen.exe"), (Join-Path $out "README.md") -DestinationPath (Join-Path $out $zip) -Force

    $sums = @("Voxen.exe", $zip) | ForEach-Object {
        "$((Get-FileHash -Algorithm SHA256 -Path (Join-Path $out $_)).Hash.ToLower())  $_"
    }
    Set-Content -Path (Join-Path $out "SHA256SUMS.txt") -Value $sums -Encoding ascii

    Write-Output "Staged release artifacts in $out"
}
finally {
    Pop-Location
}
