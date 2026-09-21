# Storage

## What Empryo does

Empryo keeps sessions as JSONL under its home directory and a single
`memory.db` for persistent memory. It does not open a database per session.
The transcript is a file; the database is the thing you query across files.

## What titi does

The same split, already:

| Thing | Where | Why |
|---|---|---|
| Transcript | `sessions/<id>.jsonl` | Append-only. A crash loses at most one line. A session copies with one file. |
| Search | `state.db` (SQLite, WAL, FTS5) | One index over every session. Rebuilt from the JSONL, so it is disposable. |
| Keys | `auth.db` | Separate from the transcript on purpose. |
| Memory | `memories/MEMORY.md`, `USER.md` | Bounded, scanned for injection, injected verbatim into the prompt. |

`state.db` holds `sessions`, `entries` and `entries_fts`. It is a derived
index: delete it and the next open rebuilds it from the JSONL.

## Why not Turso

Turso (libsql) is SQLite plus replication. Nothing here replicates. The index
is local, disposable and rebuilt from files, so a network-capable database
adds a dependency and a failure mode without removing one. `rusqlite` with
`bundled` already gives FTS5 and WAL with no system library.

Revisit only if a session must be readable from a second machine while it is
being written. Until then the files are the replication: copy the directory.

## Why not a database per session

A per-session database would hold the transcript, its FTS and its memories in
one file. Three costs:

- Search across sessions becomes a loop over files instead of one FTS query.
- Resume-latest and the session list need a catalog anyway, so the single
  `state.db` comes back.
- SQLite plus WAL is three files per session. A thousand sessions is
  thousands of files the process opens one at a time.

The JSONL is already the per-session file, and it is the one format a person
can read and a crash cannot corrupt.

## What the `.empryo` directory actually is

Read from a live repo (`.empryo/`, schema version 6) and from
`src/core/memory/db.ts`. The `-shm` and `-wal` files are SQLite's own: WAL
mode keeps the write-ahead log beside the database, and `-shm` is its shared
memory. They are not separate databases.

| File | Scope | What it holds |
|---|---|---|
| `memory.db` | one per project, one global at `~/.empryo` | The memory index. `memory-config.json` says which is written (`writeScope: project`) and which are read (`readScope: all`). |
| `genome.db` | per project | The code graph: `files` (289 here, with pagerank and churn), `symbols` (2499), `edges` (672), `refs`, `cochanges`, plus FTS over symbols. This is the Soul Map. |
| `sessions/<id>/session.jsonl` | per session | The transcript, still a file, with `meta.json` beside it. |
| `history.db` | global | Command and prompt history. |

`memory.db` is not one row per fact in a file. A memory is a row in
`memories`: `summary`, `details`, `topics`, a `category` (`pref`, `decision`,
`gotcha`, `context`), a `source` (`user` or `agent`), the `session_id` it came
from, `use_count`, `last_used_at`, `pinned`, `hidden`, and `superseded_by`.
`content_hash` is unique, so writing the same fact twice increments
`use_count` instead of inserting. Two FTS5 indexes sit on top, `memories_fts`
(unicode61, for words) and `memories_fts_tri` (trigram, for CJK and short
tokens), maintained by triggers.

Three things make it more than a file:

- `memory_files` ties a memory to the files it is about, so a memory about
  `auth.ts` surfaces when that file is edited.
- `memory_edges` links memories as `similar` (by embedding cosine) or
  `supersedes`.
- `embedding` is a blob per row. Recall ranks by FTS score, file overlap,
  recency and `use_count`, and only the top few are injected. The whole store
  is never put in the prompt.

This repo holds 6 memories and 7 file links. The gotchas are goal-loop
failures the agent wrote itself.

## What titi should take from it

The markdown stores stay for what they are good at: a bounded, human-readable
snapshot that is injected whole. They cannot do the rest. A memory that grows
past 2,200 characters has to be deleted, and there is no way to find the one
memory that matters for the file being edited.

The piece worth porting is the index, not the embeddings. A `memories` table
beside `state.db` with the category, the content hash, the file link and FTS
gives dedup, search and file affinity, and it uses the `rusqlite` already in
the tree. Embeddings and the similarity graph wait until the index has more
rows than a prompt can hold.

## Memory stays global

`MEMORY.md` and `USER.md` are injected into every turn, so they are the
agent's memory, not a session's. A session-scoped note belongs in the
transcript. Linking the two would make a resumed session depend on a database
row that the JSONL does not contain, and the JSONL is the source of truth.
