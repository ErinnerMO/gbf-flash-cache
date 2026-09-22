$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$buildRoot = 'C:\gbf-build'
New-Item -ItemType Directory -Force $buildRoot | Out-Null
Start-Transcript -Path "$buildRoot\provision.log" -Append
try {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  Invoke-WebRequest 'https://aka.ms/vs/17/release/vs_BuildTools.exe' -OutFile "$buildRoot\vs.exe"
  $setup = Start-Process "$buildRoot\vs.exe" -ArgumentList '--quiet --wait --norestart --nocache --installPath C:\BuildTools --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended' -Wait -PassThru
  if ($setup.ExitCode -notin @(0,3010)) { throw "VS installer exit $($setup.ExitCode)" }
  $gitRelease = Invoke-RestMethod 'https://api.github.com/repos/git-for-windows/git/releases/latest'
  $gitAsset = $gitRelease.assets | Where-Object { $_.name -match '^Git-[0-9.]+-64-bit.exe$' } | Select-Object -First 1
  if (!$gitAsset) { throw 'Git installer not found' }
  Invoke-WebRequest $gitAsset.browser_download_url -OutFile "$buildRoot\git.exe"
  $setup = Start-Process "$buildRoot\git.exe" -ArgumentList '/VERYSILENT /NORESTART /NOCANCEL /SP-' -Wait -PassThru
  if ($setup.ExitCode -ne 0) { throw "Git installer exit $($setup.ExitCode)" }
  & 'C:\Program Files\Git\cmd\git.exe' clone --depth 1 --branch 3.47.4 https://github.com/flutter/flutter.git C:\flutter
  if ($LASTEXITCODE -ne 0) { throw 'Flutter checkout failed' }
  $env:Path = 'C:\flutter\bin;C:\Program Files\Git\cmd;' + $env:Path
  & 'C:\Program Files\Git\cmd\git.exe' config --global --add safe.directory C:/flutter
  & C:\flutter\bin\flutter.bat config --no-analytics
  & C:\flutter\bin\flutter.bat doctor -v
  if ($LASTEXITCODE -ne 0) { throw 'Flutter doctor failed' }
  Set-Content "$buildRoot\ready.txt" 'ready'
} catch { $_ | Out-File "$buildRoot\failed.txt"; throw }
finally { Stop-Transcript }
