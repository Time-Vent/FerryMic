# SoundBridge launcher - output to VIRTUAL CABLE (inject into mic for gaming)
$env:CARGO_TARGET_DIR = "C:\Users\susha\AppData\Local\Temp\sb-target"
$env:PATH = "C:\Users\susha\.cargo\bin;C:\Users\susha\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;" + $env:PATH
$env:SOUNDBRIDGE_OUTPUT_DEVICE = "CABLE Input"
# Microphone: pick it in the web UI (remembered across restarts). This env var
# overrides the saved choice (containment match).
# NOTE: must NOT be "CABLE Output" (that would loop the output back into itself).
# $env:SOUNDBRIDGE_INPUT_DEVICE = "Microphone"
Set-Location $PSScriptRoot
Start-Process "http://127.0.0.1:8321"
& "C:\Users\susha\AppData\Local\Temp\sb-target\release\soundbridge-server.exe"
