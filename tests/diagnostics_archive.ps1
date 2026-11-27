param([string]$Executable)

$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
if (!$Executable) { $Executable = Join-Path $project 'target\x86_64-pc-windows-msvc\release\daozai.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$target = (New-Item -Path (Join-Path $project 'target') -ItemType Directory -Force).FullName
$root = Join-Path $target ("诊断测试 " + [Guid]::NewGuid().ToString('N'))
$logs = Join-Path $root 'data\diagnostics'
$packages = Join-Path $root 'out'
New-Item -ItemType Directory -Path $logs -Force | Out-Null
$previousDataDir = $env:DAOZAI_DATA_DIR
try {
    $fakePath = Join-Path $env:USERPROFILE 'DaozaiFixture\player.exe'
    $json = @{ Path = $fakePath } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $logs 'resources.log'), $json)
    [IO.File]::WriteAllText((Join-Path $logs 'frames.log'), 'frame-current')
    [IO.File]::WriteAllText((Join-Path $logs 'frames.previous.log'), 'frame-previous')
    [IO.File]::WriteAllText((Join-Path $logs 'unlisted.log'), 'not-for-diagnostics')
    $env:DAOZAI_DATA_DIR = Split-Path $logs
    $process = Start-Process -FilePath $Executable -ArgumentList @('--collect-diagnostics', '--diagnostics-output-dir', ('"' + $packages + '"')) -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(60000)) {
        Stop-Process -InputObject $process -ErrorAction SilentlyContinue
        throw '诊断入口执行超时'
    }
    if ($process.ExitCode -ne 0) { throw "诊断入口退出码为 $($process.ExitCode)" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $files = @(Get-ChildItem -LiteralPath $packages -Filter '*.zip')
    if ($files.Count -ne 1) { throw '收集一次应只产出一份诊断包' }
    $zip = [IO.Compression.ZipFile]::OpenRead($files[0].FullName)
    try {
        $reader = [IO.StreamReader]::new($zip.GetEntry('resources.log').Open())
        try { $masked = ($reader.ReadToEnd() | ConvertFrom-Json).Path } finally { $reader.Dispose() }
        if ($masked -ne '%USERPROFILE%\DaozaiFixture\player.exe') { throw '诊断包中的 JSON 路径没有隐去用户目录' }
        foreach ($name in @('frames.log','frames.previous.log')) {
            if (!$zip.GetEntry($name)) { throw "诊断包缺少 $name" }
        }
        if ($zip.GetEntry('unlisted.log')) { throw '诊断包包含未列入白名单的日志' }
        $reader = [IO.StreamReader]::new($zip.GetEntry('environment.json').Open())
        try { $info = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
        if (!$info.CollectedAt -or !$info.System.Build) { throw '诊断包缺少采集时间或系统版本' }
    } finally { $zip.Dispose() }
    Write-Output '通过：诊断归档、转义路径脱敏、当前与轮转慢帧日志'
} finally {
    $env:DAOZAI_DATA_DIR = $previousDataDir
    foreach ($dir in @($packages,$logs)) {
        if (Test-Path -LiteralPath $dir) {
            Get-ChildItem -LiteralPath $dir -File | Remove-Item -Force
            [IO.Directory]::Delete($dir)
        }
    }
    [IO.Directory]::Delete((Split-Path $logs))
    [IO.Directory]::Delete($root)
}
