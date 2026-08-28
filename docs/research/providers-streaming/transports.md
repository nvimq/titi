# Транспорты провайдеров: Anthropic / OpenAI / Gemini и fallback-цепочки

Как харнессы говорят по сети с модельными бэкендами: HTTP+SSE против WebSocket против REST-чанков, endpoint-семейства, gateway overlay, правила retry/fallback и что значит «подключить нового провайдера».

## omp

1. **OpenAI: собственный HTTP+SSE транспорт вместо SDK.** `postOpenAIStream()` (`packages/ai/src/utils/openai-http.ts`) декодирует SSE-фреймы через `readSseJson()` и заменил официальный `openai` SDK; Anthropic использует свой `AnthropicMessagesClient` (`packages/ai/src/providers/anthropic-client.ts`) с retry/timeout-логикой; Gemini — REST `POST .../models/{model}:streamGenerateContent?alt=sse` (omp://provider-streaming-internals.md, omp://provider-quirks.md).
2. **Codex — единственный WS-транспорт с SSE fallback**: WebSocket (`OpenAI-Beta: responses_websockets=2026-02-06`) с heartbeat ping/pong 10s/60s, повторным использованием сокета (idle reuse cap 30s), queue capacity 4096; мгновенный откат на SSE при connection/handshake-ошибках (`CODEX_WEBSOCKET_FATAL_PATTERNS`), fallback разрешён только до эмиссии replay-unsafe контента; `previous_response_id` чейнится только по WS, SSE никогда не чейнится (omp://provider-quirks.md, omp://provider-endpoint-constraints.md).
3. **Watchdogs и grace-окна**: `iterateWithIdleTimeout` следит за первым событием и меж-событийными паузами (`X-Stainless-Timeout` вниз по потоку); Chat Completions держит 2500ms post-finish grace (`OPENAI_COMPLETIONS_POST_FINISH_GRACE_MS`) для хвостовых usage-чанков; Codex WS — first-event 300s / idle 300s; у локальных бэкендов first-event timeout `0` (безлимитный prefill/model load) (omp://provider-quirks.md, omp://provider-compat-reference.md).
4. **Пустые завершения ретраятся**: `withEmptyCompletionRetry` — до 2 повторов с backoff 500ms, если `finish_reason: "stop"` без видимого контента и ≤1 output token; покрывает OpenAI Completions/Responses, Anthropic, Google, Ollama (omp://provider-quirks.md, omp://provider-streaming-internals.md).
5. **Gateway overlay над семейством**: Azure перестраивает URL Chat Completions в `/deployments/{dep}/chat/completions?api-version=...`, а Responses шлёт деплоймент полем `model` и авторизуется header'ом `api-key` (не Bearer); OpenRouter добавляет routing-суффиксы `:nitro`/`:floor`, routing через `provider`-объект и не шлёт дефолтный `max_tokens` (это hint маршрутизации); Copilot парсит токен из API-key и строит динамические заголовки (omp://provider-endpoint-constraints.md).

## Hermes

1. **Транспорт — явный атрибут провайдера**: `transport: chat_completions` (проставляется визардом `hermes model` → Custom Endpoint) с auto-detection по URL как fallback (путь `/anthropic` → `anthropic_messages`); MiniMax OAuth ходит на Anthropic Messages-совместимый `/anthropic` эндпоинт; xAI всегда через Responses API (`codex_responses`-транспорт) (https://hermes-agent.nousresearch.com/docs/integrations/providers, https://hermes-agent.nousresearch.com/docs/user-guide/configuration).
2. **Fallback-цепочка `fallback_providers:`**: список резервных провайдеров, опробуемых по порядку при rate limit / server error / auth failure; смена модели/провайдера происходит mid-session без потери диалога; активация one-shot per session; конфигурируется только через `config.yaml` или интерактивно `hermes fallback` (https://hermes-agent.nousresearch.com/docs/integrations/providers#fallback-providers).
3. **Per-provider и per-model таймауты**: `providers.<id>.request_timeout_seconds` применяется к primary-клиенту на всех транспортах, к fallback-цепочке и к пересборке после ротации кредов; `stale_timeout_seconds` — нестриминговый детектор зависшего вызова (легаси-дефолты `HERMES_API_TIMEOUT=1800s`, `HERMES_API_CALL_STALE_TIMEOUT=90s`, native Anthropic 900s) (https://hermes-agent.nousresearch.com/docs/user-guide/configuration#provider-timeouts).
4. **Копилот-восстановление на 401**: one-shot credential recovery — ре-резолв токена по цепочке `COPILOT_GITHUB_TOKEN → GH_TOKEN → GITHUB_TOKEN → gh auth token`, пересборка клиента, один повтор; Z.AI автопробирует несколько эндпоинтов (global/CN/coding) и кэширует рабочий (https://hermes-agent.nousresearch.com/docs/integrations/providers).

## Vellum

Харнес тему транспортов LLM-провайдеров не покрывает (материал о памяти/SOUL/безопасности/каналах). Ближайший аналог:

1. **«Без самописного token refresh» для OAuth-интеграций** (Slack, Notion, Google, HubSpot, Linear, Discord, Twitter, Telegram, Twilio) — принцип «транспортная/auth-логика централизована, не дублируется» переносится на модельные транспорты: один код-путь SSE/HTTP для всех провайдеров (local://vellum-summary.md, «OAuth»).
2. **Каждый вызов в песочнице, default-deny** — аналог принципа omp «нет raw provider passthrough»: все запросы идут через единый provider-logic слой, где применяются шейпинг и квирки (local://vellum-summary.md, «Безопасность»).

## Решение (одно/комбо)

Комбо: **единый SSE-движок + endpoint-family адаптеры + статическая fallback-цепочка**. Одна реализация HTTP+SSE-чтения (в omp это уже заменило SDK и окупилось) с тремя тонкими адаптерами событий — Anthropic Messages, OpenAI (Completions/Responses), Gemini `alt=sse`; WebSocket рассматривать только как оптимизацию для Codex-подобного провайдера позже, с тем же условием «fallback только до видимого контента». Fallback — модель Hermes: декларативный список провайдеров, переход только на границе хода (между turns), one-shot за сессию, потому что mid-stream переключение неминуемо ломает replay thinking-блоков. Watchdogs (first-event + idle) и grace-окно для хвостовых usage-чанков — копируем у omp как проверенные тайминги. Quirk-политики Azure/OpenRouter-класса оформлять как gateway overlay над endpoint-family, а не отдельными провайдерами.

## Rust-маппинг

Крейты: `titi-providers` (новый), `titi-core`. Внешние: `reqwest` (streaming `bytes_stream()`), `eventsource-stream` (SSE `EventSource`-стрим над `Stream<Item=Bytes>`), `tokio`, `futures`, `serde_json`.

```rust
pub enum ApiKind { OpenAiCompletions, OpenAiResponses, AnthropicMessages, GeminiGenerateContent }

// Один SSE-движок, три декодера
pub async fn open_sse(
    client: &reqwest::Client, url: &str, headers: HeaderMap, body: serde_json::Value,
) -> Result<impl Stream<Item = Result<SseFrame, TransportError>>, TransportError> {
    let resp = client.post(url).headers(headers).json(&body).send().await?;
    Ok(EventSource::new(resp.bytes_stream())) // eventsource-stream
}

#[async_trait]
pub trait Transport {
    fn api(&self) -> ApiKind;
    /// watchdog-конфиг копирует omp: first-event/idle таймауты, grace для usage-хвоста
    fn watchdog(&self) -> WatchdogConfig { WatchdogConfig::default() }
    async fn stream(&self, req: WireRequest, ctx: RequestCtx)
        -> Result<EventStream<StreamEvent>, TransportError>;
}

// Gateway overlay — узкий слой над Transport
pub struct GatewayOverlay { pub openrouter_routing: Option<RoutingSuffix>, pub azure_deployment: Option<SmolStr> }
pub fn apply_overlay(req: &mut WireRequest, overlay: &GatewayOverlay);

// Fallback-цепочка Hermes-стиля
pub struct FallbackChain { primary: ModelRef, backups: Vec<ModelRef>, activated: bool }
impl FallbackChain {
    /// вызывается ТОЛЬКО на границе хода; one-shot
    pub fn on_turn_error(&mut self, err: &TurnError) -> Option<ModelRef> {
        if !self.activated && err.is_retryable_chain_failure() {
            self.activated = true;
            self.backups.first().cloned()
        } else { None }
    }
}
```

## Definition of Done

- [ ] Golden-тесты: одна и та же история разговора сериализуется в wire-запросы Anthropic Messages / OpenAI Completions / OpenAI Responses / Gemini (4 фикстуры, дифф по снапшотам).
- [ ] SSE-декодер переживает malformed-фрейм: битый `data:` JSON превращается в `StreamEvent::Error`, а не панику, и не съедает последующие фреймы того же соединения до политики ретрая.
- [ ] Idle-watchdog тест: эмуляция потока без событий N секунд → `TransportError::Stalled`; grace-окно тест: usage-чанк, пришедший через 2s после `finish_reason`, засчитан.
- [ ] Empty-completion retry: `finish_reason: "stop"` без контента ретрится ≤2 раз с backoff, максимум — суммарно 3 запроса (тест по счётчику вызовов mock-сервера).
- [ ] Fallback: 429 на primary → переключение на резервного провайдера на следующем ходе, `activated` не даёт цепочке ходить по кругу (тест).
- [ ] Fallback не срабатывает mid-stream: ошибка после эмитированных дельт возвращает ошибку хода, а не переключение (тест).
- [ ] Gateway overlay: OpenRouter-запрос не содержит `max_tokens` при отсутствии явного кэпа; Azure-запрос несёт `api-key` header и деплоймент в `model` (unit-тесты на сериализацию).
- [ ] Новый OpenAI-совместимый провайдер = запись дескриптора + конфиг, нулевые правки в коде транспорта (компиляционный/снапшот-тест).

## Deep-dive

Подсистемы, вынесенные/спланированные:

- [streaming-events.md](./streaming-events.md) — нормализация событий стрима и квирки моделей (написан).
- [auth-credentials.md](./auth-credentials.md) — креды и их защита (написан).

План 2-го уровня: `docs/research/providers-streaming/usage-costs.md` — использование/cache-токены, service-tier мультипликаторы, OpenRouter reported cost; `docs/research/providers-streaming/adding-provider.md` — пошаговый чеклист подключения провайдера (catalog-entry + registry + тест-моки).
