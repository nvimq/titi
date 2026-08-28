# Компакция и контекст

Тема: стратегии компакции, пороги срабатывания, context-files (AGENTS.md и др.), prewalk, инъекция файлов. Материал собран из доков omp (`omp://compaction`, `omp://context-files`, `omp://prewalk`, `omp://ttsr-injection-lifecycle`), документации Hermes Agent и сводки Vellum (`local://vellum-summary.md`).

## omp

omp покрывает тему наиболее полно: отдельные механизмы компакции, context-files с приоритетами и dedup, prewalk-хэндофф и TTSR-инъекции правил.

1. **Модель компакции — first-class сессийные записи, а не простые сообщения.** `CompactionEntry` (`type: "compaction"`, поля `summary`, `firstKeptEntryId` — граница компакции, `tokensBefore`, опциональные `details`/`preserveData`) и `BranchSummaryEntry` (`type: "branch_summary"`, `fromId`, `summary`). При пересборке контекста (`buildSessionContext`) последняя компакция активного пути превращается в одно сообщение `compactionSummary`, записи от `firstKeptEntryId` до точки компакции — «kept» и досылаются как есть; `convertToLlm()` рендерит `compactionSummary`/`branchSummary` как user-сообщения через статические шаблоны `packages/agent/src/compaction/prompts/compaction-summary-context.md` и `branch-summary-context.md`. Источник: `omp://compaction`.
2. **Шесть триггеров компакции**: (1) ручной `/compact [instructions]`; (2) авто-восстановление после context-overflow ошибки той же модели (сначала пробуется context promotion на большую модель, потом walk по `compaction.methodOrder`); (3) авто-восстановление после `stopReason === "length"`; (4) пороговая поддержка после успешного хода, когда adjusted tokens > `resolveThresholdTokens(...)`; (5) mid-turn проверка перед следующим запросом провайдера на границах tool-loop (`compaction.midTurnEnabled`, по умолчанию `true`); (6) idle-компакция `runIdleCompaction()` (`compaction.idleEnabled`, порог `idleThresholdTokens` 200k, таймаут 300 c). Источник: `omp://compaction`.
3. **Методы и порядок.** `compaction.methodOrder` по умолчанию `["remote", "snapcompact", "handoff", "shake", "soft"]`; недоступный/упавший метод передаёт эстафету следующему. `remote` — provider-native компакция OpenAI-совместимых серверов (custom endpoint `{systemPrompt, prompt}` → `{summary}` либо `/chat/completions`, плюс Responses V2 streaming с `compaction_trigger` и бюджетом `compaction.v2RetainedMessageBudget` = 64000). `shake` — локальная механическая элизия: eligible tool results и крупные блоки заменяются на `artifact://`-ссылки с защищённым recent-окном; `snapcompact` — архив истории в PNG-кадры (model-aware shapes: Claude `11on16-bw`, Gemini `8on22-bw` 2048px и т.д.) без вызова модели, `preserveData.snapcompact` с head+tail-усечением tool-результатов (2000 chars, 0.6 head ratio). Источник: `omp://compaction`.
4. **Асинхронная (спекулятивная) компакция.** `compaction.asyncEnabled` = `true`: при входе в пре-пороговую полосу `[threshold − lead, threshold)` (lead = `clamp(threshold × 0.125, 8192, 32000)`) фоновая суммаризация первого LLM-метода идёт по снапшоту ветки в side-session; при реальном пересечении порога armed-результат коммитится мгновенно, скрывая латентность; discard при изменении префикса ветки. Источник: `omp://compaction`.
5. **Пороги.** `compaction.thresholdPercent` = `-1` / `thresholdTokens` = `-1` (положительный фиксированный лимит важнее процента, иначе резервный порог); резерв `compaction.reserveTokens` — floor 16384 токенов и ≥15% окна; `keepRecentTokens` = 20000; `autoContinue` = `true`. Источник: `omp://compaction`.
6. **Pre-compaction pruning и useless-result elision.** `pruneToolOutputs` защищает новейшие 40000 tool-токенов, требует ≥20000 экономии, не трогает результаты <50 токенов (`MIN_PRUNE_TOKENS` — плейсхолдер `[Output truncated - N tokens]` стоит ~8 токенов) и никогда не режет skill-результаты и `skill://`-read'ы. Флаг `AgentToolResult.useless` гасится плейсхолдером `[Uneventful result elided]` (`compaction.dropUseless`, по умолчанию on), cache-aware: только когда суффикс ≤ ~8k токенов или сессия простояла дольше жизни промпт-кэша. Источник: `omp://compaction`.
7. **Cut-point и split-turn.** `prepareCompaction()` работает только с записями после прошлой компакции; `findCutPoint()` режет на user/assistant/bashExecution/custom/branchSummary-границах, жёсткий запрет — резать на `toolResult`; метаданные (`model_change` и пр.) затягиваются в kept-регион. Разрезанный ход даёт два саммари (history + turn-prefix), слитые через `**Turn Context (split turn):**`. Источник: `omp://compaction`.
8. **Файловый контекст в саммари.** Компакция кумулятивно трекает read/write/edit по tool-call'ам и добавляет в summary тег `<files>` — prefix-folded дерево с маркерами `(Read)/(Write)/(RW)`, cap 20 файлов. Источник: `omp://compaction`.
9. **Context-files — multi-provider discovery с приоритетами.** Провайдеры: `native` (`.omp/AGENTS.md`, приоритет 100), `claude` (80), `agents`/`codex` (70), `gemini` (60), `opencode` (55), `github` (30), `agents-md` (standalone `AGENTS.md`, 10). Один user-файл на все провайдеры, один project-файл на глубину каталога (cwd = depth 0), byte-identical коллапсируются; порядок инъекции — дальние предки первыми, user-файл последним. Инъекция — блок `<repo-rules>` с `<file path="...">` на файл. Источник: `omp://context-files`.
10. **`@`-импорты и sticky-правила.** Внутри context-file `@path` разворачивается рекурсивно (до 5 хопов, циклы скипаются, код-блоки не трогаются, относительные пути — от импортирующего файла). Отдельно `RULES.md` (только native-локации) — always-apply sticky-правило, переприкрепляемое возле текущего хода, чтобы переживать длинные сессии. Отключение файлов — `disabledProviders` (весь источник) или `disabledExtensions` c id `context-file:<level>:<basename>`. Источник: `omp://context-files`.
11. **Prewalk — one-shot хэндофф на дешёвую модель после планирования.** `prewalk.enabled` (по умолчанию off, таргет `@smol`); arm через `omp --prewalk` / `--prewalk-into <model|role>` / `/prewalk`. Ворота открывает любой успешный вызов `todo` (включая read-only `view`), свитч модели происходит после первого `edit`/`write`; после свитча prewalk disarms. Источник: `omp://prewalk`.
12. **TTSR-инъекции — стриминговый монитор правил.** `TtsrManager` + `TtsrCoordinator` мониторят `text_delta`/`thinking_delta`/`toolcall_delta` на regex/ast-grep-условия правил; при совпадении с `interruptMode` — немедленный `agent.abort()`, затем через 50ms инъекция `<system-interrupt reason="rule_violation" ...>` (custom_message `customType: "ttsr-injection"`) и `agent.continue()`; non-interrupting tool-совпадения вшивают `<system-reminder>` в начало tool-результата через `afterToolCall`. Repeat-политики `once`/`after-gap` (`repeatGap` = 10 ходов), персист `ttsr_injection`-записей. Источник: `omp://ttsr-injection-lifecycle`.

## Hermes

Hermes покрывает тему через двухслойную систему компрессии, агрессивное управление context-files и политики reset сессий.

1. **Двухслойная компакция с разными порогами.** Слой 1 — Gateway Session Hygiene в `gateway/run.py`: safety-net, срабатывает на **85%** окна модели до обработки сообщения (ловит сессии, убежавшие от компрессора за ночь в Telegram/Discord; при 50% вызывал преждевременную компрессию каждый ход). Слой 2 — агентный `ContextCompressor` (`agent/context_compressor.py`) внутри tool-loop: порог `compression.threshold` = **0.50** от контекстного окна (по токенам главного агента, не суммаризатора), per-model overrides `compression.model_thresholds` (substring-match, longest wins, floor 0.75 для окон <512K). Источник: https://hermes-agent.nousresearch.com/docs/developer-guide/context-compression-and-caching.
2. **4-фазный алгоритм компрессии.** Фаза 1 — дешёвая обрезка старых tool-результатов (>200 chars) на `[Old tool output cleared to save context space]` без LLM; фаза 2 — границы: `protect_first_n: 3` (закреплено) + хвост по токен-бюджету с fallback на `protect_last_n: 20` и `min_tail_user_messages: 1`, выравнивание границ `_align_boundary_backward()` чтобы не рвать пары tool_call/tool_result; фаза 3 — структурированный саммари (Goal/Constraints/Progress/Key Decisions/Relevant Files/Next Steps/Critical Context), бюджет `content_tokens × 0.20`, min 2000, max `min(context × 0.05, 12000)`; фаза 4 — сборка + санитизация orphan tool-пар `_sanitize_tool_pairs()`. Итеративная рекомпрессия: предыдущий саммари подаётся на **обновление**, а не с нуля. Источник: тот же URL.
3. **`tail_mode: lean` — компактный хвост с recovery-механизмами.** Вместо legacy-хвоста `target_ratio` (0.20, ~100K+ токенов на больших окнах) lean держит clamp 2.5% окна (floor 10K, cap 25K) и переносит непрерывность в саммари: chunked identifier-preserving дайджесты, механически извлечённый anchor-индекс (PR-номера, SHA, пути, error strings — regex, никогда не парафраз), каждое реальное user-сообщение дословно, и recovery-указатель на `session_search`. На реальных 500K-токеновых сессиях: ~49K удержанных против ~162K при большем recall. Источник: тот же URL.
4. **In-place компакция.** `compression.in_place: true` (default): компакция переписывает живой список сообщений на **том же session id**, пре-компакционные ходы soft-архивируются (`active=0, compacted=1`) — остаются доступными через `session_search`, никогда не удаляются; устранила баг-кластер ротации сессий (потеря `/goal`, сироты, разрывы поиска). Источник: тот же URL.
5. **Session reset / flush политики.** По умолчанию gateway-сессии **никогда не авторесетятся** (`session_reset.mode: none`); opt-in политики: `idle` (после N минут неактивности), `daily` (в заданный час), `both`. Перед авторесетом агенту даётся ход сохранить важные памяти/скиллы; сессии с активными фоновыми процессами не сбрасываются никогда. Ручной flush — `/new` (опционально `/new <имя>`) или `/reset`; компрессия — `/compress` (+ `/compress here [N]`, `/compress focus <topic>`), recovery после компрессии — `session_search` по архивным ходам. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/sessions и https://hermes-agent.nousresearch.com/docs/guides/troubleshooting-agent-quality.
6. **Context-files: один проект-файл + прогрессивная инъекция сабдиректорий.** Приоритет: `.hermes.md` → `AGENTS.override.md` → `AGENTS.md` → `CLAUDE.md` → `.cursorrules` (first match wins); `SOUL.md` грузится только из `HERMES_HOME` как личность (слот #1). В git-репо грузится цепочка AGENTS.md от корня до cwd (глубже — позже в промпте, идентичные дедуплицируются). `SubdirectoryHintTracker` в `agent/subdirectory_hints.py` после каждого tool-call извлекает пути из аргументов, проверяет каталог и до 5 предков, инъектирует найденный AGENTS.md **в tool-результат** (cap 8000 chars/файл) в момент, когда каталог стал релевантным — «no system prompt bloat» + стабильный system prompt для кэша. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/context-files.
7. **Транкция и security-scan context-files.** Лимит `context_file_max_chars` или динамический (floor 20000, ceiling 500000 chars, от окна модели); усечение head 70% / tail 20% с маркером 10% посередине. Все файлы сканируются на prompt-injection (instruction override, скрытые HTML-комментарии, exfiltration-паттерны `curl ... $API_KEY`, zero-width/bidi-символы) — заражённый файл блокируется целиком: `[BLOCKED: AGENTS.md contained potential prompt injection ...]`. Источник: тот же URL.
8. **Prompt-caching как часть контекст-менеджмента.** Anthropic-стратегия `system_and_3`: 4 breakpoint'а `cache_control` — system prompt + rolling-окно из 3 последних не-системных сообщений; TTL `5m`/`1h` (`prompt_caching.cache_ttl`); компрессия инвалидирует кэш только сжатой области, system-кэш переживает; смена модели/credential mid-session обнуляет кэш полностью. Источник: https://hermes-agent.nousresearch.com/docs/developer-guide/context-compression-and-caching.

## Vellum

Vellum **пороги и механику компакции не покрывает** — в материале нет ни token-порогов, ни summary-стратегий, ни механизмов усечения истории. Ближайшие аналоги — управление контекстом через ограниченную, структурированную память вместо сжатия транскрипта:

1. **Память как замена длинной истории.** Восемь типов памяти (episodic, semantic, procedural, emotional, prospective, behavioral, narrative, shared), у каждого своё **окно устаревания (staleness window)**; гибридный dense + sparse retrieval; изоляция per-user и per-channel. Это альтернативная философия: не сжимать растущую историю компакцией, а держать рабочие знания в bounded-хранилище с ретривом. Источник: `local://vellum-summary.md`.
2. **NOW.md — скретчпад текущего фокуса.** Поведение в SOUL.md; per-user журнал размышлений (reflections) и `NOW.md` — скретчпад активных нитей. Это функциональный аналог handoff-документа omp: контекст «что происходит сейчас» вынесен из переписки в файл, который перечитывается при каждом взаимодействии. Источник: `local://vellum-summary.md`.
3. **Proactive-переосмысление вместо ретро-компакции.** Каждый час ассистент перечитывает свои заметки, ищет незавершённое и обновляет состояние — контекст поддерживается актуальным фоновым процессом, а не сжимается по порогу. Источник: `local://vellum-summary.md`.
4. **Что конкретно отсутствует**: token-пороги, summary-модели, cut-point логика, усечение tool-результатов, context-file discovery — в сводке не описаны. Для компакции Vellum — источник идей (staleness, NOW.md, reflections), а не механики.

## Решение (одно/комбо)

Комбо, ядро — omp-модель с lean-хвостом Hermes. Берём omp-архитектуру как канон: first-class `CompactionEntry` с `firstKeptEntryId` (перезаписываемая граница + нетронутый display-transcript через дивидеры), шесть триггеров (ручной, overflow, incomplete, threshold, mid-turn, idle) и chain of methods (`provider-native → snapcompact → handoff → shake → local-summary`) с прогрессивным фоллбэком — это самое эффективное решение по latency (shake/snapcompact не зовут модель, спекулятивный prefetch прячет LLM-латентность, promotion спасает от лишней компакции). Из Hermes добавляем две дешёвые и доказанно работающие вещи: `tail_mode: lean` (clamp-хвост 2.5% окна + identifier-anchor-индекс + recovery-указатель на полнотекстовый поиск по архиву — на 3× меньше удержанных токенов при том же recall) и in-place компакцию с soft-архивом (один session id, не теряем поиск). Из Vellum берём только идею «скретчпад текущего фокуса» (NOW.md-аналог) как содержимое handoff-документа. Threshold-механику Vellum не применяем: для coding-харнессаOMP-модель порогов точнее, чем bounded-память с ретривом.

## Rust-маппинг

**Крейты workspace:**

- `titi-core` — ядро компакции и контекста: entry-модель, триггеры, метод-chain, pruning.
- `titi-providers` — provider-native компакция (Responses `compaction_trigger`, `/chat/completions`-remote endpoint, Vision-тарификация для snapcompact), token-counting по API usage.
- `titi-tools` — флаги tool-результатов (`useless`), перехват путей для прогрессирующей инъекции context-files, `todo`-сигнал для prewalk.
- `titi-tui` — display-transcript с дивидерами компакций (`── compacted · ctrl+o ──`), индикатор спекулятивной компакции.
- `titi-cli` — команды `/compact`, `/shake`, `/handoff`, флаги `--prewalk`/`--prewalk-into`, конфиг-схема.
- предлагаемый новый крейт: `titi-snapcompact` — сериализация истории + рендер PNG-кадров (шрифты, шейпы по семействам моделей).

**Ключевые типы (эскизы, titi-core):**

```rust
pub struct CompactionEntry {
    pub summary: String,
    pub short_summary: Option<String>,
    pub first_kept_entry_id: EntryId,
    pub tokens_before: u64,
    pub details: Option<CompactionDetails>, // read_files, modified_files
    pub preserve_data: Option<PreserveData>, // snapcompact, openai_remote_compaction
}

pub enum CompactionTrigger { Manual, Overflow, Incomplete, Threshold, MidTurn, Idle }

#[async_trait]
pub trait CompactionMethod {
    fn name(&self) -> &'static str;
    async fn run(&self, ctx: &CompactionCtx, reason: CompactionTrigger) -> Result<CompactionOutcome>;
}
// реализации: RemoteCompaction, Snapcompact, Handoff, Shake, SoftSummary
// SessionMaintenance обходит Vec<Box<dyn CompactionMethod>> по method_order с фоллбэком

pub trait ContextFileProvider {
    fn id(&self) -> &'static str;          // "native", "agents-md", ...
    fn priority(&self) -> u8;              // 100..=10
    fn discover(&self, cwd: &Path) -> Vec<ContextFile>; // (path, scope, depth)
}

pub struct ThresholdPolicy {
    pub threshold_percent: Option<f64>,    // None => reserve-based
    pub threshold_tokens: Option<u64>,     // приоритет над percent
    pub reserve_tokens: u64,               // floor 16384, >=15% окна
    pub keep_recent_tokens: u64,           // 20000
}

pub struct PrewalkArmed { pub target: ModelRef, /* disarm после первого edit/write */ }
```

**Внешние крейты:** `tiktoken-rs` (оценка токенов для порогов/cut-point; API-reported usage — из titi-providers), `image` + встроенные bitmap-шрифты (snapcompact-кадры), `rusqlite` (персист сессий и `compaction`/`branch_summary`/`ttsr_injection` записей), `regex` + `ast-grep` Rust-binding (TTSR-условия правил), `tokio` (спекулятивная фоновая компакция в side-session, idle-таймер), `fancy-regex` не нужен — достаточно `regex`.

## Definition of Done

- [ ] `titi-core::compaction` реализует все 6 триггеров; unit-тест подтверждает: threshold-триггер не срабатывает ниже `resolveThresholdTokens`, mid-turn — не срабатывает внутри tool-loop без границы.
- [ ] Chain of methods проходит фоллбэк-тест: при недоступном provider-native вызове выполняется следующий метод и событие `compaction_method_fallback` публикуется.
- [ ] Cut-point тесты: компакция никогда не режет на `toolResult`; split-turn ход даёт merged-саммари с секцией `**Turn Context (split turn):**`; метаданные (`model_change`) затянуты в kept-регион.
- [ ] Pruning-тесты: tool-результат <50 токенов не блючится; `skill://`-read и skill-результаты переживают `pruneToolOutputs`; `useless`-флаг даёт `[Uneventful result elided]`.
- [ ] Context-files: интеграционный тест на fixture-репо проверяет приоритеты (native > claude > agents-md), one-user-file dedup, per-depth project-файлы, `@`-импорт (вкл. цикл — скип, 5 хопов — стоп), инъекцию `<repo-rules><file path=...>`.
- [ ] Display-transcript тест: после компакции scrollback сохраняется, дивидер рендерится в точке срабатывания, LLM-контекст начинается с `firstKeptEntryId`.
- [ ] Snapcompact: golden-тест рендера кадра для каждого shape-семейства (claude/gemini/openai/kimi) и test на реконструкцию plain-text/image/plain-text блоков из `preserveData` при пересборке контекста.
- [ ] Prewalk: тест one-shot-семантики — handoff срабатывает после `todo` + первого `edit`, и не срабатывает повторно до re-arm.

## Deep-dive

План подсистем-доков второго уровня (пишутся при необходимости):

- `docs/research/compaction-context/compaction-pipeline.md` — pipeline omp: триггеры, cut-point, split-turn, персист, display-transcript.
- `docs/research/compaction-context/methods.md` — детальный разбор пяти методов: remote/V2-streaming, snapcompact (shapes, тарификация), handoff, shake, soft; спекулятивная async-компакция.
- `docs/research/compaction-context/context-files-discovery.md` — таблица провайдеров omp и Hermes, приоритеты, shadowing, RULES.md, disabledProviders/disabledExtensions.
- `docs/research/compaction-context/progressive-injection.md` — Hermes SubdirectoryHintTracker и omp `<dir-context>`: стратегии late-binding контекста в сабдиректориях.
- `docs/research/compaction-context/ttsr-injection.md` — стриминговый монитор правил, interrupt/deferred пути, repeat-политики, персист.
- `docs/research/compaction-context/prewalk-handoff.md` — prewalk-триггеры, роли моделей, связь с handoff-документом и NOW.md-скретчпадом.
