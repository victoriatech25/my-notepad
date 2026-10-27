@echo off
REM 실행 스크립트
:: 다른 주석 형식
setlocal
set APP_HOME=%~dp0
if not exist "%APP_HOME%app.exe" (
    echo 실행 파일이 없습니다
    exit /b 1
)
for %%i in (*.txt) do echo %%i
endlocal
