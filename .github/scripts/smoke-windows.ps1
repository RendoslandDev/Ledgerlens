# Windows smoke test, in two parts.
#  1. The installer as shipped: installs silently, the app launches, stays up, and creates its data.
#  2. The real IPC end to end: the same source rebuilt with WebView2's debug port open, then driven
#     by smoke-ipc.mjs (import a text PDF and wait for it to be read). The port can only be opened
#     through the window's additionalBrowserArgs: the app's own browser arguments replace both the
#     WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS variable and the loader-override registry key.
$ErrorActionPreference = 'Stop'
$data = Join-Path $env:LOCALAPPDATA 'com.ledgerlens.app'

$installer = Get-ChildItem apps/desktop/src-tauri/target/release/bundle/nsis/*.exe | Select-Object -First 1
if (-not $installer) { throw 'no NSIS installer was built' }
Write-Host "installing $($installer.Name)"
$install = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
if ($install.ExitCode -ne 0) { throw "the installer exited with $($install.ExitCode)" }
$exe = Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Ledgerlens') -Filter *.exe -Recurse |
  Where-Object { $_.Name -notlike 'uninstall*' } | Select-Object -First 1
if (-not $exe) { throw 'the installer did not put the app in %LOCALAPPDATA%\Ledgerlens' }

Write-Host "launching the installed $($exe.FullName)"
$app = Start-Process -FilePath $exe.FullName -PassThru
for ($i = 0; $i -lt 30 -and -not (Test-Path (Join-Path $data 'ledgerlens.db')); $i++) { Start-Sleep 1 }
Start-Sleep 5
if ($app.HasExited) { throw "the installed app exited with $($app.ExitCode)" }
if (-not (Test-Path (Join-Path $data 'ledgerlens.db'))) { throw "the installed app created no database in $data" }
if (-not (Test-Path (Join-Path $data 'EBWebView'))) { throw 'the installed app started no WebView2' }
Stop-Process -Id $app.Id
Write-Host 'the installed app launched, created its database, and started WebView2'

$config = Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$window = $config.app.windows[0]
$window | Add-Member -NotePropertyName additionalBrowserArgs -Force -NotePropertyValue `
  '--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=9333'
$overlay = Join-Path $env:RUNNER_TEMP 'smoke-config.json'
@{ app = @{ windows = @($window) } } | ConvertTo-Json -Depth 10 | Set-Content $overlay
Write-Host 'building the app with the debug port open'
Push-Location apps/desktop
npx tauri build --no-bundle --config $overlay
$built = $LASTEXITCODE
Pop-Location
if ($built -ne 0) { throw "the debug-port build failed with $built" }

node apps/desktop/scripts/smoke-ipc.mjs apps/desktop/src-tauri/target/release/ledgerlens-desktop.exe `
  crates/ll-app/tests/fixtures/invoice-cups.pdf
exit $LASTEXITCODE
