param([string]$Version = '1.14.98', [string]$OutputDirectory = $env:WINDOWS_BUILD_OUTPUT)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
# Cargo runs from the repository root; keep this path stable after entering app/.
if (![IO.Path]::IsPathRooted($target)) { $target = Join-Path $root $target }
$target = [IO.Path]::GetFullPath($target)
$env:CARGO_TARGET_DIR = $target
$outRoot = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { "$root\dist" }
New-Item -ItemType Directory -Force $outRoot | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\flutter\bin;C:\Program Files\Git\cmd;" + $env:Path
& dart.bat "$root\app\tool\check_version.dart" $Version
if ($LASTEXITCODE) { throw 'Release version mismatch; update Cargo and pubspec versions before building' }
if (!(Test-Path "$env:USERPROFILE\.cargo\bin\rustup.exe")) {
  Invoke-WebRequest 'https://win.rustup.rs/x86_64' -OutFile "$root\rustup-init.exe"
  & "$root\rustup-init.exe" -y --profile minimal --default-toolchain 1.98.1
  if ($LASTEXITCODE) { throw 'Rust installation failed' }
}
& rustup toolchain install 1.98.1 --profile minimal
if ($LASTEXITCODE) { throw 'Rust toolchain unavailable' }
# Remap source/dependency locations embedded by Rust panic and location macros.
$remaps = @(
  "--remap-path-prefix=$env:USERPROFILE=/build-home",
  "--remap-path-prefix=$($env:USERPROFILE.Replace('\', '/'))=/build-home",
  "--remap-path-prefix=$root=/gfc",
  "--remap-path-prefix=$($root.Replace('\', '/'))=/gfc"
)
$env:CARGO_ENCODED_RUSTFLAGS = (@($env:CARGO_ENCODED_RUSTFLAGS) + $remaps | Where-Object { $_ }) -join [char]31
Set-Location $root
# Certificate installation requires desktop confirmation; run that test separately.
& cargo +1.98.1 test --locked --workspace --all-targets -- --skip windows_trust_is_exact_and_old_ca_is_removed
if ($LASTEXITCODE) { throw 'Rust tests failed' }
& cargo +1.98.1 build --locked -p gbf-flash-cache-app --release
if ($LASTEXITCODE) { throw 'Rust build failed' }
Set-Location "$root\app"
& flutter.bat pub get
if ($LASTEXITCODE) { throw 'pub get failed' }
& flutter.bat analyze
if ($LASTEXITCODE) { throw 'analysis failed' }
& flutter.bat test
if ($LASTEXITCODE) { throw 'Flutter tests failed' }
& dart.bat run tool/native_smoke.dart "$target\release\gbf_flash_cache_app.dll"
if ($LASTEXITCODE) { throw 'Native bridge test failed' }
& dart.bat "$root\app\tool\prepare_release.dart"
if ($LASTEXITCODE) { throw 'Flutter release path mapping failed' }
& flutter.bat build windows --release --no-pub "--split-debug-info=$root\app\build\symbols" "--dart-define=GBF_VERSION=$Version" "--build-name=$Version"
if ($LASTEXITCODE) { throw 'Windows build failed' }
$name = "gbf-flash-cache-$Version-portable-windows-x64"
$out = "$root\app\build\package\$name"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null
# Only Flutter runtime files belong in the package; adjacent data/ is user state.
$release = 'build\windows\x64\runner\Release'
Copy-Item "$release\gbf_flash_cache.exe", "$release\*.dll" $out
Copy-Item "$release\ui-assets" "$out\ui-assets" -Recurse
Rename-Item "$out\gbf_flash_cache.exe" 'GBF Flash Cache.exe'
New-Item -ItemType Directory "$out\core" | Out-Null
Copy-Item "$target\release\gbf_flash_cache_app.dll" "$out\core"
Copy-Item "$root\LICENSE" $out
if (!(Test-Path "$root\dist\licenses")) { throw "Generate dist/licenses with scripts/collect_licenses.py before packaging" }
Copy-Item "$root\dist\licenses" "$out\licenses" -Recurse
# Ship the VC runtime beside the EXE; no separate runtime installer.
$crt = if ($env:GBF_VC_REDIST) { Get-Item $env:GBF_VC_REDIST } else { Get-ChildItem 'C:\BuildTools\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT' -Directory | Sort-Object FullName -Descending | Select-Object -First 1 }
if (!$crt) { throw 'VC runtime not found' }
Copy-Item "$($crt.FullName)\*.dll" $out
Compress-Archive $out "$outRoot\$name.zip" -Force
(Get-FileHash "$outRoot\$name.zip" -Algorithm SHA256).Hash.ToLower() | Set-Content "$outRoot\$name.zip.sha256"
