$ErrorActionPreference = "Continue"
cd c:\Users\info18\Dev\openvpn_flutter

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Push vers dev_branch sur GitHub" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 1. S'assurer qu'on est sur dev_branch
$currentBranch = git rev-parse --abbrev-ref HEAD
Write-Host "[1/5] Branche actuelle: $currentBranch" -ForegroundColor Yellow

if ($currentBranch -ne "dev_branch") {
    Write-Host "      -> Passage sur dev_branch..." -ForegroundColor Yellow
    git checkout dev_branch
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ERREUR: Impossible de passer sur dev_branch" -ForegroundColor Red
        exit 1
    }
}

# 2. Fetch pour avoir les dernières infos
Write-Host ""
Write-Host "[2/5] Recuperation des dernieres modifications..." -ForegroundColor Yellow
git fetch origin
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERREUR: Impossible de fetch depuis origin" -ForegroundColor Red
    exit 1
}

# 3. Verifier les modifications non commitees
Write-Host ""
Write-Host "[3/5] Verification des modifications non commitees..." -ForegroundColor Yellow
$uncommitted = git status --porcelain
if ($uncommitted) {
    Write-Host "ATTENTION: Modifications non commitees detectees:" -ForegroundColor Red
    Write-Host $uncommitted -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Il faut committer ces modifications avant de pousser." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Souhaites-tu les committer maintenant? (y/n)" -ForegroundColor Cyan
    $response = Read-Host
    if ($response -eq "y" -or $response -eq "Y") {
        Write-Host ""
        Write-Host "Ajout de tous les fichiers modifies..." -ForegroundColor Cyan
        git add .
        
        Write-Host ""
        Write-Host "Veuillez entrer un message de commit (format Conventional Commits en anglais):" -ForegroundColor Cyan
        Write-Host "Exemples:" -ForegroundColor Gray
        Write-Host "  feat: add new feature" -ForegroundColor Gray
        Write-Host "  fix: resolve bug" -ForegroundColor Gray
        Write-Host "  refactor: restructure code" -ForegroundColor Gray
        Write-Host "  docs: update documentation" -ForegroundColor Gray
        Write-Host "  test: add tests" -ForegroundColor Gray
        Write-Host "  chore: update dependencies" -ForegroundColor Gray
        
        Write-Host ""
        $commitMsg = Read-Host "Message de commit"
        if ([string]::IsNullOrWhiteSpace($commitMsg)) {
            Write-Host "ERREUR: Message de commit vide" -ForegroundColor Red
            exit 1
        }
        
        git commit -m $commitMsg
        if ($LASTEXITCODE -ne 0) {
            Write-Host "ERREUR: Le commit a echoue" -ForegroundColor Red
            exit 1
        }
        Write-Host "Commit cree avec succes" -ForegroundColor Green
    } else {
        Write-Host "Push annule. Committe d'abord tes modifications." -ForegroundColor Yellow
        exit 1
    }
} else {
    Write-Host "Aucune modification non commitee" -ForegroundColor Green
}

# 4. Verifier les commits a pousser
Write-Host ""
Write-Host "[4/5] Verification des commits a pousser..." -ForegroundColor Yellow
$unpushed = git log origin/dev_branch..HEAD --oneline
if ($unpushed) {
    Write-Host "Commits a pousser:" -ForegroundColor Cyan
    Write-Host $unpushed -ForegroundColor White
} else {
    Write-Host "Aucun commit a pousser. Tout est a jour." -ForegroundColor Green
    exit 0
}

# 5. Verifier si la branche distante existe
Write-Host ""
Write-Host "[5/5] Verification de la branche distante..." -ForegroundColor Yellow
$remoteBranchExists = git ls-remote --heads origin dev_branch
if (-not $remoteBranchExists) {
    Write-Host "La branche distante n'existe pas encore. Creation avec --set-upstream..." -ForegroundColor Yellow
    git push -u origin dev_branch
} else {
    # Verifier si la branche locale est en retard
    $localCommit = git rev-parse HEAD
    $remoteCommit = git rev-parse origin/dev_branch 2>$null
    
    if ($remoteCommit) {
        $isBehind = git rev-list --count HEAD..origin/dev_branch
        if ($isBehind -gt 0) {
            Write-Host "ATTENTION: La branche distante est en avance de $isBehind commit(s)." -ForegroundColor Red
            Write-Host ""
            Write-Host "Options:" -ForegroundColor Cyan
            Write-Host "  1. Pull et merge (recommandé)" -ForegroundColor White
            Write-Host "  2. Pull et rebase" -ForegroundColor White
            Write-Host "  3. Force push (DANGEREUX - ecrasera les commits distants)" -ForegroundColor Red
            Write-Host ""
            $choice = Read-Host "Ton choix (1/2/3)"
            
            switch ($choice) {
                "1" {
                    Write-Host ""
                    Write-Host "Pull avec merge..." -ForegroundColor Cyan
                    git pull origin dev_branch
                    if ($LASTEXITCODE -ne 0) {
                        Write-Host "ERREUR: Le pull a echoue. Resous les conflits manuellement." -ForegroundColor Red
                        exit 1
                    }
                }
                "2" {
                    Write-Host ""
                    Write-Host "Pull avec rebase..." -ForegroundColor Cyan
                    git pull --rebase origin dev_branch
                    if ($LASTEXITCODE -ne 0) {
                        Write-Host "ERREUR: Le rebase a echoue. Resous les conflits manuellement." -ForegroundColor Red
                        exit 1
                    }
                }
                "3" {
                    Write-Host ""
                    Write-Host "Force push (DANGEREUX)..." -ForegroundColor Red
                    git push --force origin dev_branch
                    exit $LASTEXITCODE
                }
                default {
                    Write-Host "Choix invalide. Push annule." -ForegroundColor Yellow
                    exit 1
                }
            }
        }
    }
    
    # Push normal
    Write-Host ""
    Write-Host "Push vers origin/dev_branch..." -ForegroundColor Cyan
    git push origin dev_branch
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "Push reussi!" -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "Le push a echoue (code: $LASTEXITCODE)" -ForegroundColor Red
        Write-Host ""
        Write-Host "Verifie:" -ForegroundColor Yellow
        Write-Host "  - Tes permissions sur le depot GitHub" -ForegroundColor White
        Write-Host "  - Ta connexion SSH (si tu utilises git@github.com)" -ForegroundColor White
        Write-Host "  - Si la branche distante existe" -ForegroundColor White
        exit 1
    }
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Termine!" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
