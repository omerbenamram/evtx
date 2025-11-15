Your task is to help pull latest changes from main and rebase branch on top of it.

# Workflow

1. Stash any uncommitted changes: `git stash push -m "Stashing changes before rebase"`
2. Fetch latest: `git fetch origin`
3. Update main: `git checkout main && git pull origin main`
4. Rebase branch (use --onto with fork-point to replay only your branch commits on top of main — fewer conflicts):
   `git checkout <branch> && GIT_EDITOR=true git rebase --onto origin/main "$(git merge-base --fork-point origin/main HEAD)"`
5. Restore stashed changes: `git stash pop`

# Important

- When running git rebase/commit operations, use `GIT_EDITOR=true` to prevent interactive editors from opening
- Do not force push without asking for confirmation first.
    - When asking for confirmation, you should explain each conflict and how it was solved in a sentence. And also if they were resolved generally by using 'main' (theirs) or 'branch' (ours) version. or a combination of both with an explanation.
- When encountering merge conflicts:
    - For `bun.lock`: Use `git checkout --theirs bun.lock && bun install` to regenerate
    - For `package.json`/`turbo.json`: Usually accept main's version (`git checkout --theirs`)
    - For dependency-related conflicts: In most cases, accept updated dependencies from main
    - For other files: Review conflicts and resolve appropriately, or ask for clarification
- If you are not sure about the changes, you should ask me for clarification.
