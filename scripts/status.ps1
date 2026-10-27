#Requires -Version 5.1
<#
.SYNOPSIS
    Show which worktree works on what, and which slices can be built next.

.DESCRIPTION
    Lists every git worktree with its branch, uncommitted changes, last
    commit and the pull request of that branch. A worktree whose PR is merged
    or closed and that has no uncommitted changes is marked "stale".

    Then lists the open slice issues (label "slice") that have no open
    blockers: the slices that can be built next. The full picture is the
    "lvpm mission" project: https://github.com/users/ThatsTheEnd/projects/2

    Needs git and an authenticated gh.
#>
$ErrorActionPreference = 'Stop'

$prs = gh pr list --state all --limit 300 --json headRefName,number,state,title | ConvertFrom-Json

$rows = @()
$wt = $null
foreach ($line in (git worktree list --porcelain) + '') {
    if ($line -like 'worktree *') { $wt = @{ Path = $line.Substring(9); Branch = '(detached)' } }
    elseif ($line -like 'branch refs/heads/*') { $wt.Branch = $line.Substring(18) }
    elseif ($line -eq '' -and $wt) {
        # Agent metrics and review reports change in every worktree; they are not work in progress.
        $dirty = @(git -C $wt.Path status --porcelain -- . ':!.claude/metrics' ':!.dev-team-reports').Count
        $age = git -C $wt.Path log -1 --format=%cr
        # The newest PR of the branch; a branch name can be reused.
        $pr = $prs | Where-Object headRefName -eq $wt.Branch | Sort-Object number -Descending | Select-Object -First 1
        $prText = if ($pr) { "#$($pr.number) $($pr.state) $($pr.title)" } else { '(no PR)' }
        git merge-base --is-ancestor $wt.Branch origin/main 2>$null
        $inMain = $LASTEXITCODE -eq 0 -and $wt.Branch -ne 'main'
        $stale = (($pr -and $pr.state -ne 'OPEN') -or $inMain) -and $dirty -eq 0
        $rows += [pscustomobject]@{
            Worktree = Split-Path $wt.Path -Leaf
            Branch   = $wt.Branch
            Dirty    = $dirty
            Last     = $age
            PR       = $prText
            Note     = if ($stale) { 'stale' } else { '' }
        }
        $wt = $null
    }
}
'== Worktrees'
$rows | Format-Table -AutoSize | Out-String -Width 200

'== Ready to build (open slices, no open blockers)'
$issues = gh api 'repos/{owner}/{repo}/issues?labels=slice&state=open&per_page=100' | ConvertFrom-Json
$ready = $issues | Where-Object {
    $_.issue_dependencies_summary.blocked_by -eq 0 -and $_.sub_issues_summary.total -eq 0
}
$ready | Sort-Object number | ForEach-Object { "  #$($_.number)  $($_.title)  [$($_.milestone.title)]" }
