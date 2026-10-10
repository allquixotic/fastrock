$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
python dev/check-build-policy.py
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo build --locked --bin fastrock @args
exit $LASTEXITCODE
