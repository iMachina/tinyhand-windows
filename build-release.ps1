# Builds target\release\tinyhand.exe for handing to other people:
#   - the C runtime is linked in, so no Visual C++ Redistributable is needed;
#   - folder names from the build machine are kept out of the executable.
#
#   powershell -ExecutionPolicy Bypass -File build-release.ps1

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:Path = "$cargoHome\bin;$env:Path" }

# Setting this replaces the flags in .cargo\config.toml, so the static runtime is repeated here.
# The separator is the unit separator character, which lets paths contain spaces.
$flags = @(
    '-Ctarget-feature=+crt-static',
    "--remap-path-prefix=$cargoHome=cargo",
    "--remap-path-prefix=$rustupHome=rustup",
    "--remap-path-prefix=$root=tinyhand"
)
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f

Push-Location $root
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
    Remove-Item Env:\CARGO_ENCODED_RUSTFLAGS
}

$exe = Join-Path $root 'target\release\tinyhand.exe'
$bytes = [System.IO.File]::ReadAllBytes($exe)
$ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
$wide = [System.Text.Encoding]::Unicode.GetString($bytes)
foreach ($needle in @($cargoHome, $rustupHome, $root, $env:USERPROFILE)) {
    foreach ($form in @($needle, $needle.Replace('\', '/'))) {
        if ($ascii.IndexOf($form, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
            $wide.IndexOf($form, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "A local path is still inside the executable. Do not publish it."
        }
    }
}

$hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
"{0}  {1:N0} bytes" -f $exe, $bytes.Length
"SHA-256  $hash"
