param(
    [string]$PrivateKeyPath = (Join-Path $PSScriptRoot "..\private-key.pem"),
    [string]$PublicKeyPath = (Join-Path $PSScriptRoot "..\public-key.pem")
)

$ErrorActionPreference = "Stop"

function Find-OpenSsl {
    $cmd = Get-Command openssl -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $candidates = @(
        "C:\Program Files\Git\usr\bin\openssl.exe",
        "C:\Program Files\Git\mingw64\bin\openssl.exe",
        "C:\OpenSSL-Win64\bin\openssl.exe"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    throw "openssl was not found. Install Git for Windows or OpenSSL, then run this script again."
}

$PrivateKeyPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($PrivateKeyPath)
$PublicKeyPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($PublicKeyPath)

if (Test-Path $PrivateKeyPath) {
    throw "Private key already exists: $PrivateKeyPath"
}

$openssl = Find-OpenSsl

& $openssl ecparam -name prime256v1 -genkey -noout -out $PrivateKeyPath
& $openssl ec -in $PrivateKeyPath -pubout -out $PublicKeyPath

Write-Host "Created:"
Write-Host "  $PrivateKeyPath"
Write-Host "  $PublicKeyPath"
Write-Host ""
Write-Host "Add the private key content to GitHub repository secret PRIVATE_KEY."
Write-Host "Commit public-key.pem, but never commit private-key.pem."

