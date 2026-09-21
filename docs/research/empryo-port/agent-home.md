# Agent home

Everything that belongs to the agent lives in one directory, outside every
repository. OMP keeps its state in `~/.omp/agent`, pi in `~/.pi`; titi keeps
its own in `~/.titi/agent`. A repository holds the project. It never holds the
agent's sessions, memory or secrets.

## Resolution

`titi_config::agent_dir()`:

1. `$TITI_AGENT_DIR`, when set.
2. `~/.titi/profiles/<name>/agent`, when `$TITI_PROFILE` names a profile other
   than `default`.
3. `~/.titi/agent`.

## Layout

```
~/.titi/agent/
  config.yml        settings (global layer)
  SOUL.md           identity, seeded once, never overwritten
  PERSONALITY.md    personality override, optional
  .env              secrets fallback, below the project .env
  auth.db           stored provider keys
  state.db          session index (SQLite, WAL)
  sessions/         one JSONL file per session
  trajectories/     one JSONL file per session
  memories/
    MEMORY.md       what the agent remembers about the work
    USER.md         what it remembers about the user
  themes/           custom themes
```

A named profile is the same tree under `~/.titi/profiles/<name>/agent/`, so
two profiles never see each other's sessions or keys.

## What a repository holds

`<project>/.titi/config.yml` is the one project file, and it is settings only:
it overrides the global layer for that repository and is read-only through the
settings API. There is no `<project>/.titi/.env` and no session store in the
repository. Secrets resolve as process env, then `<project>/.env`, then
`<agent_dir>/.env`.
