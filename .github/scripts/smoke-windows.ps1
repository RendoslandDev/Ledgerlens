# Installs the NSIS build silently and runs the IPC smoke test against the installed app.
$ErrorActionPreference = 'Stop'
$installer = Get-ChildItem apps/desktop/src-tauri/target/release/bundle/nsis/*.exe | Select-Object -First 1
if (-not $installer) { throw 'no NSIS installer was built' }
Write-Host "installing $($installer.Name)"
$install = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
Write-Host "installer exit code: $($install.ExitCode)"
$root = Join-Path $env:LOCALAPPDATA 'Ledgerlens'
if (Test-Path $root) { Get-ChildItem $root -Recurse -File | ForEach-Object { Write-Host "  $($_.FullName)" } }
$exe = Get-ChildItem $root -Filter *.exe -Recurse -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -notlike 'uninstall*' } | Select-Object -First 1
if (-not $exe) { throw "the installer did not put the app in $root" }
Write-Host "launching $($exe.FullName)"
node apps/desktop/scripts/smoke-ipc.mjs $exe.FullName crates/ll-app/tests/fixtures/invoice-cups.pdf
exit $LASTEXITCODE
