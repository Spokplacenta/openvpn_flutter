cd c:\Users\info18\Dev\openvpn_flutter

# S'assurer qu'on est sur dev_branch
git checkout dev_branch 2>&1 | Out-Null

# Fetch
git fetch origin 2>&1 | Out-Null

# Afficher l'état
Write-Host "=== État Git ===" -ForegroundColor Cyan
git status

Write-Host "`n=== Commits locaux non poussés ===" -ForegroundColor Cyan
$unpushed = git log origin/dev_branch..HEAD --oneline
if ($unpushed) {
    Write-Host $unpushed
} else {
    Write-Host "Aucun commit à pousser"
}

Write-Host "`n=== Tentative de push ===" -ForegroundColor Cyan
$pushResult = git push origin dev_branch 2>&1
Write-Host $pushResult
Write-Host "Code de sortie: $LASTEXITCODE" -ForegroundColor $(if ($LASTEXITCODE -eq 0) { "Green" } else { "Red" })
