# Сессии и персистентность

Исследование: модель сессии, дерево/форк/resume, хранение (SQLite+FTS5 у Hermes vs JSONL у omp), titles.

## omp

omp — source of truth по сессиям; файловый append-only формат + древовидная модель поверх.

1. **Формат и layout.** Сессия — JSONL-файл `~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<sessionId>.jsonl`. Файл начинается с фиксированного 256-байтного `type: "title"` слота (обновляется без перезаписи тела), затем header (`type: "session"`: `version` (текущая — 3), `id`, `cwd`, `title`, `titleSource` (`auto`|`user`), `parentSession`, `previousSessionFiles`, `providerPromptCacheKey`) и entries. Source: `omp://session.md` (разделы «On-Disk Layout», «File Format»).
2. **Entry-таксономия.** `SessionEntry` — union из 15 типов (`message`, `compaction`, `branch_summary`, `reset_boundary`, `custom`, `custom_message`, `label`, `title_change`, `model_change`, `session_init`, `mode_change` и др.); у каждого entry — 8-символьный `id`, `parentId`, `timestamp`. `custom`-записи с зарезервированными `customType` (`tool_execution_start`, `session_exit`, `user_todo_edit`, …) используются для восстановления состояния после крэша. Source: `omp://session.md` («Entry Taxonomy»).
3. **Дерево + leaf-pointer.** Модель runtime — append-only дерево: каждый append создаёт ребёнка текущего `leafId`; `branch(entryId)` двигает только указатель, история не переписывается; `resetLeaf()` обнуляет `leafId` (следующий append — новый root); `branchWithSummary()` двигает leaf и дописывает `branch_summary` (при ветвлении от корня — `fromId: "root"`). Runtime-индекс: `#entriesById`, `#children` (parent→children adjacency), `#labels`, `#leaf`. Source: `omp://session-tree-plan.md` («Tree data model in SessionManager», «Leaf movement semantics»).
4. **Контекстная реконструкция.** `buildSessionContext(entries, leafId, byId)` идёт от leaf к root по `parentId` (с защитой от циклов), восстанавливает state (`thinking_level_change`, `model_change`, `mode_change`, `service_tier_change`), применяет последнюю compaction (summary + kept-сообщения от `firstKeptEntryId`) или `reset_boundary`, отрезает dangling tool calls. Source: `omp://session.md` («Context Reconstruction»).
5. **Навигация vs форк.** `/tree` — навигация внутри файла (leaf-move; выбор user-сообщения возвращает draft в редактор, leaf → его `parentId`); `/branch` — создание нового session-файла (`createBranchedSession(leafId)`: копия root→leaf пути без `label`-entries, пересборка лейблов); `/fork` — полный дубликат в новый файл с `parentSession` и наследованием `providerPromptCacheKey`; `/resume`/`--continue` — переключение файлов через `switchSession` с rollback-снапшотом и событиями `session_before_switch`/`session_switch`. Source: `omp://session-operations-export-share-fork-resume.md` (операционная матрица), `omp://tree.md` (таблица «/tree vs adjacent operations»).
6. **Titles и listing.** `title_change` — append-only аудит переименования; текущий title живёт в 256-байтном слоте, чтобы listing не читал тело. Discovery читает только 4 KiB префикс файла (`getRecentSessions`, дефолтный лимит 4), полный список — префикс + bounded 32 KiB хвост для lifecycle-статуса (`complete|interrupted|aborted|error|pending`). Resume-матчинг — case-insensitive префиксы id/имени файла. Source: `omp://session-switching-and-recent-listing.md` («Two listing paths»), `omp://session.md` («Session Discovery Utilities»).
7. **Гарантии записи.** Запись — синхронный append без `fsync` (защита от софтверных крэшей, не от потери питания); атомарные полные переписывания через stage+rename с EPERM-fallback; новая сессия — memory-only до первого assistant-сообщения или `ensureOnDisk()`. Отдельный подсистем — `HistoryStorage` (`~/.omp/agent/history.db`, SQLite, таблица `history` + FTS5 `history_fts` с триггерной синхронизацией, батчинг ~100 ms) — только для prompt-recall, не для replay. Source: `omp://session.md` («Persistence Guarantees», «Related but Distinct: Prompt History Storage»).

## Hermes

Hermes — сессия как строка в единой SQLite-БД, вся история и поиск централизованы.

1. **Единая БД.** Все сессии (CLI, TUI, 22 source-платформы: `cli`, `telegram`, `discord`, `cron`, `batch`, …) хранятся в `~/.hermes/state.db` в WAL-режиме. Таблицы: `sessions` (id, source, user_id, model, title, token counts, `started_at`/`ended_at`, `parent_session_id` для compression-lineage), `messages` (role, content, tool_calls, tool_name, token_count), `messages_fts` — FTS5 virtual table. Идентификатор — `YYYYMMDD_HHMMSS_<hex>` (6 hex у CLI, 8 у gateway). Source: https://hermes-agent.nousresearch.com/docs/user-guide/sessions (разделы «How Sessions Work», «Storage Locations», «Database Schema»).
2. **Resume и lineage.** `hermes -c` — terminal-aware continue: breadcrumb-файл в `~/.hermes/terminal-sessions/` (tty device, tmux pane, kitty window, …), при отсутствии/устаревании (>30 дней) — fallback на глобально последний; отключается `session.terminal_continue: false`. `--resume <id|title|latest>` — по id (полный или уникальный префикс) или title. Компрессия (`/compress`) создаёт continuation-сессию с тайтлом `"my project #2" → "#3"`; resume по имени автоматически берёт последнего в lineage. Source: тот же URL («CLI Session Resume», «Session Naming»).
3. **Titles.** Авто-тайтл (3–7 слов) генерируется фоновым потоком быстрой моделью после первого обмена, один раз, пропускается при ручном `/title`. Правила: уникальность (unique index в `sessions`, NULL разрешены), ≤100 символов, санитизация control/zero-width/RTL-символов. CLI-переименование: `hermes sessions rename <id> <title>`. Source: тот же URL («Session Naming», «Title Rules»).
4. **FTS5-поиск как инструмент агента.** Встроенный tool `session_search` — 4 формы вызова (discovery по `query` → FTS5 + dedupe по lineage; scroll по `session_id`+`around_message_id` с окном ±N; read всей сессии; browse без аргументов). Поддержка синтаксиса FTS5: фразы `"..."`, `OR`/`NOT`, префикс `deploy*`; параметры `sort`, `role_filter`, `detail=adaptive` (полная гидратация только top-результата). Типичное время: десятки ms на discovery, 1–2 ms на scroll. Source: тот же URL («Session Search Tool»).
5. **Жизненный цикл.** `hermes sessions prune/archive/pin/export/delete/rename/stats`; авто-prune выключен по умолчанию (`sessions.auto_prune`, `retention_days: 90`, VACUUM не чаще `min_vacuum_interval_days: 30`); pinned-сессии не подчищаются. Экспорт: jsonl/md/qmd/html/trace (Claude Code JSONL для HF Agent Trace Viewer), `--redact` для секретов. Continuity после крэша: routing-identity пишется атомарно при создании строки, recovery уважает `/new`-границы. Source: тот же URL («Session Expiry and Cleanup», «Continuity After Crashes and Restarts», «Export Sessions»).

## Vellum

Vellum-материал (`local://vellum-summary.md`) тему файлового формата сессий, resume и полнотекстового поиска **не покрывает** — сессии как отдельная сущность не описаны. Ближайшие аналоги, влияющие на персистентность:

1. **Персистентная память вместо журнала сессий.** Восемь типов памяти (episodic, semantic, procedural, emotional, prospective, behavioral, narrative, shared), каждый со своим staleness window; структурные элементы (identity, preferences, projects, events) извлекаются из диалогов с указанием источника и дедупликацией. Т.е. долговременное состояние живёт в структурированной памяти, а не в воспроизводимом транскрипте. Source: `local://vellum-summary.md`, раздел «Память».
2. **Скретчпад и журналы как первый класс.** `NOW.md` — скретчпад текущего фокуса и активных нитей; per-user журнал размышлений (reflections); ежечасный пересмотр заметок на предмет незавершённого. Source: `local://vellum-summary.md`, разделы «Идентичность (SOUL)» и «Проактивность».
3. **Изоляция по каналу/боту.** Один ассистент, одна память, каждый канал; пер-бот изоляция SOUL+memory+skills — ключи изоляции хранилища (per-user/per-channel) прямо применимы к схеме БД сессий в titi. Source: `local://vellum-summary.md`, разделы «Каналы» и «Приоритеты пользователя для titi».

## Решение (одно/комбо)

Комбо: **omp-модель как канонический формат + Hermes-слой как производный индекс**. Каноническое хранилище — append-only JSONL с `id`/`parentId` и leaf-указателем (это даёт бесплатно дерево, форк без копирования истории, дешёвый append и потоковую загрузку), а поверх — единая SQLite-БД-индекс (`rusqlite`, WAL) с FTS5 по сообщениям, titles, source/cwd и lifecycle-статусом, который строится инкрементально при append и служит для listing/resume/`session_search` (омповый подход «читать 4 KiB префикс каждого файла» не масштабируется на сотни сессий и не даёт полнотекстового поиска по истории). Titles — гибрид: omp-слот не нужен (индекс и так мгновенный), берём правила Hermes — авто-тайтл фоновой моделью, уникальность, `#N`-lineage при компакции, `titleSource: auto|user` с запретом перезаписи пользовательского. Для мультиботности Vellum в схему индекса добавляются колонки `bot_id`/`channel`, обеспечивая пер-бот изоляцию поиска без размножения файлов. Это оптимально по совокупности: JSONL — надёжный, потокочитаемый, git-friendly формат replay; SQLite+FTS5 — O(log n) выборки и мгновенный recall; оба слоя decoupled — индекс всегда можно перестроить из JSONL.

## Rust-маппинг

**Крейты workspace:** `titi-core` (домен сессии), `titi-tui` (tree/picker UI), `titi-cli` (resume/continue); предлагаемые новые: `titi-sessions` (формат, дерево, миграции), `titi-sessions-index` (SQLite+FTS5 индекс). `titi-providers`/`titi-tools` только потребляют контекст.

**Ключевые типы (эскизы):**

```rust
// titi-sessions/src/entry.rs
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message { message: AgentMessage },
    Compaction { summary: String, first_kept_entry_id: EntryId, .. },
    BranchSummary { from_id: Option<EntryId>, summary: String },
    ResetBoundary,
    TitleChange { title: String, source: TitleSource },
    Custom { custom_type: String, data: serde_json::Value },
    // model_change, mode_change, label, session_init, ...
}

#[derive(Serialize, Deserialize)]
pub struct SessionEntryBase { pub id: EntryId, pub parent_id: Option<EntryId>, pub timestamp: DateTime<Utc> }

pub struct SessionHeader { pub id: SessionId, pub version: u32, pub cwd: PathBuf,
    pub title: Option<String>, pub title_source: TitleSource, pub parent_session: Option<String> }

// titi-sessions/src/tree.rs — runtime-индекс omp-стиля
pub struct EntryIndex {
    entries_by_id: HashMap<EntryId, SessionEntry>,
    children: HashMap<Option<EntryId>, Vec<EntryId>>,
    labels: HashMap<EntryId, String>,
    leaf: Option<EntryId>,
}
impl EntryIndex {
    pub fn append(&mut self, entry: SessionEntry) -> EntryId;        // ребёнок текущего leaf
    pub fn branch(&mut self, id: EntryId) -> Result<()>;             // move leaf, no write
    pub fn reset_leaf(&mut self);                                     // leaf = None
    pub fn path_to_root(&self, from: EntryId) -> Vec<EntryId>;       // с защитой от циклов
    pub fn build_context(&self, leaf: Option<EntryId>) -> SessionContext;
}

// titi-sessions/src/storage.rs
pub trait SessionStorage {
    fn append(&self, path: &Path, line: &str) -> io::Result<()>;
    fn write_atomic(&self, path: &Path, body: &str) -> io::Result<()>; // stage+rename
    fn read_prefix(&self, path: &Path, n: usize) -> io::Result<String>;
    fn load(&self, path: &Path) -> Result<Vec<SessionEntry>, LoadError>; // lenient JSONL, миграции v1→v3
}

// titi-sessions-index/src/lib.rs — Hermes-слой
pub struct SessionIndex { db: rusqlite::Connection } // WAL, путь ~/.titi/index.db
impl SessionIndex {
    pub fn upsert_session(&self, meta: &SessionMeta) -> Result<()>;   // ON CONFLICT(id)
    pub fn index_message(&self, sid: &SessionId, msg: &IndexedMessage) -> Result<()>;
    pub fn search(&self, q: &FtsQuery) -> Result<Vec<SearchHit>>;      // FTS5 MATCH, snippet()
    pub fn recent(&self, scope: &ScopeFilter, limit: usize) -> Result<Vec<SessionMeta>>;
    pub fn resolve_resume(&self, key: &str) -> Result<Option<SessionId>>; // id/title/префикс
    pub fn set_title(&self, sid: &SessionId, title: &str, source: TitleSource) -> Result<()>;
}
pub struct FtsQuery { pub text: String, pub bot_id: Option<BotId>, pub source: Option<Source>, pub sort: Sort }
```

**Схема БД:** `sessions(id PK, bot_id, source, cwd, title UNIQUE NULL, title_source, model, parent_session_id, started_at, ended_at, status, tokens_in, tokens_out)`, `messages(session_id, idx, role, content, tool_calls_json, ts)` + `messages_fts` (FTS5, content-таблица на `messages`, триггеры sync, как `history_fts` у omp). Внешние крейты: `rusqlite` (+ bundled feature для FTS5), `serde`/`serde_json` для JSONL, `tokio` (фоновая генерация тайтлов, батч-очередь индексации ~100 ms по образцу omp HistoryStorage), `chrono`/`time`, `sha2` (content-addressed blobs, если решим взять externalization картинок omp), `uuid` или `rand` для 8-символьных id.

## Definition of Done

- [ ] `titi-sessions` пишет JSONL-файл с header `version: 3` и entries `id`/`parentId`/`timestamp`; формат зафиксирован тестом на golden-файл (roundtrip: load → append → load сохраняет дерево).
- [ ] `EntryIndex.append/branch/reset_leaf` покрыты тестами: после `branch(id)` история не переписана, новый append стал ребёнком `id`; `reset_leaf` даёт новый root с `parent_id: null`; цикл в `parentId` не вызывает бесконечный обход (`path_to_root` терминируется).
- [ ] `build_context` корректно применяет последнюю `compaction` (summary + kept-сообщения от `first_kept_entry_id`) и `reset_boundary` (всё до границы скрыто из контекста, но читается в transcript-режиме) — по юнит-тестам на обоих случаях.
- [ ] SQLite-индекс (rusqlite, WAL, FTS5) создаётся миграцией; `search` находит по фразе/prefix/`OR` и возвращает snippet; перестройка индекса из чистых JSONL-файлов даёт идентичный результат (свойство: индекс — производная структура) — property/сравнительный тест.
- [ ] Resume: `resolve_resume` находит по полному id, уникальному префиксу и title (последний в lineage `#N`); неоднозначный префикс возвращает ошибку, а не первый попавшийся (улучшение против omp-каверн «first match wins»).
- [ ] Titles: авто-тайтл фоновым вызовом модели после первого обмена (tokio task), один раз; ручной title с `title_source: user` не перезаписывается авто; уникальность enforced на уровне БД (тест на UNIQUE violation → ошибка, не panic).
- [ ] Мультиботность: колонки `bot_id`/`source` в индексе; `search`/`recent` с фильтром по `bot_id` не возвращают чужие сессии — тест на два бота в одной БД.
- [ ] Крэш-восстановление: kill -9 между append-ами не портит JSONL (lenient-парсер пропускает обрезанную последнюю строку), индекс перестраивается; тест через симуляцию обрезанного файла.

## Deep-dive

План подсистем-доков (писать при детализации):

- `docs/research/sessions-persistence/format-jsonl.md` — физический формат файла, миграции v1→v3, lenient-парсинг, blob-externalization (omp `session-persistence.ts`).
- `docs/research/sessions-persistence/tree-and-branching.md` — семантика leaf, `/tree` vs `/branch` vs `/fork`, `branch_summary`, labels, события `session_before_tree`/`session_tree`.
- `docs/research/sessions-persistence/sqlite-index.md` — схема state.db Hermes и history.db omp, FTS5-синтаксис, адаптивная гидратация результатов `session_search`, WAL и concurrency.
- `docs/research/sessions-persistence/titles-and-naming.md` — title-слот omp, auto-тайтл Hermes, уникальность, `#N`-lineage, план-одобрение (humanizePlanTitle).
- `docs/research/sessions-persistence/resume-switching.md` — `--continue`/breadcrumb, `--resume <id>`, `switchSession` c rollback, cross-project re-rooting, lifecycle-статусы.
