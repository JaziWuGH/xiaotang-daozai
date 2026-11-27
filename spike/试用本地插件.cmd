@echo off
setlocal
pushd "%~dp0.."
set "DAOZAI_DATA_DIR=%CD%\target\local-trial"
set "trial_exe=target\x86_64-pc-windows-msvc\debug\daozai.exe"
powershell -NoProfile -ExecutionPolicy Bypass -File "spike/cargo-serial.ps1" build --locked --features sdk-js --bin daozai --target x86_64-pc-windows-msvc
if errorlevel 1 exit /b 1
start "" "%trial_exe%"
popd
