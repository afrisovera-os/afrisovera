@echo off
rem Copy the AfriSovera field reports saved on its USB stick into build\field-reports.
rem It only reads the stick; it never deletes or changes anything on it.
set "DRIVE=%~1"
if "%DRIVE%"=="" set /p "DRIVE=Drive letter of the AfriSovera USB stick (for example E): "
if "%DRIVE%"=="" goto :none
set "DRIVE=%DRIVE::=%"
set "DRIVE=%DRIVE:\=%"
python "%~dp0report-receiver.py" --from-stick "%DRIVE%:" %2 %3 %4 %5 %6 %7 %8 %9
pause
goto :eof
:none
echo No drive letter given; nothing copied.
pause
