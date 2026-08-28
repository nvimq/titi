# Хранилища памяти: bounded-сторы, инструмент memory, типы памяти, гибридный retrieval

Как агент хранит долговременные факты: жёстко ограниченные файловые сторы с ручным курированием моделью, авто-сохранение фактов, типологизация памяти (8 типов Vellum), гибридный dense+sparse retrieval на локальных эмбеддингах.

## omp

omp реализует память как сменные бэкенды, а не bounded-файлы:

1. `memory.backend` ∈ {`off`, `local`, `hindsight`, `mnemopi`}, по умолчанию `off` (источник: omp://memory.md). Бэкенд `local` — двухфазный фоновый пайплайн: Phase 1 извлекает per-session durable-сигнал (роль `default`), Phase 2 консолидирует (роль `smol`) в `MEMORY.md` + `memory_summary.md` + `skills/`; аренда/heartbeat против двойного запуска; секреты редактируются перед записью на диск (omp://memory.md). Конфиг-ключи: `memories.summaryInjectionTokenLimit` (5000, общий лимит инъекции summary+lessons), `memories.maxRolloutsPerStartup` (64), `memories.stage1Concurrency` (8) (omp://memory.md).
2. Инструменты: `retain` (`items: [{content, context?}]`; Mnemopi-путь: synchronous `rememberScoped` с `importance: 0.75`, `memoryType: "fact"`, extract entities; Hindsight-путь: очередь, flush при 16 элементах или debounce 5 c) (omp://tools/retain.md); `recall` (bullet-формат с `(id: <id>)` и preview ≤500 символов, `recallLimit=8`); `reflect` (в Mnemopi — это локальный recall+форматирование, синтез-модель не вызывается) (omp://tools/recall.md, omp://tools/reflect.md).
3. `memory_edit` — `update|forget|invalidate` по id; importance клампится в 0..1; fact-строки read-only (`not_editable`); перед `update` обязательно `read memory://<id>`, потому что preview обрезан и заливка превью убьёт невидимый хвост (omp://tools/memory_edit.md).
4. Авто-сохранение и retrieval-механика: Mnemopi auto-retain каждые `retainEveryNTurns=4` (memoryType `episode`, importance 0.65); `polyphonicRecall` — 4 голоса (vector, graph, fact, temporal) + reciprocal rank fusion; `enhancedRecall` — тировый кеш повторных запросов; локальные эмбеддинги `BAAI/bge-base-en-v1.5` (768d) / `intfloat/multilingual-e5-large` (1024d), `noEmbeddings: true` → FTS-only; `proactiveLinking` — новых воспоминания линкуются в эпизодический граф при записи (omp://mnemosyne-memory-backend.md). Scoping банков: `global` / `per-project` (basename+hash cwd) / `per-project-tagged` (запись в проект, recall из проекта+глобала) — там же.
5. Ближайший аналог bounded-сторов: файловый `local`-бэкенд с `learned.md` — newest-first, дедуп по нормализованной строке, cap 100 записей, lesson ≤2000 символов / context ≤400, secret-redaction, инъекция со следующей сессии без мутирования prompt-cache (omp://memory.md, omp://tools/learn.md).

## Hermes

Hermes — эталон bounded curated memory; отсюда берём ядро:

1. Два стора с жёсткими лимитами: `MEMORY.md` — 2,200 символов (~800 токенов, 8–15 записей), `USER.md` — 1,375 символов (~500 токенов, 5–10 записей); лежат в `~/.hermes/memories/`, инжектятся в system prompt frozen-снапшотом на старте сессии (сохраняет prefix-cache); хедер блока показывает `% — used/limit chars`, записи разделены `§`, многострочные разрешены (источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory).
2. Инструмент `memory`: `add` / `replace` / `remove`; действия `read` нет. `replace`/`remove` используют substring matching: `old_text` должен однозначно идентифицировать одну запись, иначе ошибка «match multiple entries → уточните» (там же).
3. Capacity errors без автокомпакта: при переполнении `add` возвращает `{success:false, error: "Memory at 2,100/2,200 chars. Adding this entry (250 chars) would exceed the limit. Consolidate now: …", current_entries: [...], usage: "2,100/2,200"}`; `replace` тоже связан лимитом (замена на более длинную может переполнить). Рекомендация: выше 80% — консолидация до добавления (там же).
4. Авто-сохранение фактов: agent saves proactively — предпочтения → `user`; среда, конвенции, коррекции, завершённая работа, явные просьбы → `memory`; skip: тривиальное, легко переоткрываемое, сырые дампы, сессионная эфемерка. Фоновый self-improvement review после хода пишет память/навыки; `memory.write_approval: true` ставит записи (включая фоновые, тег `[auto]`) в staged-очередь `/memory pending|approve <id>|reject <id>`; `auxiliary.background_review.provider/model` гоняет review на дешёвой модели с digest-реплеем вместо полного транскрипта; `display.memory_notifications: off|on|verbose` управляет уведомлениями `💾 Memory updated` (там же, там же).
5. Гигиена: точные дубликаты отвергаются («no duplicate added»); перед записью скан на prompt-injection, credential-exfiltration, SSH-backdoor паттерны и невидимые Unicode. Отключение: `memory_enabled`/`user_profile_enabled: false` убирает тул и guidance из схемы (модель не узнаёт о несуществующем тулe); external provider (8 плагинов, `memory.provider`) аддитивен: inject → prefetch перед ходом → sync хода → extract на session end → mirror встроенных записей (источники: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory, https://hermes-agent.nousresearch.com/docs/user-guide/features/memory-providers).
6. Honcho как модель «памяти с рассуждением»: two-layer инъекция — base (session summary + user representation + peer card, refresh по `contextCadence`) + dialectic supplement (LLM-reasoning по `dialecticCadence`, глубина `dialecticDepth` 1–3: pass 0 cold/warm prompt, pass 1 self-audit, pass 2 reconciliation; уровни `dialecticDepthLevels`); query-adaptive reasoning (+1 уровень на ≥120 символов, +2 на ≥400, cap `reasoningLevelCap`); session-start prewarm на полную глубину; 5 инструментов `honcho_profile|search|context|reasoning|conclude`; recallMode `hybrid|context|tools` (источники: https://hermes-agent.nousresearch.com/docs/user-guide/features/honcho, https://hermes-agent.nousresearch.com/docs/user-guide/features/memory-providers).

## Vellum

1. Восемь типов памяти: episodic, semantic, procedural, emotional, prospective, behavioral, narrative, shared — у каждого своё окно устаревания (staleness window); изоляция per-user и per-channel (local://vellum-summary.md). Это отличается и от плоских Hermes-сторов (нет типов), и от omp Mnemopi (типы только working/episodic/fact).
2. Гибридный dense + sparse retrieval: dense — локальные эмбеддинги (по умолчанию считаются на машине, без облачного API), sparse — текстовый поиск; изоляция per-user/per-channel применяется на уровне запроса. Структурные элементы (identity, preferences, projects, events) извлекаются из диалогов с сохранением источника и дедупликацией (там же).
3. Смежные механизмы, влияющие на стор: per-user журнал reflections, `NOW.md` (скретчпад текущего фокуса); часовой цикл перечитки заметок; actor identity (guardian/trusted/unknown) — unknown не может читать память, т.е. стор обязан проверять identity при каждом чтении (там же). Харнесс не даёт готовых алгоритмов слияния dense/sparse — ближайший публичный аналог в собранном материале: polyphonic recall omp с reciprocal rank fusion (omp://mnemosyne-memory-backend.md).

## Решение (одно/комбо)

Комбо. Ядро — Hermes-схема: два bounded-стора `memory.md`/`user.md` (лимиты 2200/1375 символов, кириллица уместится — лимит в символах, не токенах), инструмент `memory` с add/replace/remove, substring-match, capacity-error с выдачей `current_entries`, dup-reject и security-сканом. Авто-сохранение — двухуровневое: явное сохранение моделью в ходу + фоновый post-turn review (по умолчанию на дешёвой «tiny»-роли, как omp `smol`), с опциональным staged-режимом `/memory pending`. Типологию Vellum не размазываем по физическим сторам: поле `kind` из 8 значений + per-kind staleness TTL в единой SQLite-таблице фактов — так retrieval остаётся одним индексом (эффективно), а семантика типов и окошек устаревания сохраняется. Гибридный retrieval: FTS5 (sparse) всегда; dense-ветка на локальных эмбеддингах — feature-gated опция (bge-small через локальный ONNX-рантайм), слияние через RRF как у polyphonic recall omp. Экономика: без dense-фичи клон полностью функционален (Hermes-уровень), dense включается только когда база фактов перерастает keyword-поиск.

## Rust-маппинг

Крейты: `titi-core` — типы и трейты; новый `titi-memory` — сторы, retrieval, авто-retain; `titi-tools` — регистрация инструмента `memory`; `titi-cli` — `/memory view|stats|clear|pending`; `titi-tui` — просмотр/консолидация записей.

```rust
// titi-core::memory
#[derive(Serialize, Deserialize)]
pub struct Entry { pub id: u64, pub kind: MemoryKind, pub text: String, pub added_at: DateTime<Utc> }
pub enum MemoryKind { Episodic, Semantic, Procedural, Emotional, Prospective, Behavioral, Narrative, Shared }
pub struct StoreLimit { pub max_chars: usize }                       // 2200 / 1375
pub struct CapacityError { pub used: usize, pub limit: usize, pub entries: Vec<String> } // Hermes payload
pub enum EditError { AmbiguousMatch(Vec<String>), NoMatch, ReadOnlyFacts, InjectionBlocked }

#[async_trait]
pub trait MemoryStore: Send + Sync {
    fn add(&self, t: Target, draft: Draft) -> Result<Added, CapacityError>;      // Added::Duplicate при dup
    fn replace(&self, t: Target, old_text: &str, new: String) -> Result<Added, EditError>;
    fn remove(&self, t: Target, old_text: &str) -> Result<usize, EditError>;
    fn render_snapshot(&self) -> String;                                          // frozen, один раз на сессию
}

// titi-memory
pub struct FactTable { db: rusqlite::Connection }                   // таблица фактов + FTS5
impl FactTable {
    pub fn recall(&self, q: &str, limit: usize) -> Result<Vec<Scored>, Error>;     // FTS5 ветка
    pub fn recall_hybrid(&self, q: &str, limit: usize) -> Result<Vec<Scored>, Error>; // RRF(fts, dense), cfg feature
    pub fn stale_filter(&self, cut: impl Fn(MemoryKind) -> chrono::Duration);
}
pub struct AutoRetainer { every_n_turns: u32, dedup: bool, redactor: SecretRedactor }
pub struct BackgroundReview { role: ModelRole /* smol */, write_approval: bool /* staged queue */ }
```

Внешние крейты: `rusqlite` (bundled → FTS5), `tokio` (фоновые таски review/retain, аренды через `tokio::sync`), `serde`, `serde_yaml`, `async-trait`, `chrono` (staleness windows), `regex` (redaction/injection-паттерны), `unicode-normalization` (детект невидимых Unicode, как Hermes-скан). Dense-ветка (feature `dense`): `fastembed-rs` или `ort` + tokenizers с моделью bge-small-en-v1.5; cosine в f32-векторах, хранение blob в SQLite.

## Definition of Done

- [ ] `titi-memory`: `add` в полный стор возвращает `CapacityError { used, limit, entries }` и не пишет; `replace` более длинной записью до переполнения — тоже ошибка (тест на оба случая).
- [ ] Substring-match: `old_text` с 0 и >1 совпадениями даёт `EditError::NoMatch` / `AmbiguousMatch` с листингом записей; уникальный substring заменяет/удаляет ровно одну запись.
- [ ] Точный дубликат при `add` возвращает `Added::Duplicate` без вставки (тест).
- [ ] Строка с паттерном exfiltration или невидимым Unicode (U+200B) отвергается до записи (тест-фикстура).
- [ ] Frozen-снапшот: `render_snapshot()` даёт хедер `MEMORY (x% — n/2200 chars)`, `§`-разделители; вызов в течение сессии возвращает один и тот же байт-в-байт результат, записи в БД при этом обновляются.
- [ ] `AutoRetainer` пишет факт каждые `every_n_turns` ходов, дедуплицирует по нормализованному тексту, прогоняет redactor; unit-тест с фейковыми ходами.
- [ ] Таблица фактов имеет `kind` со всеми 8 вариантами и per-kind TTL; recall с протухшей `Episodic`-записью её не возвращает (тест).
- [ ] `recall` без feature `dense` работает чисто на FTS5; с фичей — RRF-слияние детерминировано при одинаковых входных данных (golden-тест).

## Deep-dive

Дальнейшие подсистемные доки (2-й уровень):

- [session-search-fts.md](./session-search-fts.md) — FTS5-поиск по всем прошлым сессиям: schema, 4 формы вызова, адаптивная детализация.
- [journey-graph.md](./journey-graph.md) — граф обучения: таймлайн навыков+памяти, scrubber, prune/edit.

Кандидаты на будущие доки: `titi-memory/embeddings.md` (локальный ONNX-инференс, выбор модели, деградация без неё), `titi-memory/consolidation.md` (фоновый review, staged writes, аренды).
