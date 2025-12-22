$ErrorActionPreference = "Stop"
cd c:\Users\info18\Dev\openvpn_flutter

Write-Host "=== Diagnostic Git ===" -ForegroundColor Cyan

# Vérifier la branche actuelle
$currentBranch = git rev-parse --abbrev-ref HEAD
Write-Host "Branche actuelle: $currentBranch" -ForegroundColor Yellow

# S'assurer qu'on est sur dev_branch
if ($currentBranch -ne "dev_branch") {
    Write-Host "Passage sur dev_branch..." -ForegroundColor Yellow
    git checkout dev_branch
}

# Fetch pour avoir les dernières infos
Write-Host "`nRécupération des dernières modifications..." -ForegroundColor Cyan
git fetch origin

# Vérifier les modifications non commitées
Write-Host "`nVérification des modifications non commitées..." -ForegroundColor Cyan
$status = git status --porcelain
if ($status) {
    Write-Host "ATTENTION: Il y a des modifications non commitées:" -ForegroundColor Red
    Write-Host $status
    Write-Host "`nIl faut committer ces modifications avant de pousser." -ForegroundColor Yellow
    exit 1
} else {
    Write-Host "Aucune modification non commitée." -ForegroundColor Green
}

# Vérifier les commits non poussés
Write-Host "`nVérification des commits non poussés..." -ForegroundColor Cyan
$unpushed = git log origin/dev_branch..HEAD --oneline
if ($unpushed) {
    Write-Host "Commits à pousser:" -ForegroundColor Yellow
    Write-Host $unpushed
} else {
    Write-Host "Aucun commit à pousser. Tout est à jour." -ForegroundColor Green
    exit 0
}

# Essayer de pousser
Write-Host "`nTentative de push vers origin/dev_branch..." -ForegroundColor Cyan
try {
    git push origin dev_branch 2>&1 | Tee-Object -Variable pushOutput
    if ($LASTEXITCODE -eq 0) {
        Write-Host "`nPush réussi!" -ForegroundColor Green
    } else {
        Write-Host "`nErreur lors du push (code: $LASTEXITCODE)" -ForegroundColor Red
        Write-Host $pushOutput
    }
} catch {
    Write-Host "`nException lors du push:" -ForegroundColor Red
    Write-Host $_.Exception.Message
}
