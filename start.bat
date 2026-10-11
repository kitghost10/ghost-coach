@echo off
cd /d "%~dp0"
if not exist "bin\sonic-sales.exe" (
  echo Building Ghost Coach so it can open without a local server...
  call npm run desktop
  if errorlevel 1 exit /b 1
  mkdir bin 2>nul
  copy /Y "src-tauri\target\debug\sonic-sales.exe" "bin\sonic-sales.exe" >nul
)
start "" "%~dp0bin\sonic-sales.exe"
