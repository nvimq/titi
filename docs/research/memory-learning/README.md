# Память и обучение

Обзор темы: как агент запоминает факты между сессиями, как ищет их в прошлом (включая по-настоящему старые разговоры) и как визуализирует/корректирует накопленный опыт. Три подсистемы:

- [stores.md](./stores.md) — bounded-хранилища фактов (MEMORY/USER-подобные сторы), инструмент `memory` (add/replace/remove, substring match, capacity errors), авто-сохранение фактов, типологизация памяти (8 типов Vellum), гибридный retrieval и локальные эмбеддинги.
- [session-search-fts.md](./session-search-fts.md) — полнотекстовый поиск по всем прошлым сессиям (FTS5 в SQLite), scroll/read/browse-формы, no-LLM-ответы.
- [journey-graph.md](./journey-graph.md) — граф обучения: таймлайн выученного (навыки + куски памяти), scrubber-реплеи, prune/edit узлов.

## omp

omp покрывает тему глубокo, но через бэкенд-модель, а не bounded-файлы:

1. Четыре режима памяти (`memory.backend`: `off`, `local`, `hindsight`, `mnemopi`), память выключена по умолчанию (источник: omp://memory.md). `local` — пайплайн из двух фаз (per-session extraction + consolidation), который пишет `MEMORY.md`, `memory_summary.md` и `skills/`; `learned.md` ведётся инструментом `learn`, newest-first, дедуп, secret-redaction, cap 100 записей, контент ≤2000 символов (omp://memory.md).
2. Инструменты `recall` / `retain` / `reflect` / `memory_edit` доступны только для бэкендов `hindsight` и `mnemopi`; `memory_edit` поддерживает `update|forget|invalidate` по id, fact-строки read-only (omp://tools/memory_edit.md, omp://tools/recall.md). `retain` в Mnemopi пишет synchronously в scoped SQLite-банк (importance 0.75, memoryType `fact`), в Hindsight — батч-очередь с flush при 16 элементах или debounce 5 c (omp://tools/retain.md).
3. Авто-сохранение: Mnemopi auto-retain каждые 4 пользовательских хода (`mnemopi.retainEveryNTurns=4`, memoryType `episode`), Hindsight — каждые 3 хода; recall первого хода (`autoRecall: true`) инжектится блоком `<memories>` (omp://mnemosyne-memory-backend.md). Полифонический recall (vector, graph, fact, temporal) с reciprocal rank fusion — опция `mnemopi.polyphonicRecall`; локальные эмбеддинги: `BAAI/bge-base-en-v1.5` (768d) или `intfloat/multilingual-e5-large` (1024d), ключ `mnemopi.noEmbeddings` форсирует FTS-only (omp://mnemosyne-memory-backend.md).

## Hermes

Hermes даёт ровно ту модель, которую просит тема — bounded curated memory:

1. Два bounded-стора: `MEMORY.md` (2,200 символов ≈ 800 токенов) и `USER.md` (1,375 символов ≈ 500 токенов) в `~/.hermes/memories/`, инжектятся в system prompt frozen-снапшотом на старте сессии (prefix-cache friendly); записи разделены `§`, хедер показывает заполнение `67% — 1,474/2,200 chars` (источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory).
2. Инструмент `memory` с действиями `add|replace|remove`; `replace`/`remove` находят запись по уникальному substring (`old_text`); действие `read` отсутствует — контент и так в промпте. Переполнение НЕ автокомпактится: возвращается ошибка с `current_entries` и `usage`, агент консолидирует в том же ходу (там же).
3. Авто-сохранение: агент сохраняет proactively (предпочтения → `user`, среда/конвенции/уроки → `memory`); фоновый self-improvement review после хода пишет память/скиллы, шлюз — `display.memory_notifications`, гейт — `memory.write_approval` со staged-записями `/memory pending|approve|reject` (там же).
4. Безопасность: дубликаты отвергаются автоматически; записи сканируются на prompt-injection/exfiltration-паттерны и невидимые Unicode-символы до записи (там же). Внешние провайдеры (`memory.provider`: Honcho, Mem0, Hindsight, … из 8 плагинов) аддитивны к встроенной памяти, prefetch перед ходом + sync хода после ответа (источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory-providers).
5. Honcho-провайдер: двухслойная инъекция (base: session summary + user representation + peer cards; dialectic: LLM-рассуждение) с ортогональными ручками `contextCadence` / `dialecticCadence` / `dialecticDepth` (1–3 pass: cold/warm prompt → self-audit → reconciliation), 5 инструментов `honcho_profile|search|context|reasoning|conclude` (источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/honcho).

## Vellum

Материал пользователя (local://vellum-summary.md) задаёт более богатую типологию, чем файловые сторы:

1. Восемь типов памяти — episodic, semantic, procedural, emotional, prospective, behavioral, narrative, shared — каждый со своим окном устаревания (staleness window). Изоляция per-user и per-channel; канал не «видит» чужую память (local://vellum-summary.md).
2. Гибридный dense + sparse retrieval; структурные элементы (identity, preferences, projects, events) извлекаются из диалогов с указанием источника и дедупликацией; эмбеддинги считаются локально по умолчанию (там же).
3. Смежное: per-user журнал размышлений (reflections), `NOW.md` — скретчпад текущего фокуса; ежечасная проактивная перечитка заметок. Actor identity (guardian/trusted/unknown): unknown-акторы не могут читать память — это ограничение доступа напрямую влияет на дизайн стора (там же).

## Решение (одно/комбо)

Комбо «Hermes-сторы как ядро + Mnemopi-подобный ретривал как росток». Базовый уровень — два bounded-стора (`memory.md` ≤2200 символов, `user.md` ≤1375) с инструментом `memory` (add/replace/remove, substring-match, capacity-error с выдачей текущих записей, dup-reject, скан на инъекции): это дёшево, детерминированно и решает 80% ценности без эмбеддингов. Поверх — один SQLite-стор с FTS5-таблицей + опциональная локальная dense-ветка: гибридный RRF-fusion и 8-типовая разметка Vellum вводим как поле `kind` + per-kind staleness TTL, а не как восемь физических сторов — это оптимум по стоимости (одна БД, один индекс) и по inode/памяти. Фоновый авто-retain (каждые N ходов, дедуп, redaction) берём от omp, frozen-снапшот инъекции — от Hermes (prefix-cache). Всё остальное (Hindsight-сервер, Honcho-диалектика) откладываем: внешние зависимости противоречат требованию эффективности локального клона.

## Rust-маппинг

Крейты: `titi-core` (типы памяти, трейты), `titi-tools` (инструменты `memory`, `memory_search`), `titi-cli` (команды `/memory`), `titi-tui` (просмотр/консолидация). Предлагаемый новый крейт `titi-memory` — сторы + retrieval + фоновая konsolidacija.

```rust
// titi-memory
pub enum MemoryTarget { Memory, User }                       // два bounded-стора
pub struct Entry { pub id: u64, pub kind: MemoryKind, pub text: String, pub at: DateTime<Utc> }
pub enum MemoryKind { Episodic, Semantic, Procedural, Emotional, Prospective, Behavioral, Narrative, Shared }
pub struct CapacityError { pub used: usize, pub limit: usize, pub entries: Vec<String> } // Hermes-style ошибка

pub trait MemoryStore: Send + Sync {
    fn add(&self, t: MemoryTarget, e: EntryDraft) -> Result<AddOutcome, CapacityError>;
    fn replace(&self, t: MemoryTarget, old_text: &str, new: &str) -> Result<(), EditError>; // substring match
    fn remove(&self, t: MemoryTarget, old_text: &str) -> Result<(), EditError>;
    fn snapshot(&self, t: MemoryTarget) -> FrozenSnapshot;      // инъекция в system prompt на старте
}
pub trait Retriever: Send + Sync {                              // hybrid dense+sparse
    async fn recall(&self, q: &str, limit: usize) -> Vec<Scored<Entry>>; // RRF over FTS5 + cosine
}
pub struct AutoRetainer { every_n_turns: u32 }                  // фоновое сохранение фактов
```

Внешние крейты: `rusqlite` (FTS5, `bundled`), `tokio` (фоновые задачи retention/consolidation), `serde`/`serde_yaml` (конфиг `memory.*`), `futures` (RRF-слияние потоков), `normpath`/`directories` (`~/.titi/memories/`). Подсистема «до dense» компилируется без внешних embedding-моделей; dense-ветка — feature-gated (`fastembed-rs`/`ort`, локальный bge-small).

## Definition of Done

- [ ] `titi-memory::MemoryStore` реализует add/replace/remove над `memory.md`/`user.md` с лимитами 2200/1375 символов; переполнение возвращает `CapacityError` (не молча режет).
- [ ] `replace`/`remove` матчатся по substring: 0 совпадений и >1 совпадение — ошибка с просьбой уточнить; точный дубликат при `add` не добавляется (dup-reject).
- [ ] Каждая запись перед сохранением проходит скан на паттерны инъекций/секретов (тест с фикстурой вредоносной строки отвергается).
- [ ] Frozen-снапшот инжектится в system prompt один раз на сессию; запись в ходе сессии не мутирует активный промпт (тест: снапшот до/после записи идентичен).
- [ ] SQLite-стор с FTS5-таблицей для фактов; `recall` гибридно сливает FTS и (feature) cosine-векторы через RRF; без dense-фичи recall работает FTS-only.
- [ ] Авто-retain срабатывает каждые `retain_every_n_turns` ходов, дедуплицирует и записывает в SQLite; в README-конфиге есть ключи `memory.*` (limits, autolearn, retain_every_n_turns).
- [ ] `kind` записи принимает все 8 значений `MemoryKind`; per-kind staleness TTL фильтрует устаревшие записи при recall (unit-тест с протухшей episodic-записью).
- [ ] CLI: `titi memory view|stats|clear` работают против активного стора (smoke-тест).

## Deep-dive

Подсистемные доки (написаны в этой фазе):

- [stores.md](./stores.md) — bounded-сторы, инструмент memory, 8 типов, гибридный retrieval, локальные эмбеддинги.
- [session-search-fts.md](./session-search-fts.md) — FTS5-поиск по сессиям, 4 формы вызова, schema БД.
- [journey-graph.md](./journey-graph.md) — граф обучения, scrubber, prune/edit узлов.
