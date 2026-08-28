# MCP

Тема: клиент MCP — конфигурация серверов, транспорты (stdio / Streamable HTTP / legacy SSE), жизненный цикл подключений и tool-authoring на стороне сервера.

## omp

1. **Конфиг и валидация.** Конфиг живёт в `.omp/mcp.json` (проект) и `~/.omp/agent/mcp.json` (юзер, или `~/.omp/profiles/<name>/agent/mcp.json` при активном профиле); валидация в `validateServerConfig()` (`packages/coding-agent/src/mcp/config.ts`): stdio требует `command`, http/sse требуют `url`, одновременный `command`+`url` запрещён, неизвестный `type` отвергается; опущенный `type` = `stdio`. Плюс кросс-источниковые оверраиды юзер-конфига: `disabledServers` (высший приоритет, скрывает сервер из любого источника) и `enabledServers` (force-enable, но не побеждает denylist). Источник: `omp://mcp-config.md`.
2. **Транспорты.** Три реализации за трейтом `MCPTransport` (`request`, `notify`, `close`, `connected`, колбэки `onClose/onError/onNotification/onRequest`): `StdioTransport` — JSONL через stdin/stdout сабпроцесса; `HttpTransport` — Streamable HTTP (POST, опциональный SSE-ответ, заголовок `Mcp-Session-Id`, `DELETE` при закрытии); `LegacySseTransport` — ревизия 2024-11-05: GET-стрим, первый `endpoint` event даёт URL для POST'ов JSON-RPC. Request-id по умолчанию — монотонные числа с 1 (совместимость с экосистемой), опция `requestIdFormat: "string"` включает snowflake-строки. Источник: `omp://mcp-protocol-transports.md`.
3. **Lifecycle.** Подключение = `initialize` (протокол `2025-11-25`, реклама capability `roots`) → `notifications/initialized` → для Streamable HTTP запуск фонового SSE-листенера. Менеджер держит раздельные реестры (`#connections`, `#pendingConnections`, `#pendingToolLoads`, `#pendingReconnections`, `#serverConfigs`, `#reconnectHistory`); стартовый гейт 250 мс (`STARTUP_TIMEOUT_MS`) возвращает `DeferredMCPTool` из кэша для неуспевших серверов; реконнект с backoff 500/1000/2000/4000 мс; circuit breaker — >5 реконнектов за 30 с приостанавливает автореконнект; таймауты: `OMP_MCP_TIMEOUT_MS` → `config.timeout` → 30 с (0 = отключить). Источник: `omp://mcp-runtime-lifecycle.md`.
4. **Tool-мост и авторинг.** Инструменты регистрируются как `mcp__<server>_<tool>`: lower-case, не-`[a-z_]` → `_`, коллапс повторных `_`, обрезка избыточного префикса `<server>_`; имена >64 символов получают префикс + 8 base-36-символов `Bun.hash()`; коллизии санитизированных имён разрешаются детерминированно лексикографическим сравнением оригинального ключа `<server>\0<tool>`. Аргументы нормализуются перед `tools/call`: не-объект → `{}`, харнесс-поле `i` вырезается, опциональные пустые значения выбрасываются, `local://`-URL резолвятся в пути файлов. Источник: `omp://mcp-server-tool-authoring.md`.

## Hermes

1. **Конфиг-референс.** Конфиг — YAML в `~/.hermes/config.yaml` под `mcp_servers:`; ключи на сервер: `command/args/env` (stdio) или `url/headers` (HTTP), `transport: sse` переключает на SSE-транспорт, `timeout` (300 с) / `connect_timeout` (60 с), `protocol: auto|stateless|legacy` (fallback от legacy `initialize` к probe `server/discover` 2026-07-28), `trust: full|untrusted` — на untrusted-сервере каждый write-вызов (без `readOnlyHint: true`) требует одобрения юзера, нераспознанное значение = `untrusted` (fail-closed). Фильтрация инструментов на клиенте: `tools.include` (глобы fnmatch) / `tools.exclude`, include побеждает exclude; утилитные обёртки `list_resources/read_resource` и `list_prompts/get_prompt` регистрируются только если сессия реально рекламирует capability. Источник: https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference
2. **TTL/lifecycle-ключи.** `keepalive_interval` (180 с, пол 5 с) — liveness-пинги, чтобы сервер не GC-нул idle-сессию; stdio-рекле: `idle_timeout_seconds` и `max_lifetime_seconds` (могут жить под `lifecycle:`); `ssl_verify` (bool или путь к CA PEM) и `client_cert`/`client_key` для mTLS; `supports_parallel_tool_calls` разрешает конкурентные вызовы инструментов сервера. Источник: https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference
3. **Каталог и OAuth.** Кураторский каталог в `optional-mcps/<name>/manifest.yaml` репо hermes-agent (попадание = Nous-аппрув через PR; `hermes mcp install <name>`, чек-лист инструментов на инсталле, `tools.default_excluded` для гигантских сёрфейсов типа cloudflare ~3300 тулов); удалённые OAuth-серверы — `auth: oauth`, OAuth 2.1 + PKCE через MCP Python SDK, авто CIMD (Client ID Metadata Document) с fallback на Dynamic Client Registration; Figma-хак — авто подмена `oauth.client_name: "Claude Code"` для `mcp.figma.com`. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp/

## Vellum

Напрямую MCP-клиент в собранном материале (`local://vellum-summary.md`) **не покрывается** — там нет конфига серверов, транспортов или JSON-RPC-слоя. Ближайшие аналоги, полезные для MCP-клиента titi:

1. **OAuth-делегирование**: Vellum подключает Slack, Notion, Google, HubSpot, Linear, Discord, Twitter, Telegram, Twilio через OAuth и «без самописного token refresh» — тот же принцип, что и omp-креды, привязанные к URL сервера (`mcp_oauth:profile:<profile>:<url>`): refresh-материал живёт в хранилище, конфиг остаётся definition-only. Источник: `local://vellum-summary.md` (раздел OAuth) + `omp://mcp-config.md`.
2. **Секреты и песочница**: учётные данные в отдельном процессе и никогда не попадают в модель; каждый вызов инструмента — в песочнице; по умолчанию deny. Для MCP это маппится в разрешение `env`/`headers` через `!command`/env-var indirection (omp) и trust-тир с per-tool approval (Hermes `trust: untrusted`), а не в голые токены в конфиге. Источник: `local://vellum-summary.md` (раздел Безопасность).
3. **Мультиботность**: один ассистент/одна память на канал у Vellum подсказывает, что MCP-конфиг в titi должен быть per-bot (пер-бот изоляция кредов и тулов) — как omp-профили изолируют юзер-level MCP.

## Решение (одно/комбо)

Комбо «omp-архитектура как скелет + Hermes-селекция и trust как надстройка». Скелет берём у omp: единый `MCPServerConfig` с тремя транспортами за одним трейтом, менеджер с раздельными реестрами connect/pending/reconnect, startup-гейт с deferred-инструментами из кэша и manager-level reconnect с backoff — это проверенная, отказоустойчивая модель с быстрой стартовой развёрткой. От Hermes добавляем клиентскую фильтрацию `tools.include/exclude` (дешёвая экономия контекста — не тащить в модель 3300 тулов cloudflare), `trust: untrusted` с per-call approval для write-инструментов, и keepalive/recycle-ключи, которых у omp нет. От Vellum — принцип «креды никогда не в модели»: только `!command`/env-indirection и OAuth-креды, привязанные к URL+боту. Серверный tool-authoring в titi не реализуем (мы клиент) — поэтому серверная сторона темы ограничивается поддержкой совместимости с тем, что авторы серверов генерируют (inputSchema, isError, прогресс-нотификации).

## Rust-маппинг

**Крейты:**
- `titi-core` — типы конфига, реестры менеджера, lifecycle (новый модуль `core/src/mcp/`).
- `titi-providers` — транспортные реализации и HTTP-аутентификация.
- `titi-tools` — tool-мост `MCP → AgentTool`, санитизация имён, нормализация аргументов.
- `titi-cli` / `titi-tui` — CLI-команды (`mcp add/list/test/reconnect`), TUI-статус серверов.
- Новый крейт `titi-mcp` (предлагается): JSON-RPC-слой, транспортный трейт, протокольные типы — чтобы `titi-core` не тянул HTTP-стек; зависит от `titi-providers` инверсно через трейт.

**Ключевые типы (эскизы):**
```rust
// titi-mcp/src/transport.rs
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&mut self) -> Result<(), McpError>;
    async fn request(&mut self, method: &str, params: Value, opts: ReqOpts) -> Result<Value, McpError>;
    async fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpError>;
    async fn close(&mut self) -> Result<(), McpError>;
    fn on_notification(&mut self, cb: NotificationSink);
    fn on_close(&mut self, cb: CloseSink);
}

// titi-core/src/mcp/config.rs
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerConfig {
    Stdio { command: String, args: Vec<String>, env: BTreeMap<String, String>, cwd: Option<PathBuf> },
    Http  { url: Url, headers: BTreeMap<String, String> },
    Sse   { url: Url, headers: BTreeMap<String, String> }, // legacy 2024-11-05
}
pub fn validate(cfg: &ServerConfig) -> Result<(), ConfigError>; // command+url несовместимы и т.д.

// titi-core/src/mcp/manager.rs
pub struct Manager { /* connections, pending, pending_tools, reconnects: DashMap<Name, _> */ }
impl Manager {
    pub async fn connect_servers(&self, cfgs: &[NamedConfig]) -> ConnectReport; // startup gate 250ms
    pub async fn reconnect(&self, name: &str) -> Result<(), McpError>;          // backoff 0.5/1/2/4s
    pub fn tools(&self) -> Vec<Arc<dyn AgentTool>>;                             // mcp__server_tool
    pub async fn disconnect_all(&self);
}

// titi-tools/src/mcp_bridge.rs
pub fn sanitize_tool_name(server: &str, tool: &str) -> String;      // mcp__<server>_<tool>, >64 → hash
pub fn normalize_args(schema: &Schema, args: &mut Value) -> Result<(), McpError>; // пустые optional выкинуть
```

**Внешние крейты:** `tokio` (process spawn, время, sync), `serde`/`serde_json` (JSON-RPC + конфиг), `async-trait`, `reqwest` (+`stream`) для Streamable HTTP, `eventsource-stream` или самописный SSE-парсер для per-request SSE и legacy GET-стрима, `rustls` + `tokio-rusts` (вместо native-tls — HERMETIC-сборка), `dashmap` для реестров, `snowflake`-генератор или `nanoid` для строковых request-id. Серверный SDK (`rmcp`) не нужен — titi только клиент.

## Definition of Done

- [ ] `titi-mcp::Transport` имеет три реализации: stdio (JSONL, per-line parse, malformed line = skip), streamable-http (POST + `Mcp-Session-Id` + опц. SSE-ответ), legacy-sse (GET + `endpoint` event); юнит-тесты на correlation по id для каждой.
- [ ] `validate()` отвергает: stdio без `command`, http/sse без `url`, `command`+`url` одновременно, неизвестный `type` — тест на каждый кейс.
- [ ] Startup-гейт: менеджер возвращает управление ≤250 мс; неуспевшие серверы с кэш-хитом дают deferred-инструменты, без кэша — регистрируются фоном через `on_tools_changed` (интеграционный тест с медленным mock-сервером).
- [ ] Reconnect: transport-close триггерит backoff 0.5/1/2/4 c; >5 реконнектов за 30 с ставит сервер на паузу; тест по временной шкале на `tokio::time::pause`.
- [ ] `sanitize_tool_name("My-Server", "my-server.echo")` = `mcp__my_server_echo` без коллизии с `("my.server", "my-server echo")` — детерминированный победитель стабилен при разном порядке discovery (property-тест).
- [ ] Секреты: значение `env`/`headers`, начинающееся с `!`, исполняется как shell-команда с таймаутом 10 с; пустой вывод/ошибка → ключ опускается; сырые `Authorization: Bearer <hardcoded>` в дефолтных примерах отсутствуют (тест на resolver).
- [ ] Конфиг: `.titi/mcp.json` (проект) + `~/.titi/agent/mcp.json` (юзер) с `${VAR}`-expansion и `disabledServers`/`enabledServers`-оверраидами; тест precedence: юзер-denylist побеждает project-enable.
- [ ] Клиентская фильтрация `tools.include`/`tools.exclude` (fnmatch-глобы, include побеждает) применяется до регистрации инструментов в реестре — тест на list_tools mock-сервера.

## Deep-dive

План подсистем-доков 2-го уровня (не пишутся в этой фазе):

- `docs/research/mcp/json-rpc-correlation.md` — request-id форматы, pending-map, malformed-строки, server-to-client запросы (`ping`, `roots/list`), `-32601` fallback.
- `docs/research/mcp/transports-stdio-http-sse.md` — фрейминг трёх транспортов, SSE-режимы (per-request vs фоновый листенер), `Mcp-Session-Id`, backpressure.
- `docs/research/mcp/server-tool-authoring-compat.md` — что titi должен терпеть от серверов: inputSchema-нормализация, `isError`, прогресс/`list_changed`-нотификации, большие сёрфейсы и default_excluded-паттерн.
- `docs/research/mcp/auth-secrets.md` — OAuth credential binding по URL, `!command`/env resolution, trust-тиры и approval-поверхность.
