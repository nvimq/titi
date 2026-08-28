# Стриминг событий: единый контракт, нормализация и thinking-уровни

Как разнородные потоки Anthropic (`content_block_delta`), OpenAI (`response.*` lifecycle) и Gemini (`parts`-чанки) сводятся к одному набору событий, какие квирки моделей проявляются прямо в стриме и как передаются thinking-уровни.

## omp

1. **Единый контракт `AssistantMessageEvent`** (`packages/ai/src/types.ts`): `start`; триплеты `text_start→text_delta*→text_end`, `thinking_*`, `toolcall_*`; `image_end`; терминал `done` с `reason: stop|length|toolUse` либо `error` с `aborted|error`. `AssistantMessageEventStream` доставляет события немедленно в порядке push (без батчинга), `result()` резолвится на `done`/`error` (omp://provider-streaming-internals.md).
2. **Троттлинг переехал в парсинг tool-аргументов**: сама очередь не троттлит; частичный JSON аккумулируется в `partialJson` и перепарсивается `parseStreamingJsonThrottled()` только после ≥256 новых байт (`STREAMING_JSON_PARSE_MIN_GROWTH`), что сводит стоимость с квадратичной к линейной; финальный парс на `toolcall_end` — безусловный и авторитетный. Парсинг: `JSON.parse` → самописный repairing-парсер `RelaxedJson` → `{}` (omp://provider-streaming-internals.md).
3. **Stop-reason таблицы по провайдерам**: Anthropic `end_turn→stop`, `max_tokens→length`, `tool_use→toolUse`, safety→`error`; OpenAI Responses `completed→stop`, `incomplete→length`, `failed/cancelled→error`; Google `STOP→stop`, `MAX_TOKENS→length`, safety/malformed-function-call→`error`. Плюс продвижение: у Chat Completions `finish_reason: "stop"` повышается до `toolUse`, если в ходе были структурные tool-блоки (omp://provider-streaming-internals.md, omp://provider-quirks.md).
4. **Thinking-уровни — каноническая шкала `Effort`**: `minimal|low|medium|high|xhigh|max` (`packages/catalog/src/effort.ts`); per-model `ThinkingConfig` с `mode` (`effort`, `budget`, `google-level`, `anthropic-adaptive`, `anthropic-budget-effort`), `efforts`, `effortMap`, `requiresEffort`, `suppressWhenOff`. Wire-маппинг: Anthropic adaptive = `thinking:{type:"adaptive"}` + `output_config.effort` (beta `effort-2025-11-24`) либо budget `thinking:{type:"enabled",budget_tokens:N}`; Responses = `reasoning:{effort,summary}`; Chat = `reasoning_effort` или dialect-поля (`thinking:{type:"enabled"}` zai, `enable_thinking` qwen, `reasoning:{enabled:false}` openrouter); Gemini = `thinkingConfig:{thinkingLevel|thinkingBudget, includeThoughts}` (omp://provider-compat-reference.md, omp://provider-quirks.md).
5. **Квирки прямо в декодере стрима**: reasoning-дельты MiniMax-M3 — кумулятивные снапшоты (дедуп по сигнатуре); DeepSeek-эндпоинты утекут chat-template маркеры `<｜...｜>` в видимый текст — буферизация и стриппинг с учётом разрезания маркера между чанками; Mistral Medium 3.5 шлёт `delta.content` массивом текстовых частей; `StreamMarkupHealing` реконструирует утекшие XML/markdown tool-call'ы в структурные события и повышает `stop→toolUse`; `wrapLeakedThinkingStream` превращает in-band ` ```thinking ` фенсы в thinking-блоки; `withThinkingLoopGuard` убивает зацикленный reasoning (omp://provider-quirks.md, omp://provider-compat-reference.md).

## Hermes

1. **Сохранение reasoning-метаданных OpenAI-wire**: Hermes сохраняет `reasoning`, `reasoning_content` и стриминговые reasoning-дельты, когда OpenAI-совместимые серверы их возвращают; метаданные трактуются как thinking-trace, а не замена видимого ответа; для Qwen на vLLM с `--reasoning-parser qwen3` ожидается непустой `content`, иначе — рекомендация отключить парсер (https://hermes-agent.nousresearch.com/docs/integrations/providers).
2. **Auxiliary-модель на отдельном маршруте**: vision, web-summ, MoA используют отдельный auxiliary-модельный вызов; по умолчанию `auxiliary.*.provider: "auto"` маршрутизирует на main-модель, можно переопределить на дешёвую модель (например Gemini Flash) (https://hermes-agent.nousresearch.com/docs/integrations/providers).
3. **xAI reasoning без параметров**: xAI ходит через Responses API, Grok 4 «reasons by default» — `reasoning_effort` не передаётся, и усилие reasoning у Actual-эндпоинтов клампится в `none/low/medium/high/max`, чтобы глобальный `xhigh` не давал 400 (https://hermes-agent.nousresearch.com/docs/integrations/providers).

## Vellum

Харнес тему нормализации стрим-событий не покрывает (материал о памяти/SOUL/безопасности/каналах). Ближайший аналог:

1. **Извлечение структурированных элементов (identity, preferences, projects, events) из диалогов с дедупликацией** (local://vellum-summary.md, «Память») — тот же принцип, что и в стриме: сырой текст модели нормализуется в структуру с дедупом (аналог дедупа кумулятивных reasoning-снапшотов omp).
2. **Per-user/per-channel изоляция** (local://vellum-summary.md) — для стриминга titi означает изоляцию потока событий на бота/канал: события одного хода не пересекаются с другим (мультиботность).

## Решение (одно/комбо)

Комбо: **контракт omp как канон + их же политики деградации**. Берём enum `StreamEvent` с триплетами-жизненными циклами и push-доставкой без батчинга (проверено на десятке провайдеров), repairing-парсер частичного JSON с троттлингом ≥256 байт (важно для TUI-рендера в Rust: без троттлинга re-parse на каждую дельту съедает CPU) и таблицы stop-reason. Квирки (кумулятивный reasoning, утечки шаблонных маркеров, markup-healing, thinking-loop guard) — это не provider-name ветки, а декларативные флаги в `CompatPolicy`/декодер-политике endpoint-семейства, включаемые из каталога моделей. Thinking-уровни: одна шкала `Effort` + per-model лествица с клампом и `effortMap`; где у хоста нет выключателя — `lowest-effort`/`suppressWhenOff` режимы omp. От Hermes берём clamp усилия на нестандартных эндпоинтах как страховку от 400.

## Rust-маппинг

Крейты: `titi-providers`, `titi-core`, `titi-tui` (рендер дельт). Внешние: `serde_json` (+ самописный `partial_json`-модуль), `smol_str` (дельты без аллокаций где возможно), `tokio::sync::mpsc` (push-очередь событий).

```rust
// titi-core — контракт (см. README); здесь — специфика
pub struct ThinkingConfig {
    pub mode: ThinkingMode,           // Effort | Budget | GoogleLevel | AnthropicAdaptive
    pub efforts: &'static [Effort],   // лествица модели
    pub effort_map:FxHashMap<Effort, SmolStr>,
    pub requires_effort: bool,        // нельзя выключить
    pub suppress_when_off: bool,      // "off" надо послать явно
}
pub enum Effort { Minimal, Low, Medium, High, Xhigh, Max }
pub fn clamp(level: Effort, cfg: &ThinkingConfig) -> Effort;

// тит-providers: декодер-политика endpoint-семейства (не ветки по имени провайдера)
pub struct StreamDecodePolicy {
    pub reasoning_deltas_cumulative: bool,   // MiniMax-класс
    pub strip_special_tokens: Option<Regex>, // DeepSeek-класс
    pub markup_healing: Option<HealingPattern>, // kimi | dsml | thinking
    pub content_is_parts_array: bool,        // Mistral-класс
    pub thinking_loop_guard: LoopGuardConfig,
}
impl StreamDecodePolicy { pub fn from_compat(c: &CompatPolicy) -> Self; }

// repairing-парсер с троттлингом (аналог parseStreamingJsonThrottled)
pub struct PartialJson { buf: String, last_parsed_len: usize }
impl PartialJson {
    pub fn push(&mut self, delta: &str);
    /// возвращает Some(args) только если приросло >= MIN_GROWTH (256) байт
    pub fn parse_throttled(&mut self) -> serde_json::Value;
    pub fn finalize(self) -> serde_json::Value; // безусловный финальный парс
}
fn relaxed_parse(s: &str) -> serde_json::Value; // JSON.parse -> repairing -> {}

// маппинг stop-reason per-family
pub fn map_stop_reason(family: ApiKind, wire: &str) -> StopReason;
```

## Definition of Done

- [ ] Golden-тесты нормализации: 3 фикстуры сырых SSE (Anthropic/OpenAI Responses/Gemini) дают побитово одинаковые последовательности `StreamEvent` (снапшот-дифф).
- [ ] `PartialJson`: обрезанный `{"path":"/a/b","cont` с дочинкой валидными дельтами восстанавливает полный объект; `parse_throttled` не перепарсивает чаще, чем каждые 256 байт прироста (тест по счётчику парсов).
- [ ] Тест на кумулятивные reasoning-дельты: повторяющиеся снапшоты MiniMax-стиля дедуплицируются, дубликаты не попадают в `ThinkingDelta`.
- [ ] Тест на markup-healing: tool-call, утекший в текст как XML, конвертируется в `toolcall_*` события и повышает `Stop`→`ToolUse`.
- [ ] Таблица маппинга stop-reason для трёх семейств покрыта unit-тестами (все ветки таблиц omp).
- [ ] Thinking: `clamp` на лествице модели + `effortMap` дают ожидаемый wire-токен для каждой пары (модель, запрошенный уровень) из тест-таблицы; `requires_effort` блокирует "off".
- [ ] Thinking-loop guard: детерминированный повтор вербатима в reasoning завершает стрим с ретраебельной ошибкой, а не зависанием (тест с таймаутом).
- [ ] Порядок доставки: события стрима доходят до консюмера в push-порядке без батчинга (стресс-тест с рандомной задержкой продюсера).

## Deep-dive

Написанные подсистемы: [transports.md](./transports.md), [auth-credentials.md](./auth-credentials.md).

План 2-го уровня: `docs/research/providers-streaming/thinking-dialects.md` — полная матрица dialect-режимов thinking/reasoning (zai/qwen/openrouter/openai/anthropic/google) с примерами wire-запросов; `docs/research/providers-streaming/replay-history.md` — replay thinking-блоков и подписей (Anthropic signature validation, encrypted reasoning, DeepSeek exact replay) при смене моделей.
