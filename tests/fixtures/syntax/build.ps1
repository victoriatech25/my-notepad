<#
  빌드 스크립트 (여러 줄 주석)
#>
param([string]$Config = "Release")

$files = Get-ChildItem -Path .\src -Filter *.rs
foreach ($f in $files) {
    Write-Host "검사: $($f.Name)"   # 한 줄 주석
}
if ($LASTEXITCODE -ne 0) { exit 1 }
