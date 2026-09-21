# Memory: the cases, and ours

How an agent remembers between sessions. Four designs in use, then the one
titi ships and why it differs.

## The cases

**Hermes — two capped files.** `MEMORY.md` (2,200 chars) and `USER.md` (1,375)
under the agent home, injected whole into the system prompt. A `memory` tool
adds, replaces and removes entries separated by `§`. Overflow is an error the
agent fixes in the same turn; nothing is truncated silently. Entries are
scanned for injection before they touch disk. Deterministic and readable. It
stops scaling the moment the cap is hit, and it cannot say which entry matters
for the file being edited.

**omp — a backend you switch.** `memory.backend` is `off`, `local`,
`hindsight` or `mnemopi`, and the default is off. `local` extracts facts per
session and consolidates them into `MEMORY.md`. The other two add `recall`,
`retain` and `reflect` over a remote store, with embeddings
(`bge-base-en-v1.5`, 768d) and reciprocal rank fusion across vector, graph,
fact and temporal indexes. Powerful, and it takes the agent offline the moment
the backend is.

**Empryo — an index per project.** `.empryo/memory.db`, schema version 6. A
row is `summary`, `details`, `topics`, a category (`pref`, `decision`,
`gotcha`, `context`), a content hash for dedup, `use_count`, `pinned`,
`hidden` and `superseded_by`. Two FTS5 indexes (unicode61 and trigram), a
`memory_files` table tying a row to the files it is about, a `memory_edges`
table for similarity, and an embedding blob per row. Recall ranks and injects
the top rows, never the whole store. `memory-config.json` chooses the write
scope. The embedding model is configured, not fixed.

**Vellum — eight kinds.** Episodic, semantic, procedural, emotional,
prospective, behavioral, narrative, shared, each with its own staleness
window, isolated per user and per channel. Hybrid dense and sparse retrieval.
Richer than a file, and eight physical stores for what is one column.

## Ours

One index, `~/.titi/agent/memory.db`, and no file in the prompt.

- A row per memory, deduplicated by a SHA-256 of its text. The same fact
  stored twice increments `use_count`.
- FTS5 for the words, a file link so a memory about `auth.ts` surfaces while
  `auth.ts` is being edited, and a vector for the paraphrase a word search
  misses.
- The vector comes from an embedder the config picks. `memory.embeddingModel`
  empty or `local` uses a hashed bag of character trigrams: 256 dimensions,
  no model, no network, the same text always mapping to the same vector. Any
  other value names an OpenAI-compatible embeddings model, resolved through
  the provider registry. Each row records which embedder produced it, so a
  switch of model never compares vectors from two spaces.
- Recall ranks by cosine, boosts a row whose file the turn has touched, and
  injects the top five. The markdown stores are not injected at all.

## Where it is better

| | Hermes | omp | Empryo | titi |
|---|---|---|---|---|
| Works with no network | yes | only `local` | no, once embeddings are on | yes, by default |
| Finds the memory for the file being edited | no | yes | yes | yes |
| Grows past a few kilobytes | no | yes | yes | yes |
| Survives a model change | n/a | no | no | yes, rows are tagged |
| Readable without a tool | yes | partly | no | no |

The last row is the cost. A file you can open and edit by hand is gone. The
`memory` tool is the only writer, and that is deliberate: a hand-edited file
cannot be deduplicated, ranked or tied to a file.

## What is not built yet

The configured model is read and stored. The call that fetches its vector is
not wired, so a named model currently still embeds locally and records the
name it was asked for. Wiring it is one function: POST `/embeddings` through
the provider registry and pass the returned vector in. The trigram embedder
stays the fallback for when that request fails.
