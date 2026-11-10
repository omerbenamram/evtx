Your task is to help pull latest changes from master and rebase branch on top of it.

# Workflow

1. Stash any uncommitted changes: `git stash push -m "Stashing changes before rebase"`
2. Fetch latest: `git fetch origin`
3. Update master: `git checkout master && git pull origin master`
4. Rebase branch (use --onto with fork-point to replay only your branch commits on top of master — fewer conflicts):
   `git checkout <branch> && GIT_EDITOR=true git rebase --onto origin/master "$(git merge-base --fork-point origin/master HEAD)"`
5. Restore stashed changes: `git stash pop`

# Important

- When running git rebase/commit operations, use `GIT_EDITOR=true` to prevent interactive editors from opening
- Do not force push without asking for confirmation first.
    - When asking for confirmation, you should explain each conflict and how it was solved in a sentence. And also if they were resolved generally by using 'master' (theirs) or 'branch' (ours) version. or a combination of both with an explanation.
    - A cloud session cannot force-push at all. There, merge `master` into the branch instead of rebasing a pushed branch (see "How a change ships" in `AGENTS.md`).
- When encountering merge conflicts:
    - For `Cargo.lock`: take master's version (`git checkout --theirs Cargo.lock`), then run `cargo check` to add what the branch needs
    - For `CHANGELOG.md`: keep the entries of both sides
    - For `evtx-wasm/evtx-viewer/bun.lock`: Use `git checkout --theirs` on it, then `bun install` in that folder to regenerate
    - For other files: Review conflicts and resolve appropriately, or ask for clarification
- If you are not sure about the changes, you should ask me for clarification.
