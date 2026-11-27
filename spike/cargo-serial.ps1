$cargoArguments = $args
$mutex = [Threading.Mutex]::new($false, 'Local\Daozai.Cargo')
try {
    try { $null = $mutex.WaitOne() }
    catch [Threading.AbandonedMutexException] { }
    while (Get-Process cargo -ErrorAction SilentlyContinue) { Start-Sleep -Seconds 2 }
    $env:CARGO_BUILD_JOBS = '1'
    & cargo +stable-x86_64-pc-windows-msvc @cargoArguments
    $code = $LASTEXITCODE
} finally {
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
exit $code
