$ErrorActionPreference = "Continue"
cd c:\Users\info18\Dev\openvpn_flutter

# S'assurer qu'on est sur dev_branch
$currentBranch = git rev-parse --abbrev-ref HEAD
if ($currentBranch -ne "dev_branch") {
    Write-Host "Passage sur dev_branch..." -ForegroundColor Yellow
    git checkout dev_branch
}

# Fetch
git fetch origin 2>&1 | Out-Null

# Vérifier les modifications non commitées
$uncommitted = git status --porcelain
if ($uncommitted) {
    Write-Host "ERREUR: Il y a des modifications non commitées!" -ForegroundColor Red
    Write-Host "Fichiers modifiés:" -ForegroundColor Yellow
    Write-Host $uncommitted
    Write-Host "`nIl faut d'abord committer ces modifications." -ForegroundColor Yellow
    Write-Host "Souhaites-tu les committer maintenant? (y/n)" -ForegroundColor Cyan
    $response = Read-Host
    if ($response -eq "y" -or $response -eq "Y") {
        Write-Host "Ajout de tous les fichiers..." -ForegroundColor Cyan
        git add .
        Write-Host "Veuillez entrer un message de commit (format Conventional Commits):" -ForegroundColor Cyan
        Write-Host "Exemples: feat: add feature, fix: resolve bug, refactor: restructure code" -ForegroundColor Gray
        $commitMsg = Read-Host "Message"
        git commit -m $commitMsg
    } else {
        Write-Host "Push annulé. Committe d'abord tes modifications." -ForegroundColor Yellow
        exit 1
    }
}

# Vérifier si la branche distante existe
$remoteExists = git ls-remote --heads origin dev_branch
if (-not $remoteExists) {
    Write-Host "La branche distante dev_branch n'existe pas encore." -ForegroundColor Yellow
    Write-Host "Création de la branche distante avec --set-upstream..." -ForegroundColor Cyan
    git push -u origin dev_branch 2>&1
    exit $LASTEXITCODE
}

# Vérifier si la branche locale est en retard
$localCommit = git rev-parse HEAD
$remoteCommit = git rev-parse origin/dev_branch 2>$null
if ($remoteCommit -and $localCommit -ne $remoteCommit) {
    $isBehind = git rev-list --count HEAD..origin/dev_branch
    if ($isBehind -gt 0) {
        Write-Host "ATTENTION: La branche distante est en avance de $isBehind commit(s)." -ForegroundColor Yellow
        Write-Host "Options:" -ForegroundColor Cyan
        Write-Host "1. Pull et merge" -ForegroundColor White
        Write-Host "2. Pull et rebase" -ForegroundColor White
        Write-Host "3. Force push (DANGEREUX)" -ForegroundColor Red
        $choice = Read-Host "Choix (1/2/3)"
        switch ($choice) {
            "1" {
                git pull origin dev_branch
            }
            "2" {
                git pull --rebase origin dev_branch
            }
            "3" {
                Write-Host "Force push..." -ForegroundColor Red
                git push --force origin dev_branch
                exit $LASTEXITCODE
            }
            default {
                Write-Host "Annulé." -ForegroundColor Yellow
                exit 1
            }
        }
    }
}

# Push normal
Write-Host "Push vers origin/dev_branch..." -ForegroundColor Cyan
git push origin dev_branch 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "Push réussi!" -ForegroundColor Green
} else {
    Write-Host "Le push a échoué. Code d'erreur: $LASTEXITCODE" -ForegroundColor Red
}
