@echo off
setlocal EnableDelayedExpansion

rem Compute SHA256SUMS.txt for the Windows installer payload.
rem Uses certutil (built into Windows) to avoid extra dependencies.

cd /d "%~dp0"
cd /d "%~dp0\.."

set "PAYLOAD_DIR=release-package"
cd /d "%PAYLOAD_DIR%"

if exist "SHA256SUMS.txt" del /f /q "SHA256SUMS.txt" >nul 2>&1

for %%f in (dbc-installer.exe dbc-node.exe dbc-ui.exe genesis.json peers.enc README.txt USER_GUIDE.txt DBC_Node_README.pdf) do (
  set "hash="
  for /f "skip=1 tokens=1" %%h in ('certutil -hashfile "%%f" SHA256') do (
    if not defined hash set "hash=%%h"
  )
  echo !hash!  %%f>>SHA256SUMS.txt
)

endlocal

