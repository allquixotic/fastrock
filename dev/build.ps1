param([ValidateSet('windows','darwin')][string]$OS = 'windows', [ValidateSet('amd64','arm64')][string]$Arch = 'amd64')
$ErrorActionPreference='Stop'
$env:GOTOOLCHAIN='go1.26.0'
$env:GOOS=$OS
$env:GOARCH=$Arch
$env:CGO_ENABLED=if ($OS -eq 'windows') {'0'} else {'1'}
New-Item -ItemType Directory -Force build | Out-Null
$name=if ($OS -eq 'windows') {'fastrock.exe'} else {'fastrock'}
$flags=if ($OS -eq 'windows') {'-H=windowsgui -s -w'} else {'-s -w'}
go build -trimpath -ldflags $flags -o "build/$name" ./cmd/fastrock
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
