param([ValidateSet('windows','darwin')][string]$OS = 'windows', [ValidateSet('amd64','arm64')][string]$Arch = 'amd64', [switch]$Automation)
$ErrorActionPreference='Stop'
$env:GOTOOLCHAIN='go1.27.2'
$env:GOOS=$OS
$env:GOARCH=$Arch
$env:CGO_ENABLED='0'
New-Item -ItemType Directory -Force build | Out-Null
$name=if ($OS -eq 'windows') {'fastrock.exe'} else {'fastrock'}
$flags=if ($OS -eq 'windows') {'-H=windowsgui -s -w'} else {'-s -w'}
$buildArgs = @('build', '-trimpath')
if ($Automation) { $buildArgs += '-tags=fastrock_automation' }
go @buildArgs -ldflags $flags -o "build/$name" ./cmd/fastrock
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
