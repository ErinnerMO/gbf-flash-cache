# Run with PowerShell: validates the actual build-script path setup without building.
$ErrorActionPreference = 'Stop'
$source = Get-Content "$PSScriptRoot/build_windows.ps1" -Raw
$setup = $source.Substring(0, $source.IndexOf('$outRoot ='))
$temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
$repo = Join-Path $temp 'repo with spaces'
$oldTarget = $env:CARGO_TARGET_DIR
$oldCrt = $env:GBF_VC_REDIST
$oldLocation = Get-Location
try {
  New-Item -ItemType Directory -Force "$repo/scripts", "$repo/app" | Out-Null
  Set-Content "$repo/scripts/path_setup.ps1" $setup
  foreach ($cwd in @($temp, "$repo/app")) {
    foreach ($setting in @('', 'relative output', '../external output', (Join-Path $temp 'absolute output'))) {
      Set-Location $cwd
      $env:CARGO_TARGET_DIR = $setting
      & {
        . "$repo/scripts/path_setup.ps1"
        $expected = if (!$setting) { Join-Path $repo 'target' } elseif ([IO.Path]::IsPathRooted($setting)) { $setting } else { Join-Path $repo $setting }
        $expected = [IO.Path]::GetFullPath($expected)
        if ($target -ne $expected -or $env:CARGO_TARGET_DIR -ne $expected) {
          throw "Cargo and packaging paths disagree for '$setting' from '$cwd'"
        }
        # Switching to app must still select the freshly built DLL.
        New-Item -ItemType Directory -Force "$target/release" | Out-Null
        Set-Content "$target/release/gbf_flash_cache_app.dll" 'fresh'
        Set-Location "$repo/app"
        if ((Get-Content "$target/release/gbf_flash_cache_app.dll") -ne 'fresh') {
          throw 'Selected the wrong native library after changing directory'
        }
      }
    }
  }
  # Execute the real packaging block against a Release directory used by the app.
  $root = $repo
  $target = Join-Path $repo 'target'
  $outRoot = Join-Path $temp 'packages'
  $Version = 'test'
  $env:GBF_VC_REDIST = Join-Path $temp 'crt'
  $release = Join-Path $repo 'app/build/windows/x64/runner/Release'
  $files = @{
    'gbf_flash_cache.exe' = 'exe'; 'flutter_windows.dll' = 'flutter'; 'plugin.dll' = 'plugin'
    'ui-assets/icudtl.dat' = 'icu'; 'ui-assets/app.so' = 'aot'; 'ui-assets/flutter_assets/asset.txt' = 'asset'
    'data/ca/authority.json' = 'SYNTHETIC PRIVATE KEY'; 'data/settings.json' = 'SYNTHETIC CONFIG'
    'data/logs/run.log' = 'SYNTHETIC LOG'; 'data/cache/resource.gfc' = 'SYNTHETIC CACHE'
    'debug.pdb' = 'symbols'; 'notes.txt' = 'private notes'
  }
  foreach ($relative in $files.Keys) {
    $file = Join-Path $release $relative
    New-Item -ItemType Directory -Force (Split-Path -Parent $file) | Out-Null
    Set-Content $file $files[$relative]
  }
  New-Item -ItemType Directory -Force "$target/release", "$root/dist/licenses", $env:GBF_VC_REDIST, $outRoot | Out-Null
  Set-Content "$target/release/gbf_flash_cache_app.dll" 'fresh core'
  Set-Content "$root/LICENSE" 'MIT'
  Set-Content "$root/dist/licenses/notice.txt" 'third party license'
  Set-Content "$env:GBF_VC_REDIST/vcruntime140.dll" 'crt'
  Set-Location "$repo/app"
  & ([scriptblock]::Create($source.Substring($source.IndexOf('$name ='))))
  $zip = Join-Path $outRoot 'gbf-flash-cache-test-portable-windows-x64.zip'
  Expand-Archive $zip "$temp/unpacked"
  $package = Join-Path $temp 'unpacked/gbf-flash-cache-test-portable-windows-x64'
  $expected = @('GBF Flash Cache.exe', 'flutter_windows.dll', 'plugin.dll', 'ui-assets/icudtl.dat',
    'ui-assets/app.so', 'ui-assets/flutter_assets/asset.txt', 'core/gbf_flash_cache_app.dll',
    'LICENSE', 'licenses/notice.txt', 'vcruntime140.dll') | Sort-Object
  $actual = Get-ChildItem $package -Recurse -File | ForEach-Object {
    $_.FullName.Substring($package.Length + 1).Replace('\', '/')
  } | Sort-Object
  if (Compare-Object $expected $actual) { throw 'Package includes unexpected files or omits required runtime files' }
  if (!(Test-Path "$release/data/ca/authority.json")) { throw 'Packaging must not delete local runtime data' }
  Write-Output '8 Windows build path cases and contaminated-Release packaging passed (no compilation).'
} finally {
  Set-Location $oldLocation
  $env:CARGO_TARGET_DIR = $oldTarget
  $env:GBF_VC_REDIST = $oldCrt
  Remove-Item $temp -Recurse -Force
}
