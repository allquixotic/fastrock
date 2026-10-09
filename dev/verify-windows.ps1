param([Parameter(Mandatory=$true)][string]$Path)
$ErrorActionPreference = 'Stop'
$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne 'Valid') {
    throw "Authenticode verification failed: $($signature.Status) $($signature.StatusMessage)"
}
if ($null -eq $signature.TimeStamperCertificate) {
    throw 'The release executable needs a trusted timestamp.'
}
if ($env:FASTROCK_WINDOWS_PUBLISHER -and $signature.SignerCertificate.Subject -cne $env:FASTROCK_WINDOWS_PUBLISHER) {
    throw 'The release executable was signed by an unexpected publisher.'
}
Write-Output "Verified publisher: $($signature.SignerCertificate.Subject)"
