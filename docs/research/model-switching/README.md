# Свитчинг модели mid-session

Тема: смена модели посреди сессии — `/model` picker и mid-session switch (Hermes), ролевая система `modelRoles` (default/smol/slow/vision/plan/advisor), `cycleOrder`, thinking-суффиксы (`:low`, `:high`…), сохранение префикс-кеша при смене, поведение очереди сообщений при свитче.

## omp

1. **Роли и настройки.** `modelRoles` — record «имя роли → селектор модели» в `config.yml`/`settings-schema.ts`; встроенные роли: `default`, `smol`, `slow`, `vision`, `plan`, `designer`, `commit`, `tiny`, `task`, `advisor` (дока settings, раздел Models; источник: omp://settings.md). Роли `tiny` переопределяет модель фоновых задач (титулы сессий, память, классификация auto-thinking), иначе падает в `@smol`. Роли-алиасы `@smol`/`@slow` раскрываются через `modelRoles`, `*` = `@default`; значения могут быть `provider/modelId`. Есть per-role env/флаги только для `--model`/`--smol`/`--slow`/`--plan` (env: `PI_SMOL_MODEL`, `PI_SLOW_MODEL`, `PI_PLAN_MODEL`). `modelRoleStorage: global|project` решает, куда пишутся назначения ролей из model-selector (глобальный config или `<cwd>/.omp/config.yml`).
2. **`cycleOrder` и циклический свитчер.** `cycleOrder` — массив ролей, которые листает модельный свитчер; дефолт `["smol","default","slow"]` (omp://settings.md). Это array-setting: слой выше **заменяет** массив целиком, а не мёрджит (пример в доке settings явно предупреждает про `cycleOrder`).
3. **Thinking-суффиксы.** Значение роли и CLI-селектор `--model provider/modelId` могут нести суффикс `:thinkingLevel` из `off|minimal|low|medium|high|xhigh|max`; парсит `model-resolver.ts` (omp://models.md). Если роль ссылается на другую роль, цель наследуется, но явный суффикс на ссылающейся роли выигрывает для этого использования. Связанные настройки: `defaultThinkingLevel` (дефолт `high`), `thinkingBudgets` (token-бюджеты уровней), `modelTags` (кастомные роли/теги).
4. **Персистентность смены: entry `model_change`.** Смена модели пишется в append-only JSONL как entry `{"type":"model_change","model":"openai/gpt-4o","role":"default"}`; `role` опционален, отсутствие = `default`; `buildSessionContext` собирает из этих entries карту «роль → модель» по пути до leaf (omp://session.md). Временная смена (`setModelTemporary`, используется механизмом context promotion при overflow) пишет `model_change` как temporary и **не** переписывает сохранённое назначение роли (omp://models.md). При `/resume`-подобном свитче сессии восстанавливается «первая доступная записанная модель in role/default fallback order» (omp://session-switching-and-recent-listing.md).
5. **Префикс-кеш при смене.** Header сессии несёт `providerPromptCacheKey` — наследуемая cache-identity для форков; entry `credential_pin` (псевдонимный SHA-256 хеш аккаунта) ре-пинит OAuth-трафик к серверному аккаунту, сохраняя account-scoped переиспользование промпт-кеша (omp://session.md). При свитче на/с `openai-codex-responses` session закрывает websocket-state провайдера перед сменой (websocket handoff, omp://models.md). При смене сессии целиком — «close provider sessions for a different session» и снапшот/наследование provider-cache identity (omp://session-switching-and-recent-listing.md). omp **не** документирует принудительный сброс кеша при смене модели внутри сессии — кеш-ключи живут на стороне провайдера; смена модели с другим префиксом запроса естественно промахивается мимо кеша.
6. **Очередь при свитче.** `switchSession` снапшотит rollback-состояние (включая очереди и модель/thinking/tier) и затем **очищает message queues** (omp://session-switching-and-recent-listing.md). Для смены модели внутри сессии очереди не трогаются — смена пишется как `model_change` entry; очередь сообщений omp реализована на уровне сессии/TUI (см. секцию queues в доке сессий).

## Hermes

1. **`/model` — синтаксис и флаги.** `/model [model-name]` показывает/меняет модель; поддерживает `/model claude-sonnet-4`, `/model provider:model`, `/model custom:model`, `/model custom:name:model`, `/model custom` (автоопределение с endpoint) и пользовательские алиасы (`/model fav`). Флаги: `--global` (персистит в config.yaml), `--session` (только сессия), `--once` (только следующий turn), `--refresh` (перефетчить список моделей провайдера), `--provider <name>` (сменить backend). Обычный `/model <name>` — session-only, если не включён `model.persist_switch_by_default: true` (источник: https://hermes-agent.nousresearch.com/docs/reference/slash-commands). Смена возможна только между уже сконфигурированными провайдерами — нового провайдера добавляют через `hermes model` вне сессии.
2. **Интерактивный model picker.** `/model` без аргументов открывает provider→model picker; в списке моделей — type-to-fuzzy-filter (Backspace ужимает фильтр, Esc очищает/закрывает), выбор всегда резолвится в один конкретный танкер, фильтр «никогда не угадывает». В TUI это модальный picker, сгруппированный по провайдерам, с cost-хинтами (источники: https://hermes-agent.nousresearch.com/docs/reference/slash-commands, https://hermes-agent.nousresearch.com/docs/user-guide/tui).
3. **Стоимость смены: сброс префикс-кеша задокументирован явно.** «Switching models mid-conversation resets the prompt cache — the cache key includes the model, so your next turn re-reads the entire conversation at full input price instead of the ~75%-discounted cached rate» — для CLI и messaging-версии `/model` (https://hermes-agent.nousresearch.com/docs/reference/slash-commands). Никакой попытки сохранить кеш нет — это ожидаемое и незavoid-имое поведение.
4. **Thinking и алиасы.** `/reasoning [level]` меняет effort: `none|minimal|low|medium|high|xhigh|max|ultra`, `--global` персистит (там же). Кастомные алиасы: `model_aliases` в config.yaml (полная форма с `model`/`provider`/`base_url`, короткая `provider/model` через `hermes config set model.aliases.fav anthropic/claude-opus-4.6`), алиасы перекрывают встроенные короткие имена, case-insensitive (там же).
5. **Очередь сообщений.** Очередь ортогональна свитчу: `/queue <prompt>` ставит промпт в очередь следующего turn; `/steer <prompt>` инжектит после следующего tool call; `/busy [queue|steer|interrupt]` управляет, что делает Enter во время работы агента. Отдельного документированного поведения «что происходит с очередью при смене модели» нет — смена модели не описана как очищающая очередь.

## Vellum

Тема свитчинга моделей **не покрывает** собранный материал (`local://vellum-summary.md`): там память, SOUL-идентичность, проактивность, безопасность, каналы и OAuth — ни слова о выборе/смене моделей или ролях моделей.

Ближайшие аналоги, которые можно растянуть на тему:

- **«Один кодбейз, одна модель данных» + managed runtime**: Vellum абстрагирует модель за рантаймом платформы; смена модели — инфраструктурное решение деплоя, а не mid-session пользовательское действие.
- **Мультиботность с per-bot изоляцией**: у каждого бота своя SOUL+memory+skills; в titi это естественно расширяется до per-bot дефолтных моделей/ролей — но в Vellum-материале такая проекция на модельные роли отсутствует, это [INFERENCE], а не факт источника.
- **Асинхронные каналы**: проактивные уведомления «в правильный канал, не прерывая активный диалог» — ближайшая родственница behavior «очередь не рвётся при смене контекста».

## Решение (одно/комбо)

Комбо: ролевая система omp как ядро + UX-детали Hermes. Берём `modelRoles` (default/smol/slow/vision/plan/advisor + кастомные через `model_tags`) с thinking-суффиксами на значении роли — это даёт оптимальный one-place конфиг: и начальная модель, и подмодели для субтасков резолвятся из одного реестра без дублирования. `cycleOrder` оставляем как в omp (дефолт `["smol","default","slow"]`), а персистентность делаем на модели omp: append-only entry `ModelChange` в JSONL с опциональной ролью — replay при resume бесплатен и временные смены (fallback/promotion) не ломают сохранённые роли. Из Hermes берём только UX: fuzzy-filter picker по provider→model с cost-хинтами и честное правило «смена модели = полный re-read по полной цене» — вместо попыток «сохранить» кеш между разными моделями, которых не существует на уровне API. Очередь сообщений при смене модели НЕ очищаем (в отличие от omp-очистки при смене сессии): накопленные сообщения просто продолжат выполняться уже новой моделью — это самое дешёвое и предсказуемое поведение.

## Rust-маппинг

Крейты: `titi-core` (резолвер, роли, entries), `titi-providers` (кеш-ключи, transport-состояние), `titi-tui` (picker), `titi-cli` (`--model`/`--smol`/`--slow`/`--plan`), `titi-tools` — не задействован.

```rust
// titi-core/src/model/selector.rs
pub enum ThinkingLevel { Off, Minimal, Low, Medium, High, Xhigh, Max }

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ModelSelector {
    pub provider: Option<String>,     // None = bare id, резолв по реестру
    pub model: String,
    pub thinking: Option<ThinkingLevel>, // суффикс ":low"
}

impl ModelSelector {
    pub fn parse(s: &str) -> Result<Self, SelectorError>; // "anthropic/claude-opus-4-5:high"
}

// titi-core/src/model/roles.rs
pub struct ModelRoles {
    pub map: HashMap<String, ModelSelector>, // default, smol, slow, vision, plan, advisor, кастомные
    pub cycle_order: Vec<String>,            // дефолт ["smol","default","slow"]
}

pub trait ModelResolver {
    /// Раскрывает "@alias" через roles, голый id — по реестру с model_provider_order.
    fn resolve(&self, sel: &ModelSelector) -> Result<ModelRef, ResolveError>;
    fn resolve_role(&self, role: &str) -> Result<ModelRef, ResolveError>;
}

// titi-core/src/session/entries.rs — append-only, как omp model_change
pub enum SessionEntry {
    // ...
    ModelChange { id: EntryId, parent_id: EntryId, ts: Timestamp,
                  model: String,          // "provider/model-id"
                  role: Option<String> }, // None => default
}

// titi-core/src/session/replay.rs
pub struct ReplayState { pub models_by_role: HashMap<String, String>, /* из model_change */ }

// titi-providers/src/cache.rs
pub struct PromptCacheIdentity {
    pub provider_cache_key: Option<String>, // наследуемый key в header сессии
    pub account_pin: Option<[u8; 32]>,      // SHA-256 хеш аккаунта (credential_pin-аналог)
}

// titi-tui/src/picker/model.rs
pub struct ModelPicker { query: String, selected: usize }
impl ModelPicker {
    pub fn filter(&mut self, q: &str);           // fuzzy по "provider/model" + cost hint
    pub fn result(&self) -> Option<ModelSelector>; // всегда конкретная модель
}
```

Конфиг (serde_yaml): `model_roles: HashMap<String,String>`, `model_role_storage: Global|Project`, `cycle_order: Vec<String>`, `model_provider_order: Vec<String>`, `default_thinking_level: ThinkingLevel`. Внешние крейты: `serde`/`serde_yaml` (конфиг), `crossterm` (picker overlay в titi-tui), `nucleo-matcher` или `fuzzy-matcher` (fuzzy-фильтр picker'а), `tokio` (уже в workspace), `sha2` (account pin hash). Быстрый лексер суффиксов — обычный `rsplit_once(':')`, без зависимости.

## Definition of Done

- [ ] Тест `selector_parse`: `ModelSelector::parse` корректно разбирает `provider/model`, bare `model`, суффиксы `:off..:max` и отклоняет неизвестный уровень.
- [ ] Тест `roles_resolution`: роли резолвятся из `model_roles`, `@alias` раскрывается, отсутствующая роль падает в `default`, суффикс ссылающейся роли перекрывает цель.
- [ ] Тест `cycle_order`: свитчер листает роли по `cycle_order` (дефолт `["smol","default","slow"]`) и зацикливается.
- [ ] Тест `model_change_entry`: смена модели mid-session пишет append-only `ModelChange` в JSONL; `resume` восстанавливает карту «роль → модель» из replay; временная смена не переписывает сохранённую роль.
- [ ] Поведение очереди: при смене модели queued-сообщения сохраняются и следующий turn выполняется уже новой моделью (интеграционный тест на очереди).
- [ ] Тест `prompt_cache_identity`: `provider_cache_key` и `account_pin` наследуются из header при fork/resume; в usage при смене модели на turn не попадает cache_read (документированный полный re-read).
- [ ] TUI smoke: model picker открывается, fuzzy-фильтр сужает список, выбор резолвится в конкретную модель, Esc отменяет без смены.
- [ ] CLI: флаги `--model`, `--smol`, `--slow`, `--plan` переопределяют роли в runtime-слое и не персистятся.

## Deep-dive

План подсистем-доков (не писать без отдельной задачи):

- `docs/research/model-switching/roles-aliases.md` — `modelRoles`, `modelTags`, `modelProviderOrder`, `modelRoleStorage: project`, env/флаги per-role; сравнение с `model_aliases` Hermes.
- `docs/research/model-switching/thinking-suffixes.md` — уровни thinking, `thinkingBudgets`, `defaultThinkingLevel: auto` и классификатор; `/reasoning` Hermes.
- `docs/research/model-switching/prompt-cache.md` — `providerPromptCacheKey`, `credential_pin`, websocket handoff codex, честная арифметика cache-miss при смене (~75% скидка → full price, по данным Hermes).
- `docs/research/model-switching/queue-on-switch.md` — очередь сообщений: omp очищает очереди при `switchSession`, Hermes `/queue`/`/steer`/`/busy`; решение titi — очередь переживает смену модели.
- `docs/research/model-switching/model-picker-tui.md` — модальный picker Hermes (fuzzy, cost hints, группировка по провайдерам) поверх crossterm.
- `docs/research/model-switching/context-promotion.md` — overflow-фоллбек omp (`setModelTemporary`, `contextPromotionTarget`) как побочный путь mid-session свитча.
