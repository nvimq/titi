# Системный промпт и SOUL

Тема: слоты системного промпта, SOUL.md как slot #1, PERSONALITY.md, security-сканирование инъекций, кастомизация через SYSTEM.md/APPEND_SYSTEM.md.

## omp

omp не имеет отдельного «файла души» — идентичность встроена в дефолтный шаблон и кастомизируется набором дискаверимых файлов. Механика сборки промпта:

1. **Входы и приоритет.** `--system-prompt <text-or-file>` (CLI) и `SYSTEM.md` (дискавери) заменяют дефолтный шаблон инструкций на `custom-system-prompt.md`; `--append-system-prompt` и `APPEND_SYSTEM.md` добавляют текст в конец рендера. Флаг всегда побеждает файлы; в рамках одного имени файл project-скоупа побеждает user-скоуп. Дискавери ищет project-first (`<cwd>/.omp/`, затем `.claude/`, `.codex/`, `.gemini/`), потом user-level (`~/.omp/agent/`, далее те же альтернативные базы); ancestors не обходятся. Источник: `omp://system-prompt-customization.md`.
2. **Что сохраняется при замене.** `SYSTEM.md` не становится сырой системой-сообщением: шаблон `custom-system-prompt.md` всё равно рендерит сгенерированные поверхности — discovered context files, skills, always-apply rules + rulebook, secret-redaction guidance; отдельно остаётся project/environment footer. Дата и cwd вынесены из футера в `<system-reminder>` на первом user-turn каждого запроса (`date-cwd-reminder.md`) — это сохраняет prefix-cache у open-weight провайдеров и позволяет refreshing даты без пересборки промпта (#7404). Реализация: `packages/coding-agent/src/main.ts` (`discoverSystemPromptFile`, `applyResolvedSystemPromptInputs`), `src/system-prompt.ts` (`buildSystemPrompt`, `resolvePromptInput`). Источник: `omp://system-prompt-customization.md`.
3. **PERSONALITY.md.** Дефолтный шаблон рендерит personality-блок по настройке `personality` (`default`, `friendly`, `pragmatic`, `none`); user-level `~/.omp/agent/PERSONALITY.md` заменяет текст выбранного пресета (только agent-директория, без project-скоупа и других config-баз; `personality: none` опускает блок целиком — сабагенты всегда с `none`; пустой/нечитаемый файл → fallback на пресет с warning). Также есть `TITLE_SYSTEM.md` для кастомизации автотайтлов с жёстким контрактом нормализации (первая строка, strip quotes/`<title>`, отклонение >80 символов или >12 слов). Пользовательский текст вставляется verbatim: Handlebars-переменные (`{{cwd}}`) не поддерживаются. Источник: `omp://system-prompt-customization.md`.
4. **Magic keywords как скрытые слоты пер-тура.** `ultrathink`/`orchestrate`/`workflowz` — standalone prose-слова в промпте, добавляющие скрытые user-attributed notice-инструкции на этот turn: точное lowercase-совпадение, игнор fenced code/inline code/HTML-комментариев, включены по умолчанию, ключи `magicKeywords.enabled`, `magicKeywords.ultrathink|orchestrate|workflow`. Реализация атрибуции: hidden notice — «non-displayed custom messages attributed to the user». Источник: `omp://magic-keywords.md`.

**Вывод:** omp покрывает слоты промпта через: дефолтный шаблон → замена (`SYSTEM.md`) → append (`APPEND_SYSTEM.md`) → personality-блок (`PERSONALITY.md`) → пер-турные hidden notices (magic keywords). Отдельного слота identity/SOUL нет; ближайший аналог — personality-блок в дефолтном шаблоне.

## Hermes

Hermes строит промпт как упорядоченный стек слотов, где идентичность — явно первый слот. Ключевые факты:

1. **SOUL.md = slot #1, identity-слот.** Файл живёт только в `HERMES_HOME` (`~/.hermes/SOUL.md`) — не в cwd, чтобы личность не менялась от проекта к проекту. Занимает slot #1 системного промпта, заменяя hardcoded identity; контент инжектится verbatim без wrapper-языка; не дублируется в context files. Hermes автоматически сеет starter-`SOUL.md`, никогда не перезаписывает существующий, а при пустом/нечитаемом/whitespace-файле (или при `skip_context_files`) падает на built-in default identity. Полный стек промпта: SOUL.md → tool-aware guidance → memory/user context → skills guidance → context files (`AGENTS.md`, `.cursorrules`) → timestamp → platform formatting hints → overlays (`/personality`). Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/personality
2. **Security-сканирование и truncation.** `SOUL.md` проходит prompt-injection scanning «как другие context-bearing files» до включения в промпт, плюс truncation при превышении размера. Т.е. persona-файл — не доверенная зона: инъекционные паттерны вычищаются, а не вербатим. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/personality
3. **/personality — session-level overlay.** 14 built-in пресетов (helpful, concise, technical, creative, teacher, kawaii, catgirl, pirate, shakespeare, surfer, noir, uwu, philosopher, hype), переключение `/personality <name>`, сброс через `none|default|neutral`. Кастомные пресеты — в `~/.hermes/config.yaml` под `agent.personalities` (можно переопределить built-in по имени), выбор хранится в `display.personality`; персонал-пресеты никогда не трогают `agent.system_prompt` (ручной промпт применяется только без выбранной personality). SOUL.md = долговременный baseline, /personality = временный mode-switch. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/personality
4. **Разделение SOUL vs AGENTS.md.** SOUL.md — identity/tone/style («если должно следовать за вами везде»); AGENTS.md — project architecture/conventions/workflows. Рекомендованный workflow: глобальный SOUL.md + проектные инструкции в AGENTS.md + /personality только для временных сдвигов. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/personality

## Vellum

Vellum — мультиканальный персональный ассистент; идентичность — живой артефакт, а не статичный файл:

1. **SOUL.md как «душа» и поведенческий контракт.** Поведение ассистента живёт в SOUL.md (источник: `local://vellum-summary.md`, раздел «Идентичность (SOUL)»). Это ядро персональности, из которого растут остальные артефакты: per-user журнал размышлений (reflections) и `NOW.md` — скретчпад текущего фокуса и активных нитей.
2. **Самопишущаяся личность при онбординге.** При онбординге ассистент наблюдает, как пользователь общается, и **сам пишет свои файлы личности** — SOUL эволюционирует из наблюдений, а не заполняется пользователем вручную (источник: `local://vellum-summary.md`). Это модель «identity grows», противоположная статичному verbatim-инжекту Hermes.
3. **Мультиботность через пер-бот изоляцию души.** По приоритетам пользователя для titi: «Сеть ботов: пер-бот изоляция SOUL+memory+skills; обмен опытом между ботами» — у каждого бота свой SOUL.md, и изоляция души (наряду с памятью и скиллами) — требование безопасности сети. Дополнительно: actor identity (guardian/trusted/unknown) резолвится один раз и принудительно соблюдается всюду, unknown-акторы не могут читать память/триггерить инструменты/эскалировать (источник: `local://vellum-summary.md`, раздел «Безопасность»).

## Решение (одно/комбо)

Комбо: **слотовая архитектура Hermes как скелет + дискавери-кастомизация omp как механизм + пер-бот SOUL Vellum как модель сети**. Промпт titi строится из нумерованных слотов: slot #1 — identity (`SOUL.md` из `~/.titi/agent/`, auto-seed, verbatim после security-сканирования и truncation, fallback на built-in identity), далее слоты tools-guidance, context, skills, rules, project-footer — это ровно карта Hermes и одновременно соответствует omp-шаблону, где сгенерированные поверхности (context/skills/rules) сохраняются даже при замене инструкций. Кастомизация — по omp: `SYSTEM.md` (замена instruction-шаблона с сохранением сгенерированного), `APPEND_SYSTEM.md` (добавка), `PERSONALITY.md` (замена personality-блока), project-first дискавери без ancestor-walk, флаг побеждает файл. Security-сканирование инъекций берём у Hermes: все persona/context-файлы сканируются на инъекционные паттерны до инжекта + truncate; этому же подвергаются hidden notice-сообщения (аналог magic keywords, но сканируемые). Для мультиботности — правило Vellum: у каждого бота свой `SOUL.md` + memory + skills в изолированном namespace, identity-слот собирается пер-бот, а обмен опытом идёт через явный механизм, а не через общий промпт. Оптимальность: один проход сборки промпта (render-once, кэшируемый prefix — переносимемую дату/cwd держим в первом user-remindе, как omp), сканирование выполняется один раз при загрузке файла и кэшируется по mtime.

## Rust-маппинг

**Крейты workspace:**
- `titi-core` — сборка промпта и identity: слоты, сканирование, дискавери файлов.
- `titi-providers` — рендер промпта в provider-запросы (prefix-cache: разделяемая system-часть vs per-request напоминания).
- `titi-tools` — no direct role; hidden notice-сообщения аналога magic keywords атрибутируются как user-сообщения через core.
- `titi-tui` — подсветка magic keywords при редактировании (градиенты), `/personality`-команда.
- `titi-cli` — флаги `--system-prompt`, `--append-system-prompt`, дискавери конфиг-баз.

**Предлагаемый новый крейт:** `titi-soul` — identity-слот: загрузка `SOUL.md`, auto-seed, injection-сканер, truncation, per-bot namespace.

**Ключевые типы/трейты (эскизы):**

```rust
/// Порядок слотов фиксирует стек Hermes; identity — slot 0.
pub enum PromptSlot {
    Identity(Soul),            // slot #1 (Hermes)
    ToolsGuidance,             // слоты 2..n — стек Hermes/omp
    ContextFiles(Vec<ContextFile>),
    Skills(Vec<SkillRef>),
    Rules,
    ProjectFooter,
    PersonalityOverlay(Personality), // session overlay (Hermes /personality)
}

#[async_trait::async_trait]
pub trait SystemPromptBuilder {
    /// assemble рендерит слоты в порядок; per-request parts (date/cwd)
    /// НЕ попадают сюда — идут в первый user-turn reminder (omp #7404).
    async fn assemble(&self, ctx: &SessionContext) -> anyhow::Result<RenderedPrompt>;
    fn with_custom_template(&mut self, s: Option<ResolvedInput>) -> &mut Self;   // SYSTEM.md
    fn with_append(&mut self, s: Option<ResolvedInput>) -> &mut Self;            // APPEND_SYSTEM.md
}

/// Загрузка души: per-bot namespace (Vellum-изоляция), auto-seed (Hermes).
pub trait SoulLoader {
    async fn load(&self, bot: &BotId) -> anyhow::Result<Soul>; // seed при отсутствии
}

/// Security-сканирование инъекций (Hermes) до инжекта в слот.
pub trait InjectionScanner {
    fn scan(&self, content: &str) -> ScanReport; // patterns + verdict
}

pub enum ResolvedInput { Text(String), File(PathBuf) } // omp: text-or-file resolution
```

**Внешние крейты:** `tokio` (async IO при дискавери/чтении файлов), `scanlex`/`regex` для паттернов инъекций, `serde`+`yaml` для `config.yaml` (`agent.personalities`), `dirs`/`etcetera` для user-директорий (`~/.titi/agent/`, XDG/profile-осведомлённость как omp), `pulldown-cmark` при необходимости структурного разбора persona-файлов, `tracing` для warnings при fallback-ах (empty/unreadable SOUL → built-in identity).

## Definition of Done

- [ ] `titi-soul::SoulLoader::load` создаёт starter-`SOUL.md` при отсутствии и НИКОГДА не перезаписывает существующий; unit-тест на оба сценария (seed + сохранение пользовательского).
- [ ] Пустой/whitespace/нечитаемый `SOUL.md` → built-in default identity + `tracing::warn!`; тест фиксирует fallback-текст в собранном промпте.
- [ ] `InjectionScanner::scan` детектирует набор инъекционных паттернов (role-override, "ignore previous instructions", скрытые директивы) в golden-тесте; отсканированный SOUL попадает в slot #1 verbatim при чистом скане и truncation-ится сверх лимита.
- [ ] `SystemPromptBuilder::assemble` рендерит слоты в порядке Identity → ToolsGuidance → Context → Skills → Rules → ProjectFooter; тест на порядок и на сохранение context/skills/rules при активном custom-шаблоне (поведение omp `custom-system-prompt.md`).
- [ ] Дискавери: project-first (`<cwd>/.titi/`, потом `.claude/`-совместимые базы) → user (`~/.titi/agent/`), без ancestor-walk, флаг CLI побеждает файл; тест на матрицу приоритетов.
- [ ] Дата и cwd не входят в system-промпт, а эмитятся как reminder в первом user-turn; prefix-cache-friendly: тест, что два запроса в одной сессии дают идентичную system-часть при смене даты на границе полуночи.
- [ ] Мультиботность: `SoulLoader::load(bot_a)` и `load(bot_b)` читают разные namespace-файлы; тест на изоляцию двух ботов.
- [ ] `/personality` overlay применяется к текущей сессии поверх SOUL и сбрасывается `none|default|neutral`; тест, что overlay не модифицирует сохранённый `SOUL.md`.

## Deep-dive

Подсистемы-доки 2-го уровня (`docs/research/system-prompt-soul/<subsystem>.md` — план, писать при детализации):

- `docs/research/system-prompt-soul/slot-pipeline.md` — конвейер сборки слотов, порядок рендера, prefix-cache-стратегия, per-request reminders.
- `docs/research/system-prompt-soul/soul-loader.md` — auto-seed, per-bot namespace, truncation policy, файловая схема `~/.titi/agent/`.
- `docs/research/system-prompt-soul/injection-scanner.md` — паттерны prompt-injection, verdict-модель, кэширование по mtime, что сканируем (SOUL, persona-пресеты, hidden notices).
- `docs/research/system-prompt-soul/personality-presets.md` — built-in пресеты, `agent.personalities` в config, session-overlay state, совместимость с PERSONALITY.md-моделью omp.
- `docs/research/system-prompt-soul/discovery-and-flags.md` — text-or-file resolution, матрица config-баз, поведение при нечитаемых файлах, TITLE_SYSTEM-аналог для автотайтлов.
