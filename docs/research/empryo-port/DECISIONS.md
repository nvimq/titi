# Decisions

Newest first. STATE.md lists the same work; this file is the one to read
when the two disagree, because several STATE entries describe an earlier
shape that was later replaced.

## 2026-09-22

- **Memory is the index, not a file.** `MEMORY.md` and `USER.md` are no longer
  injected. `identity_prompt` sends SOUL and personality only; recall injects
  the top five rows from `memory.db`. A file pasted whole shows every fact on
  every turn and cannot say which one matters.
- **The embedder is a setting.** `memory.embeddingModel` empty or `local`
  uses a 256-d hashed trigram vector, offline. Any other value names an
  OpenAI-compatible embeddings model. Each row records which embedder
  produced it, so a model change never compares two vector spaces.
- **Suggestions follow connected providers.** `memory models` offers `local`
  always and a hosted model only when its provider is in the registry.
  Suggesting Voyage to a setup with no Voyage key is a dead end.
- **Secrets are masked before the write.** `sk-`, `ghp_`, `AKIA`, PEM blocks,
  JWTs and `token=` assignments become `[redacted]`. A commit hash and prose
  about tokens pass. A memory is shown on every later turn, so a stored key
  is a leaked key.
- **`memory` browses.** `list` shows everything, `search` ranks, `remember`
  writes. Read-tier: remembering is the point of the tool.

## 2026-09-21

- **One agent home.** Everything lives in `~/.titi/agent`
  (`$TITI_PROFILE` → `~/.titi/profiles/<name>/agent`, `$TITI_AGENT_DIR`
  overrides). The repository holds `<project>/.titi/config.yml` and nothing
  else. `<project>/.titi/.env` was removed.
- **SQLite stays.** Sessions are JSONL, `state.db` is a derived FTS index.
  Turso adds replication nothing uses. A database per session turns
  cross-session search into a loop and still needs the shared catalog.
- **A checkpoint commits only what is staged.** `git add -A` was removed
  after a test run committed unrelated dirty work into this repo. `/rewind`
  is `git reset --hard` and refuses a dirty tree.
- **Herdr is told, not guessed.** `pane.report_agent` on `HERDR_SOCKET_PATH`,
  source `herdr:titi`: a running turn is `working`, an approval or an armed
  exit is `blocked`, otherwise `idle`. Outside a pane nothing is sent.
- **Ctrl+C asks twice.** The first press arms the exit for two seconds; any
  other key disarms it. A running turn is cancelled in one press.
- **Compaction is wired.** `StructuredSummarizer` is model-free;
  `SnapCompact`/`Handoff` digest the folded prefix, `Remote`/`Soft` fail so
  the chain falls through. The kept tail never starts with a tool result.

## 2026-09-20

- **The engine is the protocol.** `EngineCommand` in, `EngineEvent` out. TUI,
  headless and a future GPUI desktop are surfaces over the same channel.
- **Ctrl+C stops a turn.** Before this it only quit. `EngineCommand::Cancel`
  had no sender anywhere in the CLI.
- **A subagent is read-only unless told otherwise.** It has no approval
  surface, so a write tool would wait forever.
