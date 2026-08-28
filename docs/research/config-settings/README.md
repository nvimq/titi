# Конфиг и settings: слоёная резолюция, deep-merge, профили, карантин, schema-driven get/set/reset

## omp

1. **Пять слоёв с фиксированным приоритетом.** Эффективное значение настройки строится как `built-in defaults <- global <- project <- CLI overlays <- runtime overrides` (источник: `omp://settings.md`, раздел Precedence). Global — `~/.omp/agent/config.yml` (или существующий `config.yaml`, который читается и обновляется in-place); project — `<cwd>/.omp/settings.json`, затем поверх мерджится `<cwd>/.omp/config.yml`; CLI overlays — повторяемый `--config <file>` плюс env-список `PI_CONFIG_FILES` (`:` на Unix, `;` на Windows), загружаемый до `--config`; runtime — in-memory флаги (`--model`, `--approval-mode`, …), никогда не персистятся. Названия файлов: `omp://config-usage.md` §4 (settings resolution model) подтверждает тот же порядок и добавляет `PI_CONFIG_FILES` как отдельный шаг перед `--config`-оверлеями; внутри каждого списка оверлеев более поздний файл побеждает более ранний.
2. **Deep-merge: объекты сливаются, скаляры и массивы заменяются целиком.** `omp://settings.md` (Merge rules) — «Objects are deep-merged; Scalars and arrays are replaced wholesale by the higher-precedence layer», с отработанным примером: project `disabledProviders: [groq]` заменяет глобальный `[anthropic, openai]`, а `tools.approvalMode` из global сохраняется. Отдельная оговорка в `omp://config-usage.md` §8: settings-capability items не дедуплицируются и мерджатся deep-merge «в порядке возврата» провайдеров (от высшего приоритета к низшему), поэтому низкоприоритетный провайдер может перебить высокоприоритетный.
3. **Карантин битых файлов и strict-оверлеи.** Валидный YAML настроек обязан иметь mapping на верхнем уровне: битый persistent-файл (global или native project) при writable-старте переименовывается в уникальный сиблинг `.broken-<timestamp>-<pid>-<uuid>` под файловой блокировкой, после чего старт падает с исходной ошибкой и путём бэкапа; нечитаемый файл падает без переноса (`omp://settings.md`, Config file formats; `omp://config-usage.md` §4). Оверлеи `PI_CONFIG_FILES`/`--config` — strict: отсутствующий файл, невалидный YAML и не-mapping корень — hard error, карантину не подлежат.
4. **Schema-driven CLI.** `omp config list|get|set|reset|path` читают merged-effective настройки и пишут только в **global**-слой (`omp://settings.md`, Where writes go / Subcommands). Парсинг значения по типу из схемы: boolean принимает `true/false/yes/no/on/off/1/0` case-insensitive, enum — точное совпадение с перечнем ошибки, array/record — только JSON, `reset` персистит schema-**default** (не удаляет ключ). Схема — `SETTINGS_SCHEMA` (`omp://config-usage.md` §4, слои 5); creds в `list` маскируются (`********`), в `get` возвращаются открыто.
5. **Профили = relocated user base.** Именованный профиль (`--profile`, `OMP_PROFILE`, legacy `PI_PROFILE`) перемещает OMP user base: `~/.omp/agent/...` → `~/.omp/profiles/<name>/agent/...` для всего нативного (settings, скиллы, hooks, sessions, `agent.db`); профиль видит только свой конфиг. Исключение — keybindings: профиль мерджит keybindings дефолтного профиля под своими, по-биндингно перебивая (`omp://config-usage.md`, раздел Profiles, issue #4867).

## Hermes

1. **Одно-файловая модель с раздельным хранением секретов.** Всё в `~/.hermes/` (или `$HERMES_HOME`): `config.yaml` для всех несекретных настроек, `.env` для секретов, `auth.json` для OAuth. Precedence (highest first): CLI arguments → `~/.hermes/config.yaml` → `~/.hermes/.env` → built-in defaults (источник: https://hermes-agent.nousresearch.com/docs/user-guide/configuration, Configuration Precedence). Проектного слоя как такового нет — вместо него контекстные файлы (`.hermes.md`, `AGENTS.md`) и `terminal.cwd` в конфиге профиля.
2. **Schema-driven CLI с самовосстановлением.** `hermes config get|set|unset|check|migrate`: `set` автоматически маршрутизирует значение в правильный файл (API-ключи → `.env`, остальное → `config.yaml`); `unset` снимает user-значение; `check` ищет отсутствующие опции после апдейтов, `migrate` интерактивно дописывает их (источник: https://hermes-agent.nousresearch.com/docs/user-guide/configuration, Managing Configuration). Внутри значений поддержана подстановка `${VAR}` и Cursor-style `${env:VAR}`; неопределённая переменная остаётся verbatim с warning, bare `$VAR` не разворачивается.
3. **Managed scope — админский immutable-слой поверх.** `/etc/hermes/config.yaml` + `/etc/hermes/.env` (root-owned `0755`/`0644`, переносится `HERMES_MANAGED_DIR`) побеждают user-конфиг и даже shell-env **только по тем ключам, которые пиннят**; мердж leaf-level — пин `model.default` не замораживает `model.*` (источник: https://hermes-agent.nousresearch.com/docs/user-guide/managed-scope, Precedence). Попытка `hermes config set` пиннутого ключа отказывается с именованием источника. Битый managed-файл **не** блокирует старт: логируется громко и игнорируется (fail-open) — проверка через `hermes doctor`.
4. **Профили = отдельные HOME-директории, а не слои.** `hermes profile create coder [--clone|--clone-all|--clone-from]` создаёт независимый home `~/.hermes/profiles/<name>/` со своим `config.yaml`, `.env`, `SOUL.md`, memory, sessions, skills и автоматически регистрирует команду-алиас `coder …` (= `hermes -p coder …`); `hermes profile use` ставит sticky-дефолт (источник: https://hermes-agent.nousresearch.com/docs/user-guide/profiles, What are profiles / Creating a profile). `--clone` копирует только config/`.env`/SOUL/skills, `--clone-all` — всё, кроме per-profile history (`state.db`, `backups/`, `checkpoints/`). Конфликты токенов между профилями ловятся «token locks» — второй gateway блокируется с именованием конфликтующего профиля.
5. **Display-настройки — плоский блок `display:` в `config.yaml`.** Ключи `display.tool_progress` (`off|new|all|verbose`), `display.language`, `display.streaming`, `display.resume_display`, `display.bell_on_complete` задаются прямо в конфиге и правятся тем же `hermes config set` (https://hermes-agent.nousresearch.com/docs/user-guide/configuration, секции display). Отдельного механизма `display.interface` в доках нет — это обычные typed-ключи той же схемы, без per-project переопределения.

## Vellum

Материал `local://vellum-summary.md` тему слоёной конфигурации **не покрывает**: в нём нет ни описания форматов (YAML/JSON), ни резолюции слоёв, ни CLI get/set. Ближайшие аналоги по смыслу:

1. **Изоляция состояния = профиль-подобный скоупинг.** «Изоляция per-user и per-channel» памяти и «Один ассистент, одна память, каждый канал» (vellum-summary.md, Память / Каналы) — это аналог omp-профилей/Hermes-home: скоуп (user/channel/bot) определяет, *какое* состояние читается, но не описан как конфиг-механизм (нет файлов, слоёв, merge-правил).
2. **Actor identity как enforced-слой.** «Actor identity (guardian, trusted, unknown) резолвится один раз и принудительно соблюдается всюду» (vellum-summary.md, Безопасность) — по духу совпадает с Hermes managed scope: неизменяемый более высокий слой, который нельзя перебить нижними, только у Vellum это про identity/permissions, а не про настройки.
3. **Хостинг как граница конфигурации.** «Managed runtime на Vellum Platform или self-hosted. Один кодбейз, одна модель данных» (vellum-summary.md, Хостинг) — выбор окружения заменяет часть того, что у omp решают оверлеи/профили, но механизма конфиг-резолюции в материале нет.

## Решение (одно/комбо)

Комбо: **скелет и семантика — от omp, изоляция ботов — от Hermes-профилей, админ-слой — от Hermes managed scope**. Берём пятиуровневую резолюцию omp (`defaults ← global ← project ← overlays ← runtime`) с его deep-merge-правилом «объекты сливаются, массивы/скаляры заменяются» — это самая проработанная и предсказуемая модель из трёх, плюс её же schema-driven `get/set/reset` с типизированным парсингом значений (это даёт эффективность: никакого ручного парсинга, ошибки на этапе set, а не в рантайме). Карантин битых файлов делаем omp-стилем (`.broken-<ts>-<pid>` сиблинг + понятная ошибка старта), но для опциональных/админских слоёв — fail-open как у Hermes managed scope, чтобы чужой битый файл не убивал запуск бота. Мультиботность titi отображаем на Hermes-подход: каждый бот — свой корневой каталог состояния (config+secrets+SOUL), а не «слой», что даёт чистую изоляцию без усложнения merge. Runtime-оверлеи и env-подстановку `${VAR}` берём у обоих — это дешёво и закрывает CI/секреты.

## Rust-маппинг

**Новый крейт: `titi-config`** (зависимости: `titi-core` — только типы ошибок/модели; ничего не тянет из providers/tools — config должен грузиться до всего остального).

Ключевые типы (эскизы):

```rust
// titi-config/src/layers.rs
pub enum Layer { Defaults, Global, Project, Overlay(usize), Runtime }
pub enum ConfigSource { DefaultYaml, ProjectYaml, Overlay(PathBuf), CliFlag, EnvVar }

// титул значения: непрозрачное serde-дерево + provenance для отладки
pub struct Resolved {
    pub value: serde_json::Value,          // merged tree
    pub provenance: BTreeMap<String, Layer>, // ключ -> слой-источник
}

pub fn deep_merge(base: &mut Value, over: Value); // Object: рекурсивно; Array/Scalar: replace

// titi-config/src/schema.rs
pub enum SettingType { Bool, Number, Enum(&'static [&'static str]), Array, Record, String }
pub struct SettingEntry { pub path: &'static str, pub ty: SettingType, pub default: Value, pub secret: bool, pub description: &'static str }
pub static SETTINGS_SCHEMA: &[SettingEntry];

// titi-config/src/store.rs
pub struct SettingsStore { layers: Vec<(Layer, Value)>, schema: &'static [SettingEntry] }
impl SettingsStore {
    pub fn load(home: &Path, project: Option<&Path>, overlays: &[PathBuf], profile: Option<&str>) -> Result<Self, ConfigError>;
    pub fn get(&self, dotted: &str) -> Option<&Value>;                 // resolved view
    pub fn provenance(&self, dotted: &str) -> Option<Layer>;
    pub fn set(&mut self, dotted: &str, raw: &str) -> Result<(), ConfigError>; // parse по SettingType; пишет ТОЛЬКО global-файл
    pub fn reset(&mut self, dotted: &str) -> Result<(), ConfigError>;  // персистит schema default в global
    pub fn runtime_override(&mut self, dotted: &str, v: Value);        // неперсистентно
}

// титул карантина
pub enum ConfigError { Parse{path, source}, Schema{path, key, expected}, Quarantined{original, backup}, StrictOverlay{path} }
fn quarantine_broken(path: &Path) -> Result<PathBuf, ConfigError>; // rename -> ".broken-{ts}-{pid}-{uuid}"
```

CLI-поверхность в `titi-cli`: `titi config list|get|set|reset|path` (маппинг 1:1 на omp-семантику: `list --json`, маскирование `secret: true`, `get` возвращает секрет открыто). Профили ботов: `titi --bot <name>` → root `~/.titi/bots/<name>/` (config.yml, .env, SOUL.md), общий `~/.titi/config.yml` как global-слой поверх defaults — профили-алиасы без дублирования схемы. Форматы: `.yml`/`.yaml` канонично, `.json`/`.jsonc` читается (без миграции), migration `settings.json → config.yml` — one-shot с `.bak`.

Внешние крейты:

| Крейт | Роль |
|---|---|
| `serde` + `serde_json` | внутреннее дерево значений (Value как каноническое представление) |
| `serde_yaml` / `serde_yml` | YAML-слой; JSONC — `jsonc-parser` |
| `schemars` | опционально: генерация JSON Schema из typed-настроек для валидации |
| `fd-lock` | файловая блокировка при карантине и debounce-сохранении |
| `dunce`/`dirs` | резолюция `~`, платформенные пути |
| `tracing` | предупреждения о fail-open битых managed/оверлей-слоёв |
| `notify` (опционально) | hot-reload конфига в long-running боте |

## Definition of Done

- [ ] `titi-config` резолвит 5 слоёв в порядке `defaults ← global ← project ← overlay ← runtime`; интеграционный тест с fixture-деревом проверяет: объект из project deep-merge-ится с global, а массив/скаляр из project **заменяет** global (кейс `disabledProviders`).
- [ ] Повторные `--config a.yml --config b.yml` и `PI_CONFIG_FILES` (env, `:`-разделитель) применяются по порядку: поздний файл побеждает ранний; тест покрывает оба источника.
- [ ] Битый (не-mapping) global/project YAML карантинится: файл переименован в `.broken-*` сиблинг, `SettingsStore::load` возвращает `ConfigError::Quarantined{original, backup}`, валидный старт после удаления не требуется; strict-оверлей с невалидным YAML возвращает `StrictOverlay` и **не** переименовывает файл.
- [ ] `titi config set <key> <value>` парсит значение по типу схемы: `yes/on/1` → bool true, невалидный enum перечисляет допустимые значения, массив/record принимают только JSON; `set` пишет исключительно global-файл (тест: project-файл не изменился).
- [ ] `titi config reset <key>` записывает schema-default в global (ключ остаётся в файле со значением по умолчанию) — проверяемо чтением файла и `config get`.
- [ ] `titi config list --json` маскирует `secret: true` ключи (`value` отсутствует, `redacted: true`), `titi config get <secret-key>` возвращает значение открыто; unknown key в `get`/`set` — ненулевой код выхода.
- [ ] Бот-профиль `titi --bot coder` читает `~/.titi/bots/coder/config.yml` и не видит конфиги других ботов; тест на изоляцию двух профилей (ключ задан только в bot A — в bot B он равен default).
- [ ] `${VAR}`-подстановка в значениях: определённая переменная разворачивается, неопределённая остаётся verbatim с `tracing`-warning; unit-тест на оба случая.

## Deep-dive

План подсистем-доков `docs/research/config-settings/<subsystem>.md` (не пишутся в этой задаче):

1. `merge-semantics.md` — формальная семантика deep-merge: рекурсия по Object, replace для Array/Scalar, поведение null-значений, provenance-трекинг для `config get --debug`.
2. `quarantine-and-locking.md` — карантин битых файлов: уникальные имена, файловые блокировки (fd-lock), debounce-сохранение с ре-чтением под локом, fail-open для необязательных слоёв.
3. `schema-and-cli.md` — устройство SETTINGS_SCHEMA: типы, enum-значения, секретность, tab-группировка для `list`, парсинг значений `set`, совместимость `.json/.jsonc`.
4. `bot-profiles.md` — мультиботная изоляция: relocated root per bot, наследование ключей keybindings/горячих настроек от базового профиля, token-locks против двух процессов на одном боте.
5. `env-substitution.md` — подстановка `${VAR}`/`${env:VAR}` в значениях, границы (что не разворачивается), взаимодействие со слоем секретов `.env`.
