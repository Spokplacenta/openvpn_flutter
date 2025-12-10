$ErrorActionPreference = "Continue"
cd c:\Users\info18\Dev\openvpn_flutter

$output = @()

$output += "=== Current branch ==="
$output += git rev-parse --abbrev-ref HEAD

$output += "`n=== Git status ==="
$output += git status

$output += "`n=== Uncommitted changes ==="
$uncommitted = git status --porcelain
if ($uncommitted) {
    $output += $uncommitted
} else {
    $output += "No uncommitted changes"
}

$output += "`n=== Local commits (last 5) ==="
$output += git log --oneline -5

$output += "`n=== Unpushed commits ==="
$unpushed = git log origin/dev_branch..HEAD --oneline
if ($unpushed) {
    $output += $unpushed
} else {
    $output += "No unpushed commits"
}

$output += "`n=== Remote branches ==="
$output += git branch -r

$output += "`n=== Attempting push (with error capture) ==="
try {
    $pushOutput = git push origin dev_branch 2>&1 | Out-String
    $output += $pushOutput
    $output += "Exit code: $LASTEXITCODE"
} catch {
    $output += "Error: $_"
}

$output | Out-File -FilePath "diagnose_output.txt" -Encoding utf8
$output | Write-Host
