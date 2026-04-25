@echo off
setlocal
cd /d "%~dp0\.."

REM Clean previous package artifacts
if exist dist rmdir /s /q dist

echo Building RepoRocket with Cargo...
cargo build --release
if errorlevel 1 (
    echo Cargo build failed
    exit /b 1
)

mkdir dist

if exist target\release\reporocket.exe (
    copy target\release\reporocket.exe dist\RepoRocket.exe
) else (
    echo target\release\reporocket.exe not found
    exit /b 1
)

if exist img (
    xcopy img dist\img /E /I /Y
) else (
    echo img folder not found
)

echo Build completed successfully.
