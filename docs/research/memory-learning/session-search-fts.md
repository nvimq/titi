# Поиск по сессиям (FTS5)

Полный поиск по всем прошлым сессиям без LLM: SQLite-БД с FTS5-индексом, мгновенные ответы, scroll/read/browse по найденным сессиям. Комплементарно bounded-стору ([stores.md](./stores.md)): память — «критичные факты всегда в контексте», session search — «обсуждали ли мы X на прошлой неделе?».

## omp

omp не имеет прямого аналога инструмента «поиск по всем сессиям для агента» — тема покрыта частично, ближе всего инфраструктура сессий и консолидация:

1. Сессии персистятся в session-файлы; SQLite в omp используется под служебные очереди памяти, а не под поиск: `packages/coding-agent/src/memories/storage.ts` — «SQLite-backed job queue and thread registry» для пайплайна консолидации (источник: omp://memory.md). Поиск по содержимому прошлых сессий для модели не экспонируется.
2. Косвенный механизм — memory pipeline: Phase 1 читает каждую изменённую сессию (`threadScanLimit=300` последних, `maxRolloutAgeDays=30`, `minRolloutIdleHours=12`), извлекает durable-сигнал моделью (это LLM-процесс, ~конкурентность 8), и только выжимки попадают в `MEMORY.md`/`memory_summary.md` (omp://memory.md). Т.е. доступ к «старым разговорам» идёт через LLM-суммаризацию с эвристическими фильтрами, а не через мгновенный полнотекстовый запрос — ближайший аналог, но с противоположной экономикой (LLM-токены против ~20ms SQL).
3. Внешний бэкенд Hindsight добавляет recall по сохранённым retains, но это ретривал по памяти, записанной ретеншном (каждые 3 хода, `full-session` режим), а не по сырому логу всех сессий; recall-запрос идёт на HTTP-сервер (omp://tools/retain.md).

## Hermes

Эталонная реализация — инструмент `session_search`:

1. Все сессии (CLI и все мессенджер-платформы — 20+ источников) пишутся в SQLite `~/.hermes/state.db`: session id, source platform, user id, уникальный человекочитаемый title, модель и конфиг, system prompt snapshot, полное сообщение-история (role, content, tool calls, tool results), token counts, timestamps, `parent_session_id` для сжатий. FTS5 full-text search индексирует этот стор; `hermes sessions optimize` мержит FTS5-сегменты и VACUUM-ит БД, не трогая данные (источник: https://hermes-agent.nousresearch.com/docs/user-guide/sessions).
2. `session_search` делает 0 LLM-вызовов и возвращает view реальных сообщений из БД, «no LLM summarization, no truncation». Четыре формы вызова без параметра `mode` — выводится из аргументов (источник: https://hermes-agent.nousresearch.com/docs/user-guide/sessions):
   - Discovery: `session_search(query, limit=3)` → FTS5, дедуп по session lineage, top-N сессий; adaptive detail: топ-1 — full (полное контекстное окно + bookends), остальные compact; `detail="full"` гидратирует всё.
   - Scroll: `session_search(session_id, around_message_id, window=10)` → ±window сообщений вокруг якоря, 1–2ms; forward = передать `messages[-1].id`, backward = `messages[0].id`; граничное сообщение встречается в обоих окнах как ориентир.
   - Read: `session_search(session_id)` → вся сессия или bounded head/tail для больших; также резолвит `@session:<profile>/<id>`.
   - Browse: без аргументов → листинг (см. доку).
3. Поля результата discovery: `session_id`, `title`, `when`, `source`, `snippet` (FTS5-highlighted excerpt), `detail`, `bookend_start`/`bookend_end` (первые/последние 3 user+assistant сообщения у full), `messages` (±5 вокруг матча у full, только anchor у compact), `match_message_id`, `messages_before`/`messages_after`. Типичная задержка discovery — «tens of milliseconds» на реальной БД; scroll — 1–2ms (там же).
4. Экономика против памяти (сравнительная таблица доки): память ~1,300 токенов на каждый промпт; session search — бесплатно до запроса, ~20ms FTS5, ~1ms scroll, ёмкость неограничена. Заголовки сессий задаются при `/new <name>` и ищутся потом через `/resume <name>`; `hermes sessions list --workspace <needle>` фильтрует по workspace-ключу (git root или cwd) (источники: https://hermes-agent.nousresearch.com/docs/user-guide/sessions, https://hermes-agent.nousresearch.com/docs/user-guide/features/memory).

## Vellum

Vellum не покрывает полнотекстовый поиск по сессиям в собранном материале (local://vellum-summary.md): её ретривал — гибридный dense+sparse по памяти с per-user/per-channel изоляцией, а не по логу сессий. Ближайшие аналоги:

1. Sparse-ветка гибридного retrieval концептуально совпадает с FTS5 (лексический матчинг против dense-векторов); у Vellum она работает поверх памяти с изоляцией per-user/per-channel — в titi это отображается на SQL-фильтры `WHERE user_id = ? AND channel = ?` поверх FTS5-запроса.
2. Дедупликация извлечённых структурных элементов с указанием источника (identity, preferences, projects, events) — аналог lineages: при поиске сессий надо дедуплицировать hits по линии сжатых сессий (`parent_session_id`), чтобы матч не вернулся пятью «разными» сессиями; у Hermes это «dedupes hits by session lineage» (источник: https://hermes-agent.nousresearch.com/docs/user-guide/sessions).
3. Изоляция unknown-акторов (нельзя читать память) распространяется и на поиск: результаты `session_search` фильтруются по actor identity до отдачи (local://vellum-summary.md).

## Решение (одно/комбо)

Одно: схема Hermes `session_search` на FTS5, локализованная в наш session-стор. Одна SQLite-БД: таблицы `sessions` (id, title, source, cwd, tokens, timestamps, parent_id) и `messages` (role, content, tool-сводки) + одна FTS5-виртуальная таблица `messages_fts` (внешний контент на `messages`, чтобы не дублировать текст). Четыре формы вызова без `mode` — это дёшево в реализации (один дизпатчер по `Option`-полям) и дружелюбно модели. Adaptive detail (top-1 full, остальные compact) экономит токены ответа и обычно закрывает вопрос первым результатом — берём как есть. Дедуп по lineage через транзитивный `parent_session_id`. Эмбеддинги здесь НЕ добавляем: гибридный dense-поиск уже живёт в фактовом сторе ([stores.md](./stores.md)), дублировать его для сырых логов избыточно; FTS5 с prefix/NEAR-запросами покрывает «did we discuss X» при нулевой стоимости инференса. Rust-маппинг не тянет новых крейтов — тот же `rusqlite` (bundled с FTS5) и `tokio`.

## Rust-маппинг

Крейты: `titi-memory` (или `titi-search` при разделении) — слой поиска; `titi-tools` — регистрация `session_search`; `titi-cli` — `titi sessions list|search|optimize`; `titi-tui` — просмотр найденного с переходом по сообщениям.

```rust
// titi-memory::session
pub struct SessionRow { pub id: SessionId, pub title: String, pub source: Source, pub parent: Option<SessionId> }
pub struct MessageRow { pub id: i64, pub session: SessionId, pub role: Role, pub text: String }

pub enum SessionSearch<'a> {                       // 4 формы — выводятся из заполненности
    Discover { query: &'a str, limit: usize, detail: Detail },   // Detail::Adaptive|Full|Compact
    Scroll { session: SessionId, around: i64, window: u32 },
    Read { session: SessionId, bound: HeadTail },
    Browse,
}
pub struct Hit {
    pub session: SessionId, pub title: String, pub when: DateTime<Utc>, pub source: Source,
    pub snippet: String,                            // FTS5 snippet()/highlight()
    pub detail: Detail,
    pub bookends: (Vec<MessageRow>, Vec<MessageRow>),
    pub messages: Vec<MessageRow>,                  // ±5 вокруг матча или anchor
    pub match_message_id: i64,
}

pub struct SessionIndex { db: rusqlite::Connection }
impl SessionIndex {
    pub fn search(&self, q: &SessionSearch) -> Result<SearchOutcome, Error>;
    fn dedup_lineage(&self, hits: Vec<Hit>) -> Vec<Hit>;           // parent_session_id транзитивно
    fn optimize(&self) -> Result<(), Error>;                       // fts5 merge + VACUUM
}
```

Внешние крейты: `rusqlite` (bundled: FTS5, `snippet()`/`highlight()`, external-content таблицы), `tokio` (асинхронная обёртка через `spawn_blocking`), `serde` (результаты для schema тулов). Локализация FTS: опциональный tokenizer-плагин не нужен на старте — default unicode61 достаточно для подстрочного «обсуждали ли X».

## Definition of Done

- [ ] Schema `sessions`/`messages`/`messages_fts` (external content) создана миграцией; каждый записанный ход автоматически индексируется в FTS5 (тест: write → search находит).
- [ ] Discovery: `session_search(query, limit)` возвращает top-N с дедупом по lineage — тест: три сессии, связанные `parent_session_id`, с одинаковым матч-текстом дают один hit.
- [ ] Adaptive detail: топ-1 содержит `bookends` и ±5 сообщений вокруг матча; компактные — только anchor (тест на структуру результата).
- [ ] Scroll: `around_message_id` + `window` возвращает ±window; forward-скролл по `messages[-1].id` и backward по `messages[0].id` не теряют и не дублируют сообщения на границах (тест).
- [ ] Read: полная сессия; для сессии >N сообщений — bounded head/tail (тест на границу).
- [ ] Никаких LLM-вызовов в тракте поиска; p95 discovery на БД в 1000 сессий × 100 сообщений — десятки миллисекунд (бенчмарк-тест с критерием).
- [ ] CLI `titi sessions search <q>` и `titi sessions list` работают против той же БД (smoke).
- [ ] Фильтрация по actor/user: поиск с identity=unknown возвращает ошибку доступа, а не результаты (тест, привязка к Vellum-изоляции).

## Deep-dive

Дальнейшие подсистемные доки (2-й уровень; писать при захвате фазы реализации):

- [../session-search-fts/tokenizers.md] — сравнение FTS5 tokenizer'ов (unicode61 vs trigram) для русского/смешанного текста, prefix-запросы.
- [../session-search-fts/schema-migrations.md] — миграции при изменении схемы сообщений, пересборка FTS-индекса, `optimize`-расписание.

Кандидаты: интеграция с compact/resume (`parent_session_id` цепочки), экспорт `--json` для бэкапов.
