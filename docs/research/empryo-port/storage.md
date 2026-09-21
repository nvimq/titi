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

## Memory stays global

`MEMORY.md` and `USER.md` are injected into every turn, so they are the
agent's memory, not a session's. A session-scoped note belongs in the
transcript. Linking the two would make a resumed session depend on a database
row that the JSONL does not contain, and the JSONL is the source of truth.
