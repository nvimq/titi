# Секреты и env

Как харнессы хранят и раздают креды: .env-файлы, разрешение API-ключей, auth store, отдельный процесс для кредов, обфускация секретов перед отправкой в LLM.

## omp

- **Слоёная dotenv-резолюция.** Большинство lookup'ов идут через `$env` из `@oh-my-pi/pi-utils` (`packages/utils/src/env.ts`). Порядок загрузки: (1) process env, (2) проектный `.env` из launch-директории, (3) agent `.env` (`~/.omp/agent/.env`), (4) config-root `.env` (`~/.omp/.env`), (5) `~/.env` — каждый следующий слой заполняет только пустые/несуществующие ключи. Имена должны быть shell-идентификаторами (`[A-Za-z_][A-Za-z0-9_]*`), небезопасные имена/значения отбрасываются; внутри распарсенных `.env` каждый `OMP_*` ключ зеркалится в `PI_*`-алиас, и зеркальное значение заменяет same-file `PI_*`. Источник: `omp://environment-variables`.
- **Разрешение API-ключей и OAuth.** Провайдерные креды разрешаются через `getEnvApiKey()` (`packages/ai/src/stream.ts`): по одному `*_API_KEY` на провайдера (десятки: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, …), при этом OAuth-токены старше API-ключей (`ANTHROPIC_OAUTH_TOKEN` > `ANTHROPIC_API_KEY`; в Foundry-режиме `ANTHROPIC_FOUNDRY_API_KEY` выше обоих). Хранилище `AuthStorage` — локальный SQLite (`agent.db`); порядок «Auth and API key resolution order» описан в `omp://models` (models.yml `apiKey` бьёт stored OAuth, но не явный `--api-key`). Источник: `omp://environment-variables`, `omp://auth-broker-gateway`.
- **Обфускация секретов (`secrets.yml`).** При `secrets.enabled: true` в `config.yml` секреты собираются на старте из: env-переменных с паттернами имени (`KEY`, `SECRET`, `TOKEN`, `PASSWORD`, `PASS`, `AUTH`, `CREDENTIAL`, `PRIVATE`, `OAUTH`, длина ≥ 8), файлов `~/.omp/agent/secrets.yml` (глобально) и `<cwd>/.omp/secrets.yml` (проект переопределяет глобал по `content`), плюс встроенный reversible-regex на GitHub-/GitLab-/OpenAI-образные токены. Два режима: `obfuscate` (детерминированный плейсхолдер `$$HASH(:hint)$$`, обратим) и `replace` (односторонний). База хеша — HMAC секрета под per-install ключом `~/.omp/agent/secret-placeholder.key` (никогда не уходит модели); плейсхолдеры восстанавливаются в tool-аргументах модели перед исполнением и реобфусцируются при provider replay. Источник: `omp://secrets`.
- **Auth broker / auth gateway — креды в отдельном сервисе.** `omp auth-broker serve` держит канонический SQLite-vault OAuth refresh-токенов и делает серверные refresh'ы (background refresher, refreshSkew 5 мин, интервал 60 с); клиенты получают редактированный snapshot, где все `refresh` поля заменены на `REMOTE_REFRESH_SENTINEL`, а при истечении access-токена дергают `POST /v1/credential/:id/refresh`. `RemoteAuthCredentialStore` запрещает локальные мутации. `omp auth-gateway serve` — forward-proxy (`/v1/chat/completions`, `/v1/messages`, `/v1/responses`, `/v1/pi/stream`): клиенты никогда не видят access-токен. Локальный кэш снапшота — AES-256-GCM, ключ `SHA-256(OMP_AUTH_BROKER_TOKEN)`, URL как authenticated data, файл `0600`, TTL 1 ч (`OMP_AUTH_BROKER_SNAPSHOT_TTL_MS`). Включается только переменной `OMP_AUTH_BROKER_URL` / `auth.broker.url`. Источник: `omp://auth-broker-gateway`.

## Hermes

- **Разделение файлов и hot reload.** Секреты (API keys, bot tokens, passwords) живут в `~/.hermes/.env`, OAuth-креды — в `~/.hermes/auth.json`, всё остальное — в `config.yaml`; приоритет: CLI args → `config.yaml` → `.env` → built-in defaults. Команда `hermes config set KEY VAL` сама маршрутизирует: API-ключи пишутся в `.env`, остальное в `config.yaml`. Слэш-команда `/reload` перезагружает `.env`-переменные в работающую сессию («picks up new API keys without restarting»); рядом есть `/reload-mcp` и `/reload-skills`; hot-swap `config.yaml` в gateway работает только для отдельных ключей (модель/компрессия), API-ключи требуют reload-путей. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/configuration, https://hermes-agent.nousresearch.com/docs/reference/slash-commands.
- **Внешние secret-источники как плагины.** Hermes умеет подтягивать ключи из Bitwarden Secrets Manager, 1Password (`op://`) и любого CLI-vault'а через command helper; источники композируются по детерминированной лестнице: `.env`/shell выигрывает по умолчанию (замена только при `override_existing: true`), mapped-источники (`env:`-map) старше bulk-инъекций, внутри одной формы — первый по списку `secrets.sources` выигрывает; конфликт даёт warning, а не молчаливую подмену. Есть `secrets.preserve_existing` (per-profile исключения) и profile-алиасинг (`TELEGRAM_BOT_TOKEN_MILLA` гидратирует `TELEGRAM_BOT_TOKEN`). Сторонний бэкенд — плагин, реализующий `SecretSource.fetch(cfg, home_path) -> FetchResult`; оркестратор сам владеет приоритетами, таймаутами и провенансом (UI показывает «(from Bitwarden)»). Источник: https://hermes-agent.nousresearch.com/docs/user-guide/secrets.
- **Гигиена секретов.** В `config.yaml` работает подстановка `${VAR_NAME}` и Cursor-style `${env:VAR}`; внешние бэкенды (`${file:}` / `${vault:}`) не резолвятся инлайн — они инжектят значения в окружение на старте через блок `secrets:`. В `execute_code` окружение скрабится (стриппятся `*_API_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, `*_CREDENTIAL`, `*_PASSWD`, `*_AUTH`); логи (`errors.log`, `gateway.log`) автоматически редактят секреты; `redact_secrets: true` в конфиге режет паттерны ключей в выводе инструментов; `docker_forward_env` прокидывает токены в контейнер из shell или `~/.hermes/.env`, чтобы секрет не попадал в конфиг. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/configuration.

## Vellum

Харнесс тему env-файлов и dotenv-резолюции напрямую не покрывает (материал пользователя не содержит деталей .env) — ближайший аналог: креды, доступные инструментам, идут через sandbox-окружение.

- **Креды в отдельном процессе и никогда в модели.** «Учётные данные живут в отдельном процессе и никогда не попадают в модель» — прямой аналог auth-broker omp, но жёстче: изоляция процессом, а не флагом конфига. Каждый вызов инструмента — в песочнице, по умолчанию deny. Источник: `local://vellum-summary.md` (раздел «Безопасность»).
- **Identity-гейт перед доступом к ресурсам.** Actor identity (`guardian`, `trusted`, `unknown`) резолвится один раз и принудительно соблюдается всюду: `unknown`-акторы не могут читать память, триггерить инструменты или эскалировать — тот же принцип «разрешение выдаётся только проверенной стороне» применим к выдаче кредов ботам сети. Источник: `local://vellum-summary.md`.

## Решение (одно/комбо)

Комбо из трёх харнессов. База — omp: слоёная dotenv-резолюция (process env → проектный `.env` → `~/.titi/agent/.env` → `~/.titi/.env` → `~/.env`, заполняем только unset) плюс `AuthStorage` на SQLite как единая точка выдачи API-ключей и OAuth-токенов, с каскадом «OAuth > stored key > env». Из Hermes берём два дешёвых и высокоценных механизма: `/reload` — hot-перечитывание `.env` в живой сессии (незаменимо для ротации ключей без перезапуска TUI) и систему pluggable secret-источников с оркестратором приоритетов (`.env`/shell по умолчанию выигрывает, провенанс каждого значения отслеживается). Для мультиботности — жёсткое правило Vellum: refresh-токены не должны лежать в процессах ботов, поэтому опциональный `titi-creds` (аналог `omp auth-broker`) держит канонический vault и раздаёт только access-токены по bearer-токену; при выключенном брокере всё работает локально через SQLite — нулевой оверхед для соло-пользователя, минимум доверенного кода, эффективный быстрый путь всегда инлайн.

## Rust-маппинг

- **Крейты workspace:**
  - `titi-core` — слоёный env, обфускация секретов, трейты резолюции кредов.
  - `titi-providers` — потребители `CredentialResolver` при построении запросов к провайдерам.
  - `titi-tools` — env-скрабинг для сабпроцессов инструментов (паттерны `*_API_KEY` и т.п.).
  - `titi-tui` — команда `/reload` и отображение провенанса ключей.
  - `titi-cli` — `titi config set` (маршрутизация ключей в `.env`), подкоманды `titi creds serve` / `titi creds login`.
  - Новый: `titi-secrets` — auth store (SQLite), creds-broker процесс, secret-источники-плагины.
- **Ключевые типы (эскиз):**

```rust
// titi-core/src/env.rs — слоёная резолюция в стиле omp $env
pub trait EnvLayer { fn load(&self) -> Result<BTreeMap<String, String>>; }
pub struct LayeredEnv { layers: Vec<Box<dyn EnvLayer>> } // process -> project -> agent -> config-root -> home
impl LayeredEnv {
    /// Каждая карта-кандидат используется, только если ключ ещё не установлен;
    /// идентификаторы валидируются regex ^[A-Za-z_][A-Za-z0-9_]*$
    pub fn get(&self, key: &str) -> Option<String>;
}

// titi-core/src/secrets.rs — обратимая обфускация в стиле omp
pub struct SecretObfuscator { hmac_key: Zeroizing<[u8; 32]>, entries: Vec<SecretEntry> }
impl SecretObfuscator {
    pub fn collect(env: &LayeredEnv, files: &[PathBuf]) -> Result<Self>; // env-паттерны + secrets.yml + builtin regex
    pub fn obfuscate(&self, text: &str) -> Cow<str>;                     // $$HASH(:hint)$$
    pub fn restore_tool_args(&self, args: &mut serde_json::Value);
}

// titi-secrets/src/store.rs — единая точка выдачи кредов
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    async fn api_key(&self, provider: &ProviderId) -> Option<ApiKey>;
    async fn access_token(&self, provider: &ProviderId) -> Result<AccessToken>;
    async fn upsert(&self, cred: Credential) -> Result<()>;   // запрещено Remote-варианту
}

// titi-secrets/src/sources.rs — плагин secret-источника в стиле Hermes
#[async_trait]
pub trait SecretSource: Send + Sync {
    async fn fetch(&self, profile: &Profile) -> Result<FetchResult>; // map env->value + origin label
}
pub struct SourceOrchestrator { sources: Vec<Box<dyn SecretSource>>, preserve_existing: Vec<String> }

// titi-secrets/src/broker.rs — opt-in отдельный процесс (Vellum/omp-broker)
pub struct CredsBroker { store: SqliteStore, refresher: BackgroundRefresher } // snapshot с sentinel, POST /credential/:id/refresh
```

- **Внешние крейты:** `dotenvy` (парсинг `.env`), `rusqlite` (+ `bundled`) — auth store и vault брокера, `tokio` — background refresher и брокерский HTTP-сервер (`axum` или `hyper`), `aes-gcm` + `sha2` — шифрованный кэш снапшота, `hmac`/`ring` — HMAC-плейсхолдеры, `secrecy` (`Zeroizing`, `SecretString`) — типы-обёртки, `regex` — builtin credential-паттерны, `serde_yaml`/`serde_json` — secrets.yml и snapshot, `reqwest` — refresh-запросы брокера, `zeroize` — очистка ключей из памяти.

## Definition of Done

- [ ] Юнит-тест слоёной резолюции: ключ, заданный в process env, не перезаписывается проектным `.env`; отсутствующий подтягивается из `~/.titi/agent/.env` (titi-core/env.rs).
- [ ] Тест парсера `.env`: имена вне `[A-Za-z_][A-Za-z0-9_]*` отбрасываются, `TITI_*` зеркалится в `TITI_*`-алиас внутри файла; битые строки дают warning, не панику.
- [ ] Round-trip тест `SecretObfuscator`: obfuscate → restore возвращает исходный текст; плейсхолдер детерминирован между перезапусками (HMAC per-install ключ), значения короче 8 символов не красятся в obfuscate-режиме.
- [ ] Тест каскада кредов в `CredentialStore`: OAuth-токен старше stored API-key, stored старше env-fallback; `models.yml`-ключ бьёт stored OAuth, но не явный `--api-key`.
- [ ] Интеграционный тест `/reload`: правка `.env` на диске видна работающей сессии после команды без перезапуска (titi-tui → titi-secrets).
- [ ] Тест `SourceOrchestrator`: shell/`.env` выигрывает без `override_existing`, конфликт источников даёт warning + провенанс у победителя.
- [ ] Тест брокера: snapshot не содержит refresh-токенов (sentinel), шифрованный кэш нечитаем при смене bearer-токена, refresh выполняется брокером по `POST /credential/:id/refresh`.
- [ ] Тест env-скрабинга в titi-tools: ни одна переменная из списка `*_API_KEY|*_TOKEN|*_SECRET|*_PASSWORD|*_CREDENTIAL|*_PASSWD|*_AUTH` не попадает в окружение сабпроцесса execute-инструмента.

## Deep-dive

План подсистем-доков (2-й уровень; пишутся при детализации реализации):

- `docs/research/secrets-env/env-resolution.md` — слоёная dotenv-резолюция: точный порядок, mirroring-алиасы, валидация имён, `/reload`.
- `docs/research/secrets-env/auth-store.md` — SQLite `CredentialStore`: схема, каскад приоритетов, ротация, провенанс.
- `docs/research/secrets-env/creds-broker.md` — opt-in процесс titi-creds: протокол snapshot/refresh, шифрованный кэш, bearer-токены, граница доверия.
- `docs/research/secrets-env/secret-obfuscation.md` — obfuscate/replace режимы, HMAC-плейсхолдеры, builtin credential-regex, восстановление в tool-аргументах.
