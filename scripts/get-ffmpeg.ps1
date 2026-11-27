# ffmpeg(Windows x64) 를 받아 Tauri sidecar 위치에 놓습니다.
#   powershell -ExecutionPolicy Bypass -File scripts\get-ffmpeg.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe"
if (Test-Path $dest) { Write-Host "이미 있음: $dest"; exit 0 }

$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
$tmp = Join-Path $env:TEMP "ffmpeg-dl"
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null

Write-Host "ffmpeg 다운로드 중..."
Invoke-WebRequest $url -OutFile "$tmp\ffmpeg.zip"
Expand-Archive "$tmp\ffmpeg.zip" -DestinationPath $tmp
$exe = Get-ChildItem $tmp -Recurse -Filter ffmpeg.exe | Select-Object -First 1
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
Copy-Item $exe.FullName $dest
Remove-Item $tmp -Recurse -Force
Write-Host "완료: $dest"
