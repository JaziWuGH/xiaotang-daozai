# 安装 / 更新 Jazi 定制岛仔
$ErrorActionPreference = 'Stop'
$destDir = Join-Path $env:LOCALAPPDATA 'Programs\Daozai'
$destExe = Join-Path $destDir 'daozai.exe'
New-Item -ItemType Directory -Force -Path $destDir | Out-Null

$api = 'https://api.github.com/repos/JaziWuGH/daozai/releases/latest'
$release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'daozai-install' }
$asset = $release.assets | Where-Object { $_.name -like '*.zip' } | Select-Object -First 1
if (-not $asset) { throw 'Release 里没有 zip 安装包' }

$zip = Join-Path $env:TEMP ('Daozai-' + $release.tag_name + '.zip')
Write-Host ('下载 ' + $asset.browser_download_url)
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -Headers @{ 'User-Agent' = 'daozai-install' }

$extract = Join-Path $env:TEMP ('Daozai-extract-' + $release.tag_name)
if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
$exe = Get-ChildItem $extract -Recurse -Filter daozai.exe | Select-Object -First 1
if (-not $exe) { throw 'zip 里找不到 daozai.exe' }

Get-Process daozai -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Copy-Item $exe.FullName $destExe -Force
Write-Host ('已安装到 ' + $destExe + ' (' + (Get-Item $destExe).Length + ' bytes)')
Start-Process $destExe
