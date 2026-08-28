# Экстенсибилити и маркетплейс

## omp

**Единая точка расширения — ExtensionAPI (extensions — надмножество hooks).**
Расширение — TS/JS-модуль с default-фабрикой `export default function (pi: ExtensionAPI)`; фабрика регистрирует обработчики событий (`pi.on`), LLM-инструменты (`pi.registerTool`), slash-команды (`pi.registerCommand`), шорткаты, рендереры сообщений и провайдеров (`pi.registerProvider`). Ключевые контракты: вызов runtime-действий (`pi.sendMessage`, `setActiveTools`) во время загрузки бросает `ExtensionRuntimeNotInitializedError` — сначала регистрация, действия только из событий/команд/инструментов; ошибки в `tool_call`-хендлере фейл-клозед (блокируют инструмент); фоновую работу надо вести через `ctx.setInterval/ctx.setTimeout` — сырые таймеры без изоляции роняют всю сессию. (omp://extensions.md, omp://skills/authoring-extensions.md)

**Дисковери и порядок загрузки расширений.** `discoverAndLoadExtensions()` строит один упорядоченный список: (1) нативный автодисковери `<cwd>/.omp/extensions` и `<agent-dir>/extensions/` (только `.ts`/`.js`, gitignore-aware), (2) JS/TS hook-фабрики из hook-капабилити, (3) расширения установленных плагинов из `package.json#omp.extensions` (принимает ещё `.mjs`/`.cjs`), (4) явные пути из CLI `--extension/-e` и настройки `extensions:`. Дедупликация по абсолютному пути — «первый выигрывает»; провал одного пути не роняет остальные. Отключение — `--no-extensions`, выборочно — `disabledExtensions: ["extension-module:<derivedName>"]` (имя = стем файла). (omp://extension-loading.md)

**Hooks — legacy-подсистема с теми же событиями.** `HookAPI` покрывает `tool_call` (можно `{block, reason}` — первый `block: true` шорт-катит; любой return `{input}` заменяет аргументы), `tool_result` (патч `content/details`; хендлеры в порядке регистрации, для HookAPI каждый видит оригинальный результат, последний override выигрывает), `context` (переписывание списка сообщений перед каждым LLM-вызовом, конвейерная передача). Extension-only события (`tool_execution_*`, `input`, `user_bash`, `user_python`) в HookAPI недоступны; для нового кода рекомендован ExtensionAPI. (omp://skills/authoring-hooks.md, omp://extensions.md)

**Skills — файловые пакеты с progressive disclosure.** Скилл = `<skills-root>/<name>/SKILL.md`; фронматтер поддерживает `name`, `description`, `globs`, `alwaysApply`, `hide`, `disable-model-invocation` (+ сохранение неизвестных ключей). Провайдеры с приоритетами: `native` (100, `.omp` user/project) > `omp-plugins` (90) > `claude` (80) > `claude-plugins`/`agents`/`codex` (70) > `opencode` (55) > `github` (30) > `omp-managed` (5, автоскиллы `<agent-dir>/managed-skills`, всегда уступают авторским); дедуп по имени, первый с высшим приоритетом выигрывает. В системный промпт уходит только `name`+`description`; полный контент читается по `skill://<name>` (и `skill://<name>/<rel>`) с защитой от `..`/escape за `baseDir`; `hide: true` прячет из промпта, но скилл остаётся доступен по URL и `/skill:<name>`. (omp://skills.md)

**Автоскиллы: инструмент manage_skill.** `manage_skill` (approval `"write"`, `loadMode: "essential"`, требует `autolearn.enabled = true`) делает `create/update/delete` управляемых скиллов в `<agent-dir>/managed-skills/<name>/SKILL.md`: имя `[a-z0-9][a-z0-9-]{0,63}`, лимит файла 64 000 байт, антисимлинк-проверки против выхода за корень, `create` не перезаписывает, а при совпадении имени с авторским скиллом возвращает `isError` c `details.shadowed = true` — авторские всегда приоритетнее. (omp://tools/manage_skill.md)

**Rulebook matching pipeline.** Все форматы правил (Cursor `.mdc`, Windsurf, Cline `.clinerules`, GitHub `*.instructions.md`, `.agent[s]/rules`, `.omp/rules`) нормализуются в единый `Rule { name, path, content, globs, alwaysApply, description, condition, astCondition, scope, interruptMode }`; дедуп и приоритет только по `name`. После дисковери `bucketRules(...)` раскладывает правила: правило с непустым `condition` (regex) или `astCondition` (ast-grep паттерны) регистрируется в `TtsrManager` и становится TTSR-only; `alwaysApply: true` — полный текст инжектится в системный промпт; иначе при наличии `description` — правило-книга, листинг `- name (globs): description` в `<domain-rules>`, контент по `rule://<name>` (точное совпадение имени; TTSR-правила тоже адресуемы). `globs` для rulebook — только подсказка в промпте, код не форсирует применимость. (omp://rulebook-matching-pipeline.md)

**Маркетплейс, совместимый с Claude Code.** Маркетплейс = git-репо/локальный каталог/URL с `.omp-plugin/marketplace.json` (или фоллбек `.claude-plugin/marketplace.json`); каталог содержит `name`, `owner.name`, `plugins[]` с `source` (относительный путь, git URL, GitHub-шортхенд, git-subdir монорепо; npm парсится, но установка отвергается). Установка `name@marketplace` на скоуп user (`~/.omp/plugins/installed_plugins.json`) или project (`<repo>/.omp/plugins/...`); включённый project-инсталл затеняет user; кэш в `plugins/cache/plugins/<marketplace>___<plugin>___<version>/`, сам плагин симлинкается в `<scope>/plugins/node_modules/` и регистрируется в `omp-plugins.lock.json` — тот же runtime-путь, что и npm/link-плагины. Имена: `[a-z0-9][a-z0-9.-]{0,62}`, id ≤128 символов. (omp://marketplace.md)

**Плагинный менеджер и инсталлятор.** Активные пути: `PluginManager` (npm/git/link: `bun install` в `~/.omp/plugins`, lockfile-стейт `{version, enabledFeatures, enabled}`, откат снапшотом `package.json`+`bun.lock`+дерева пакетов при любом провале пост-инсталляции, включая валидацию расширений на throwaway-регистрационной поверхности) и `MarketplaceManager` (каталоги, кэш, скоуп-реестры). Грамматика спека: `pkg`, `pkg[*]`, `pkg[]`, `pkg[a,b]`, `@scope/pkg@1.2.3[feat]`; git-спеки `github:user/repo[#ref]`, `gitlab:`, `codeberg:`, `sourcehut:` и полные URL; защита от command-injection — regex имени пакета + denylist шелл-метасимволов. Discovery-гейт: пакеты без `omp`/`pi`-манифеста пропускаются; project `plugin-overrides.json` перекрывает lockfile; `omp plugin doctor --fix` чинит дрейф. (omp://plugin-manager-installer-plumbing.md)

**Custom tools — модель-вызываемые функции.** Модуль экспортирует фабрику `(pi: CustomToolAPI) => CustomTool | CustomTool[]` с `execute(toolCallId, params, onUpdate, ctx, signal)` и схемой параметров (`pi.zod`/`pi.arktype`/`pi.typebox`); два пути интеграции — SDK `options.customTools` и дисковери `discoverAndLoadCustomTools` (нативные `tools/`, Claude/Codex конфиги, манифесты плагинов, явные пути; конфликты имён с built-ins отвергаются; `.md`/`.json` — метаданные, не исполняемые модули). Инструменты дефолтно `loadMode: "discoverable"`, кроме канонических essential-имён (`read`, `write`, `bash`, `edit`, `glob`, `computer`, `eval`, `task`, `hub`, `learn`, `manage_skill`). Оборачивание built-in даёт `ctx.invokeTool` для делегирования нативному одноимённому инструменту без повторного гейтинга. (omp://custom-tools.md)

## Hermes

**Skills как процедурная память агента (SKILL.md-стандарт).** Скиллы живут в `~/.hermes/skills/` (плюс `external_dirs` и проектные `.hermes/skills/` / `.agents/skills/` с приоритетом project → local → external и явным `hermes skills trust` для репо). Агент сам создаёт и правит скиллы через `skill_manage` (`create/patch/edit/delete/write_file/remove_file`; `patch` предпочтительнее — токен-эффективнее) и `/learn` — превращает URL, директорию, PDF или описание процедуры в скилл по домашнему стандарту (≤60-символьное описание, фиксированный порядок секций When to Use/Procedure/Pitfalls/Verification); большие источники становятся «knowledge-base skill» с индексом и дистиллятами per-topic в `references/`. Опциональный гейт `skills.write_approval: true` стейджит все записи скиллов в `~/.hermes/pending/skills/` с approve/deny-флоу `/skills diff|approve|reject`. (URL: https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)

**Фронматтер SKILL.md с conditional activation и секретами.** Поля: `name`, `description`, `version`, `platforms: [macos, linux]` (авто-сокрытие на несовместимых ОС), `metadata.hermes.tags/category`, условная активация `fallback_for_toolsets`/`requires_toolsets`/`fallback_for_tools`/`requires_tools` (например `duckduckgo-search` виден только когда недоступен web-toolset), декларация секретов `required_environment_variables` (запрос безопасно при загрузке, проброс в сандбоксы) и несекретного `metadata.hermes.config` (ключи в `skills.config` config.yaml, инжект значений в контекст). Progressive disclosure трёхуровневый: `skills_list()` (~3k токенов метаданных) → `skill_view(name)` → `skill_view(name, path)`. (URL: https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)

**Skills Hub — «маркетплейс скиллов» с исходниками-плагинами.** `hermes skills browse/search/inspect/install/update/uninstall` работает с 8 типами источников: `official` (bundled-каталог), `skills-sh` (каталог Vercel), `well-known` (`/.well-known/skills/index.json` на сайтах), прямой URL к `SKILL.md` (+ точно референсимые файлы из `references/templates/scripts/assets/examples`), GitHub-репо и кастомные «taps» (`hermes skills tap add owner/repo` — просто репо с `skills/*/SKILL.md`, опционально `skills.sh.json` с категориями), плюс community-интеграции `clawhub`, `lobehub`, `browse-sh`. (URL: https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)

**Безопасность установки: сканер, карантин, trust-уровни.** Каждый hub-инсталл проходит встроенный сканер (exfiltration, prompt injection, деструктивные команды); вердикт `dangerous` не обходится даже `--force` (caution/warn — можно). Trust-уровни: `builtin`/`official`/`trusted` (openai|anthropics|huggingface|NVIDIA `skills`) /`community`. Проектные скиллы сканируются при каждом изменении контента (кэш по хэшу), `dangerous` уходит в карантин — не индексируется и не загружается; дополнительно опциональный advisory-скан NVIDIA SkillEvaluator Tier 1 (PII, unicode-smuggling, SkillSpector), записи аудита в `skills/.hub/lock.json` + `audit.log`. (URL: https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)

**Слэш-команды и бандлы.** Каждый скилл — это `/skill-name [args]`; можно стековать до 5 ведущих `/skill`-токенов в одном сообщении; skill-бандлы — YAML в `~/.hermes/skill-bundles/<slug>.yaml` (`name/description/skills/instruction`), группирующие скиллы под одной командой, бандл затеняет одноимённый скилл, отсутствующие скиллы скипаются с нотисом; всё это не мутирует системный промпт (не бьёт кэш промпта). (URL: https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)

## Vellum

Тему «экстенсибилити/маркетплейс» Vellum-материал напрямую не покрывает: в нём нет расширяемых плагинов, каталогов или инсталлятора. Ближайшие аналоги и релевантные принципы (local://vellum-summary.md):

- **Обучаемость вместо установки пакетов.** Расширяемость Vellum — это агент-авторский контент: «обучаемость (память + опыт + скиллы)» (local://vellum-summary.md#4) и per-bot изоляция «SOUL+memory+skills» с обменом опытом между ботами (local://vellum-summary.md#26). То есть скиллы — часть личности/состояния бота, а не внешние артефакты из каталога; аналог omp `manage_skill`/Hermes `skill_manage`, без marketplace.
- **Безопасность как инвариант рантайма, а не сканер пакетов.** Actor identity (guardian/trusted/unknown) резолвится один раз и соблюдается всюду: unknown не может читать память, триггерить инструменты или эскалировать; учётные данные — в отдельном процессе, никогда в модели; каждый вызов инструмента — в песочнице; по умолчанию deny (local://vellum-summary.md#13). Для titi это прямая проекция на границу доверия плагинов: код плагина — это unknown-актор, а не доверенный.
- **OAuth-интеграции как «готовые custom tools».** Slack, Notion, Google, HubSpot, Linear и др. подключаются OAuth'ом без самописного token refresh (local://vellum-summary.md#19) — ближайший аналог рынка инструментов: не код-плагины, а декларативные коннекторы, предоставляемые платформой.

## Решение (одно/комбо)

Комбо: остов — слоёная модель omp, «рынок» — гибрид omp-маркетплейса и Hermes Skills Hub, с Vellum-принципом «плагин = недоверенный код». Пассивный слой (скиллы и rulebook-правила) делаем файловым с фронматтером и приоритетными провайдерами: в системный промпт — только `name`+`description`, контент — лениво по внутренним URL `skill://`/`rule://`; это дешевле всего по токенам и повторяет Hermes progressive disclosure. Исполняемый слой — единый Extension API (события + инструменты + команды + хуки в одном), потому что hooks как отдельная legacy-система в omp признаны надмножаемым подмножеством и дублируют код; custom tools — частный случай расширения с одним интерфейсом `execute`. Маркетплейс делаем двумя каналами: каталог плагинов в формате `.omp-plugin/marketplace.json` (совместимость с Claude-экосистемой, git/локальные/URL-источники, скоупы user/project, lockfile + симлинк в node_modules) и «скилл-хаб» для чисто файловых скиллов (GitHub-taps + well-known), где установка = скачивание файлов + обязательный сканер с карантином `dangerous` по модели Hermes. Автоскиллы агента (`manage_skill`) включаем за флагом с write-approval стейджингом, чтобы self-improvement не записывал контент мимо человека.

## Rust-маппинг

**Крейты workspace:** существующие `titi-core` (сессия, event bus, реестр сессий), `titi-providers` (LLM-провайдеры), `titi-tools` (built-in инструменты и реестр), `titi-tui`, `titi-cli`; предлагаемые новые: `titi-ext` (Extension API, лоадер, хук-конвейер), `titi-skills` (дисковери скиллов и правил, фронматтер, rulebook matching, внутренние URL), `titi-marketplace` (каталоги, инсталлятор, lockfile, сканер), `titi-sandbox` (изоляция плагинного кода — см. security-тему).

```rust
// titi-ext: единый Extension API (extensions = hooks = custom tools)
pub struct ExtensionContext<'a> { pub cwd: PathBuf, pub ui: &'a dyn UiContext, /* ... */ }

pub trait Extension: Send + Sync {
    fn name(&self) -> &str;
    fn register(&self, api: &mut ExtensionApi);          // фаза загрузки: только регистрация
}

pub enum Event {
    SessionStart(SessionSnapshot), SessionShutdown,
    ToolCall(ToolCallEvent), ToolResult(ToolResultEvent),   // ToolCall -> Interception { block, reason, input }
    ContextRewrite(ContextEvent),                           // -> Option<Vec<Message>>
    TurnStart, TurnEnd, /* ... */
}

pub struct ToolDefinition {
    pub name: String, pub description: String,
    pub parameters: schemars::Schema,
    pub load_mode: LoadMode, pub approval: Approval,
    pub execute: fn(ToolCallId, serde_json::Value, &ToolCtx) -> BoxFuture<AgentToolResult>,
}
// Реестр инструментов: built-ins + extension-registered; конфликты имён — reject.
// Interception: Vec<Box<dyn Fn(Event) -> BoxFuture<Interception>>> в порядке регистрации,
// fail-closed для ToolCall (ошибка хендлера = block).

// titi-skills: дисковери + matching
pub struct Skill { pub name: String, pub description: String, pub base_dir: PathBuf,
                   pub hide: bool, pub always_apply: bool, pub globs: Vec<String> }

pub struct Rule { pub name: String, pub path: PathBuf, pub content: String,
                  pub globs: Vec<String>, pub always_apply: bool, pub description: Option<String>,
                  pub condition: Option<Regex>, pub ast_condition: Vec<String>,
                  pub scope: Vec<ScopeToken>, pub source: SourceMeta }

pub enum Bucket { Ttsr(TtsrManager), AlwaysApply, Rulebook }   // приоритет: TTSR > always > rulebook

pub trait CapabilityProvider { fn priority(&self) -> u32; fn skills(&self) -> Vec<Skill>;
                               fn rules(&self) -> Vec<Rule>; }  // native(100), plugins(90), ...
// Дедуп по name, first-wins; rules/skill адресация:
pub fn resolve_skill_url(url: &str, root: &Path) -> Result<PathBuf, UrlError>; // skill://name/rel
```

**Внешние крейты:** `serde`/`serde_json`/`serde_yaml` (фронматтер и каталоги), `schemars` (JSON Schema для параметров инструментов), `regex` (condition; флаги `(?i)/(?m)/(?s)` из фронматтера — родная поддержка), `tree-sitter` + `tree-sitter-{rust,go,ts,...}` (astCondition на edit/write-потоках), `tokio` (async runtime, `CancellationToken` вместо AbortSignal), `glob`/`ignore` (gitignore-aware сканирование), `dirs` (`~/.titi`, XDG), `git2` или shell `git` для маркетплейс-источников, `reqwest` (URL-каталоги и hub), `sha2` (кэш сканов по хэшу контента, lock-записи), `semver` (версии плагинов), `fs-err`+`tempfile` (атомарные записи SKILL.md/lockfile), `rusqlite` (если индекс скиллов дорастёт до FTS, как у Hermes).

## Definition of Done

- [ ] `titi-skills` дисковерит `<root>/skills/<name>/SKILL.md` (одноуровнево) и `.omp/rules/*.md`, парсит фронматтер (`name`, `description`, `globs`, `always_apply`, `hide`, `condition`, `ast_condition`, `scope`) с фоллбеком на построчный парсинг при невалидном YAML; юнит-тесты на все 8 комбинаций бакетов (TTSR / always-apply / rulebook / никуда).
- [ ] Дедуп скиллов и правил по имени с приоритетом провайдера (native 100 → plugins 90 → …) покрыт тестом: одноимённый скилл более приоритетного провайдера затеняет остальные, `manage_skill`-скилл затеняется любым авторским.
- [ ] `skill://<name>` и `skill://<name>/<rel>` резолвятся с отказом на `..`, абсолютных путях и выходе за `base_dir` (тест на каждый отказ).
- [ ] `titi-ext` исполняет фабрику расширения: регистрация инструментов/команд/хендлеров на фазе загрузки, `Event::ToolCall`-хендлер с `block: true` останавливает инструмент, ошибка хендлера блокирует (fail-closed), `Event::ContextRewrite` конвейерно переписывает сообщения (интеграционный тест на каждый контракт).
- [ ] `titi-marketplace` устанавливает плагин из локального каталога и git-источника: валидация каталога (имя/owner/plugins/source), скоупы user/project, затенение project над user, запись lockfile, откат артефактов при провале пост-инсталляции (тест с ломающимся манифестом).
- [ ] `titi-marketplace` (скилл-хаб) ставит скилл с GitHub/URL: скачивание SKILL.md + референсимых файлов из allowlisted-папок, сканер паттернов exfiltration/injection, вердикт `dangerous` блокирует установку даже с force (тест на фикстуре-плейбуке).
- [ ] Кастомный инструмент регистрируется с JSON-Schema параметрами, конфликты имён с built-ins и повторные имена отклоняются, `onUpdate` стримит частичные результаты, `CancellationToken` пробрасывается в execute (интеграционный тест).
- [ ] `disabledExtensions`-аналог (id-формат `<capability>:<name>`) и флаг `--no-extensions` фильтруют загрузку по всем капабилити (тест: отключённый модуль не грузится, явный `-e` путь при `--no-extensions` грузится).

## Deep-dive

Подсистемы 2-го уровня (план; писать по мере проработки каждой):

- `docs/research/extensibility-marketplace/extension-api.md` — полный каталог событий ExtensionAPI, семантика deliverAs/steer, provider registration.
- `docs/research/extensibility-marketplace/extension-loading.md` — резолвер путей/манифестов, дедуп, failure isolation.
- `docs/research/extensibility-marketplace/skills-frontmatter.md` — схемы фронматтера omp/Hermes/agentskills.io, managed skills, write-approval стейджинг.
- `docs/research/extensibility-marketplace/rulebook-matching.md` — Rule-нормализация форматов, TTSR-триггеры (regex + ast-grep), scope-токены.
- `docs/research/extensibility-marketplace/marketplace-installer.md` — marketplace.json, спек-грамматика, lockfile, откат, доверие.
- `docs/research/extensibility-marketplace/custom-tools.md` — CustomTool API, делегирование built-in, loadMode/approval.
- `docs/research/extensibility-marketplace/security-trust.md` — сканер скиллов, карантин, trust-уровни, изоляция плагинного кода (стык с темой безопасности).
