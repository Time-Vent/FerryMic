# 构建 FerryMic 桌面 App，输出到 dist-app\FerryMic\（双击 FerryMic.exe 即可运行）。
# Build the FerryMic desktop app into dist-app\FerryMic\ (double-click FerryMic.exe).
#
# 步骤 Steps: 前端构建 build front-end -> cargo release 编译 App -> 复制 exe + WebView2Loader.dll
# 用法 Usage:  powershell -ExecutionPolicy Bypass -File .\build-app.ps1
$ErrorActionPreference = "Stop"

# 工具链路径（与 start-*.ps1 一致）。Toolchain paths (same as start-*.ps1).
$env:CARGO_TARGET_DIR = "C:\Users\susha\AppData\Local\Temp\sb-target"
$env:PATH = "C:\Users\susha\.cargo\bin;C:\Users\susha\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;C:\Program Files\nodejs;$env:APPDATA\npm;" + $env:PATH
Set-Location $PSScriptRoot

# 1. 前端（会被嵌入可执行文件）。Front-end (embedded into the executable).
Push-Location apps\web
pnpm install --frozen-lockfile
pnpm build
if ($LASTEXITCODE -ne 0) { throw "front-end build failed" }
Pop-Location

# 2. App。
cargo build --release -p soundbridge-app
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# 3. 打包目录：GNU 工具链构建的 App 需要 WebView2Loader.dll 放在 exe 旁边。
#    Package folder: a GNU-built app needs WebView2Loader.dll next to the exe.
$out = Join-Path $PSScriptRoot "dist-app\FerryMic"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "$env:CARGO_TARGET_DIR\release\FerryMic.exe" $out -Force
Copy-Item "$env:CARGO_TARGET_DIR\release\WebView2Loader.dll" $out -Force
Get-ChildItem $out | Select-Object Name, @{n = "MB"; e = { [math]::Round($_.Length / 1MB, 2) } }
Write-Host "OK -> $out"
