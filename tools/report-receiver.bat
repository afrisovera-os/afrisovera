@echo off
rem Start the AfriSovera field-report receiver (receive-only). Close this window to stop it.
python "%~dp0report-receiver.py" %*
pause
