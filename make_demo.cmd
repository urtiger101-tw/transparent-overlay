@echo off
setlocal
cd /d "%~dp0"
where py >nul 2>nul
if errorlevel 1 (
  set "PYTHON=python"
) else (
  set "PYTHON=py -3"
)
%PYTHON% -c "import PIL" >nul 2>nul
if errorlevel 1 (
  echo Python or Pillow not found.
  echo Install Python, then run: python -m pip install -r requirements.txt
  pause
  exit /b 1
)
set "OUT=demo_output_%RANDOM%_%RANDOM%"
%PYTHON% make_overlay.py --preview --keep-frames --out "%OUT%"
if errorlevel 1 (
  echo Failed. Check the message above and README.md.
) else (
  echo Generated folder: %OUT%
)
pause
