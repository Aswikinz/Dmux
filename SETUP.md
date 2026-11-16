# Dmux: setup and daily use

This package is the starting point for building Dmux with a team of OpenCode agents
on Arch Linux. It contains the plan, the agents, the Git workflow, the test databases
and a first proof of the comparison method. It does not yet contain the app itself:
the agents build that, phase by phase.

## What is in the package

| Path | What it is |
|---|---|
| `SETUP.md` | This guide |
| `docs/PLAN.md` | The full specification the agents work from |
| `docs/TASKS.md` | The task board, starting with Phase 0 |
| `docs/DECISIONS.md` | Decisions already made |
| `AGENTS.md` | Rules every agent follows. OpenCode loads it automatically. Local-only: never committed or pushed (decision 12); `wt-new.sh` copies it into each worktree |
| `.opencode/agents/` | The team: 13 files for 12 people (Arjun has a second, stricter file for the comparison core) |
| `opencode.json` | Default model |
| `scripts/` | Worktree scripts, Git setup, CI checks, Arch install |
| `.githooks/` | Commit rules: your identity only, no co-author lines, no private files |
| `.github/workflows/commit-rules.yml` | The same rules, checked on GitHub |
| `testdata/compose.yml` | Postgres 9.6 and 17 test databases for Podman |
| `testdata/phase0/` | The reference fingerprint queries and the script that proves them |
| `docs/dba/create_dmux_reader.sql` | Creates the SELECT-only login for a real database |
| `private/` | The parsed data dictionary. Never enters Git |

## 1. One-time setup

### 1.1 Unpack

```bash
mkdir -p ~/code && cd ~/code
unzip ~/Downloads/dmux-package.zip     # creates ~/code/dmux
cd dmux
```

### 1.2 Install the tools

```bash
bash scripts/arch-install.sh
```

Read the script first; it uses `sudo` for `pacman` only. If it adds sub-user ID ranges
for Podman, log out and back in once.

### 1.3 Git, with your name on every commit

```bash
git init -b main
git config --global user.name  "Your Name"
git config --global user.email "you@example.com"    # must be a verified email on GitHub
bash scripts/setup-git.sh
git add -A
git commit -m "Initial project setup"
git status --ignored --short | grep private         # should print: !! private/
```

`setup-git.sh` switches on the commit hooks for this folder and for every task working
copy, and records you as the only allowed author.

### 1.4 GitHub

```bash
gh auth login
gh repo create dmux-verify --private --source=. --remote=origin
git push -u origin main
```

Start private. Go through section 7 before you make it public. The repo is named
`dmux-verify` here because another open-source project is already called `dmux`; the
app itself can still be called Dmux.

### 1.5 OpenRouter

1. Create an API key at openrouter.ai and give it a **credit limit**. $30 is a sensible start.
2. Add it to your shell: `echo 'export OPENROUTER_API_KEY=sk-or-...' >> ~/.bashrc`, then open a new terminal.
3. Run `opencode`, then `/models`, and check that the OpenRouter models are listed.

### 1.6 Test databases and the first proof

```bash
podman compose -f testdata/compose.yml up -d
testdata/phase0/prove.sh
```

You should see `Result: 15 passed, 0 failed`. That is the comparison method working on
a Postgres 9.6 old database and a Postgres 17 new one.

## 2. What to give OpenCode

Nothing needs pasting. OpenCode finds everything in the folder:

- `AGENTS.md` is loaded into every agent as its rules.
- `.opencode/agents/*.md` become the team members.
- The agents open `docs/PLAN.md`, `docs/TASKS.md` and `docs/DECISIONS.md` themselves.

### Start a session

```bash
cd ~/code/dmux
opencode
```

Press **Tab** until the agent shown is **maya**. Then send this first message:

```
Read AGENTS.md, docs/PLAN.md, docs/TASKS.md and docs/DECISIONS.md.
Then start Phase 0 from the task board. Run one task at a time for now.
Stop and report to me when a task is blocked, before every merge, and
when the Phase 0 gate (T007) is ready for my review.
```

Maya opens a working copy per task, briefs the right teammate, has the work tested and
reviewed, and asks you before each merge.

### Later sessions

Every OpenCode session starts fresh. The task board and the decisions log are the
team's memory, so the message is simply:

```
Continue from docs/TASKS.md.
```

Once Phase 0 is done and you are comfortable, tell Maya she may run up to three tasks
in parallel.

### Useful things to say

| You want | Say |
|---|---|
| A status update | `Where are we? Summarise the board and anything blocked.` |
| One teammate directly | `@priya review testdata/phase0/fingerprint.sql for Postgres 9.4` |
| To change the plan | Edit `docs/PLAN.md` yourself, then tell Maya: `PLAN.md section 4.3 changed. Update the board.` |
| To stop a task | `Stop T012 and remove its working copy.` |

### Check the guard rails once

Before trusting the setup, ask a read-only teammate to break a rule:

```
@sam create a file called test.txt in the repo root
```

Sam should be refused. If the file appears, your OpenCode version is not applying the
`permissions:` blocks; see section 8.

## 3. Working copies (worktrees)

Each task lives in `.worktrees/<task ID>/` on its own branch, so agents do not collide.
Maya runs these for you, but you can too:

| Command | What it does |
|---|---|
| `scripts/wt-list.sh` | Show open working copies and unsaved work |
| `scripts/wt-new.sh T012 schema-diff` | Open a working copy for a task (three at most) |
| `scripts/in-wt.sh T012 cargo test` | Run an allowed build or test command inside it |
| `scripts/wt-commit.sh T012 "message"` | Commit its changes to its branch |
| `scripts/wt-merge.sh T012` | Merge it into main and remove it. A conflict undoes itself |
| `scripts/wt-remove.sh T012` | Throw it away without merging |

Each working copy builds its own Rust `target/` folder, which can reach several
gigabytes. Merging or removing a working copy frees it.

## 4. Commit rules

- Every commit carries your Git name and email. A commit with any other author is refused.
- Co-author lines and "generated with" lines are removed from every message.
- Anything under `private/`, and any file that looks like the licensed dictionary, is refused.
- Only Maya can commit. The other agents cannot run `git commit` or change Git settings.
- GitHub repeats the co-author check on every pull request.

To see for yourself: `git log --format='%h %an <%ae> %s'`.

## 5. RustRover (optional)

RustRover's built-in MCP server lets an agent use the IDE's inspections and
find-usages. It helps Arjun and Daniel on a large Rust codebase.

1. In RustRover: Settings, Tools, MCP Server, and enable it.
2. Copy the connection settings RustRover shows, and add them under an `mcp` entry in
   `opencode.json`, following OpenCode's MCP documentation for your version.
3. Keep RustRover open, on the main `~/code/dmux` folder, while the agents work.

RustRover sees the main checkout only, not the task working copies. It is useful for
understanding existing code, not for reviewing a task's changes.

## 6. Costs

| Who | Model on OpenRouter | Used for |
|---|---|---|
| Maya | anthropic/claude-opus-5.5 | Planning and briefing |
| Arjun (core), Daniel, Zara | anthropic/claude-sonnet-5.5 | The comparison core and both reviews |
| Arjun | z-ai/glm-5.3 | The rest of the Rust code |
| Leo | moonshotai/kimi-k3 | The UI |
| Priya | deepseek/deepseek-v4-pro | SQL review |
| Nina, Omar, Kenji, Ella | deepseek/deepseek-v4-flash | Test data, tests, CI, documents |
| Grace | google/gemini-3.7-flash | Screenshot review |
| Sam | z-ai/glm-5.3-flash | Code search |

Model names and prices change. Check them with `/models` and on openrouter.ai before
you start. To change one, edit the `model:` line at the top of that agent's file.

## 7. Before the repo goes public

- [ ] `scripts/check-public-clean.sh` passes with the private patterns configured
      (local `git config dmux.privateNamePattern`, or the CI secret). No vendor name,
      product version or dictionary file name in any tracked file or pushed history.
- [ ] `gitleaks detect` is clean.
- [ ] Public files say "a core banking system". The licensed dictionary, its parsed
      file and anything generated from them stay in `private/` only (decision 11).
- [ ] A `LICENSE` file is in place (the plan suggests Apache-2.0).
- [ ] The app identifier and package names are distinct from the existing `dmux` project.
- [ ] Your bank's policy on open-source tools has been checked (PLAN.md section 12).

## 8. If something does not work

**OpenCode rejects an agent file, or ignores its permissions.** The agent files use the
format in OpenCode's documentation as of November 2026: a `permissions:` list of
`action`, `resource` and `effect`, where the last matching rule wins. OpenCode has
changed this format before. If your version complains, only the block between the two
`---` lines at the top of each file needs adjusting; the instructions below it stay the
same. Run `opencode --version` and compare with opencode.ai/docs/agents.

**"commit refused: run scripts/setup-git.sh".** Run it once from `~/code/dmux`.

**A hook does not run.** Run `bash scripts/setup-git.sh` again; it restores the
executable flags that zip files can drop.

**Podman cannot pull an image.** Old images are large and slow the first time. Check
`podman info` reports `rootless: true`, then retry.

**`prove.sh` fails a check.** That is a finding, not a setup problem. Keep the output
and give it to Maya: the method must be fixed before any product code is written.

## 9. What has and has not been tested

- **Tested:** the Git scripts and hooks, in a scratch repo with real commits. The
  fingerprint queries, with all 15 checks passing on Postgres 16 between a LATIN1 and a
  UTF-8 database. The SELECT-only login script, on Postgres 16.
- **Not tested yet:** the fingerprint on Postgres 9.4, 9.6, 12, 17 and 18 (tasks T001
  and T002 do this). `scripts/arch-install.sh` on a real Arch machine. The agent files
  inside a running OpenCode. The Podman compose file with real containers.
- **Cannot be done on the laptop:** Windows installers. GitHub's build servers make those.
