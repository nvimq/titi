# UX агента

Тема покрывает весь интерактивный слой: композер ввода, slash-команды, overlay-панели, transcript-рендеринг, статус-линию, переключение сессий, скорость первого кадра и mouse tracking.

## omp

1. **Slash-команды — capability-система с приоритетами и дедупликацией.** Команды — capability `id: "slash-commands"` (ключ — имя); провайдеры (`native` p100 → `omp-plugins` p90 → `claude` p80 → `claude-plugins`/`agents`/`codex` p70 → `opencode` p55) сортируются по приоритету по убыванию, дубликаты: победитель в `result.items`, проигравшие помечаются `_shadowed = true` в `result.all`. Пайплайн отправки промпта строго упорядочен: built-in registry (диспатчится **до** `AgentSession.prompt` и резервирует имена) → extension-команды → TypeScript-custom/MCP prompt-команды → `expandSlashCommand` (markdown-команды с `$1`, `$@`, `$ARGUMENTS`, quote-aware `parseCommandArgs` без backslash-экранирования) → prompt templates → доставка. Неизвестный `/...` **не отбрасывается** — уходит в LLM как обычный текст. Источник: `omp://slash-command-internals.md`.
2. **Очередь сообщений non-blocking — через keybinding-действия.** `app.message.followUp` (`Ctrl+Q`, `Ctrl+Enter`) ставит сообщение в очередь во время стрима, `app.message.dequeue` (`Alt+Up`, `Shift+Up`) возвращает сообщение из очереди в редактор; при `streamingBehavior` без значения `prompt(...)` бросает ошибку — steer/followUp надо выбирать явно. Ремапы: `~/.omp/agent/keybindings.yml`, YAML-маппинг `action-id → chord | [chords]`, пустой массив отключает действие, старые `keybindings.json` мигрируются. Вставка: OSC 5522 enhanced paste передаёт clipboard-MIME напрямую (картинка → `[Image #N]`), иначе bracketed paste для текста, а путь к одиночному image-файлу при вставке загружается как картинка. Источник: `omp://keybindings.md`.
3. **Модальные панели и тема.** Overlay/диалоги рисуются рамками из токенов `boxRound.*` (скруглённые углы `╭╮╰╯`) + `boxSharp.*` (тройники/кресты); у Settings есть live-превью темы через `previewTheme` (без персиста, откат при отмене) vs персистентная `setTheme` (при неудаче — fallback на встроенный `dark`); watch только за файлом текущей кастомной темы в `~/.omp/agent/themes`. Тема валидируется по обязательному набору ~66 цветовых токенов, включая 13 `statusLine*` (model, path, gitClean/Dirty, context, spend, cost, subagents…). Источник: `omp://theme.md`.
4. **Modal-режимы как UX-паттерны.** `/pause` — TUI-only built-in: process-global gate для main-агента и подагентов (каждый паркуется на ближайшей безопасной границе, ничего не абортится; Esc/Enter/Space/Ctrl+C снимают, Ctrl+C именно возобновляет). Vibe mode — статус-линия показывает индикатор `Vibe`; вход/выход — slash-команда с инлайн-директивой `/vibe <prompt>`. Источники: `omp://slash-command-internals.md` (§10), `omp://vibe-mode.md`.
5. **Approval UX.** Три уровня декларации (`read`/`write`/`exec`, неизвестное = `exec`), режимы `tools.approvalMode: always-ask|write|yolo`, per-tool user-override `tools.approval.<tool>`, safety-override `approval: { tier: "exec", override: true, reason }` (bash: `rm -rf /`, fork-бомбы и т.п.), детализация промпта через `formatApprovalDetails(args)`. Источник: `omp://approval-mode.md`.
6. **Диалог вопросов к пользователю.** Инструмент `ask`: rich ask-диалог с per-question header/description/preview, multi-select чекбоксами, «Other (type your own)» через editor, `ask.timeout` (0 = выключен, в plan mode всегда выключен; по таймауту автоподбор `recommended`), `ask.notify` — терминальное уведомление «Waiting for input», `concurrency = "exclusive"`. Рендерер нормализует недокачанные стримингом аргументы для показа. Источник: `omp://tools/ask.md`.
7. **Чего omp-доки не покрывают:** мгновенный первый кадр (нет дока про прогрев/ранний paint), mouse tracking (нет дока), inline paste-collapse длинных вставок, автодополнение как floating panel (упомянут лишь `CombinedAutocompleteProvider` внутри `slash-command-internals.md`). Ближайшие аналоги: theme-инстанцирование в `main.ts` до старта TUI (`omp://theme.md`), autocomplete-провайдер команд (`omp://slash-command-internals.md` §4).

## Hermes

1. **Мгновенный первый кадр + non-blocking input — эталонная пара.** Баннер рисуется до того, как приложение догрузилось («the terminal never feels frozen»); ввод доступен до готовности сессии, статус `starting agent…`, и первое сообщение уходит, как только агент онлайн. Рендеринг — alternate screen с дифференциальными обновлениями: нет фликера при стриме и нет засорения scrollback после выхода. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/tui.
2. **Композер.** Inline paste-collapse длинных вставок, `Cmd+V`/`Ctrl+V`: текст → OSC52/native clipboard → image-attach fallback; bracketed-paste safety; нормализация путей картинок/файлов в аттачменты. В desktop: очередь сообщений редактируема до отправки — Stop/Esc при стоящих в очереди turns ставит очередь на паузу и раскрывает её над композером; ↑/↓ в пустом композере вспоминает прошлые промпты. Источники: TUI и desktop доки (URL выше).
3. **Slash-автодополнение — floating panel с описаниями**, не inline-дропдаун; `/help`, `/model`, `/skin` (live-превью темы при навигации), `/usage`, `/agents` (живое дерево подагентов с kill/pause и стоимостью по веткам) рендерятся как модальные overlay-панели. Источник: TUI док.
4. **Session switcher.** `Ctrl+X` / `/sessions` / `/switch` — живой свитчер только открытых в этом TUI-процессе сессий: ↑/↓ и мышь, `Enter` — переключиться, `Ctrl+D` — закрыть, `Ctrl+N` — новая, `Ctrl+R` — обновить, `+new`-строка с выбором модели по Tab. Клик по счётчику `N live sessions` в статус-линии тоже открывает. `Ctrl+X` при подсвеченном queued-сообщении удаляет его, а `Esc` — только снимает подсветку. Источник: TUI док.
5. **Статус-линия.** Машина состояний (`starting agent…` / `ready` / `thinking…` / `running…` / `interrupted` / `forging session…`), cwd с git-веткой, обновляемой по mtime-кешу из соседнего терминала; `⏱ 12s/3m 45s` — время с последнего промпта / всего сессии (после turn замораживается в `⏲`); `🗜️ N` — число автокомпрессий; `▶ N` — фоновые задачи; `⚠ YOLO` — бейдж автоаппрува. Busy-индикатор сменный (`display.tui_status_indicator: kaomoji|emoji|unicode|ascii`) с выровненной шириной глифов, чтобы статус-бар не дёргался. Источник: TUI док.
6. **Mouse tracking — градуированные пресеты.** `/mouse [on|off|toggle|wheel|buttons|all]`, персистится в `display.mouse_tracking`: `wheel` = 1000+1006 (скролл без hover — рекомендация для tmux, чтобы заглушить спам «No image in clipboard»), `buttons` = +1002 drag-select, `all` = +1003 hover-события (hover-пагинация скроллбара, link mouseenter). Drag выделяет текст равномерным фоном вместо SGR inverse. Источник: TUI док.
7. **Transcript-рендеринг.** `/details [hidden|collapsed|expanded|cycle]` глобально и per-section (`thinking`, `tools`, `subagents`, `activity`) с продуманными дефолтами: thinking и tools — **expanded** (живой стрим транскрипта, а не «стена шевронов»), subagents — collapsed, activity — **hidden** (ambient-шум; ошибки инструментов всё равно инлайном, а при полном скрытии — floating-alert backstop); явные `display.sections.*` побеждают всё. LaTeX `$…$`/`$$…$$` рендерится в Unicode-математику, неподдерживаемое — literal TeX в code-span (копируемо). Обнаружение светлого терминала в 3 слоя: `HERMES_TUI_THEME` → `COLORFGBG` → OSC 11 probe. В desktop ещё: preview-rail сбоку, timeline-rail с маркерами промптов, Cmd/Ctrl+F find-in-page по транскрипту, кликабельный контекст-метр с разбивкой токенов по категориям. Источники: TUI (включая хвост с `/details`), desktop доки.

## Vellum

Материал `local://vellum-summary.md` TUI/композер/рендеринг **не покрывает** — там нет интерфейсного слоя. Ближайшие аналоги, полезные для UX titi:

1. **Каналы и ненавязчивые уведомления:** проактивные уведомления идут «в правильный канал и не прерывают активный диалог» — прямой аналог Hermes' activity-секции (hidden по умолчанию, floating-alert backstop) и omp `ask.notify`. Для titi: ambient-события мультиботной сети (сообщения между ботами) не должны вклиниваться в активный transcript — только бейдж/счётчик.
2. **Actor identity как UI-доверие:** `guardian / trusted / unknown` резолвится один раз и соблюдается всюду; unknown не может ничего. Аналог в UI — визуальная маркировка источника сообщений/запросов (чей бейдж, чей цвет) и approval-промпты, показывающие origin (у omp это `Origin: MCP server tool` в approval prompt, `omp://approval-mode.md`).
3. Мультиканальность (macOS/iOS/Web/Voice/Email/Telegram/Slack) подсказывает, что transcript-состояние должно быть отделимо от конкретного фронтенда — как у Hermes: одна `~/.hermes/state.db`, сессия стартует в одном интерфейсе и продолжается в другом.

## Решение (одно/комбо)

Комбо с эталоном Hermes, механикой omp и учетом Vellum. **Берём у Hermes** (задача прямо называет его эталоном UX): мгновенный первый кадр (баннер до готовности), non-blocking очередь до готовности сессии, alternate-screen дифференциальный рендеринг, floating autocomplete-панель, модальные overlay для model/sessions/approval, градуированные mouse-пресеты (`off|wheel|buttons|all`), продуманные per-section дефолты transcript (`thinking`/`tools` expanded, `activity` hidden + floating-alert backstop), сменный busy-индикатор с фиксированной шириной глифов. **Берём у omp** (проверенные механизмы, дешевле в реализации, чем придумывать): строгий пайплайн slash-команд с built-in реестром, резервацией имён и дедупом по приоритетам; keybindings как YAML action-id → chord с отключением пустым массивом; schema-тему с обязательными токенами + `previewTheme`/`setTheme`; approval-иерархию `read/write/exec` + режимы + per-tool overrides. **От Vellum** — только принцип: ambient-события мультиботной сети в бейджи, не в поток, и маркировка актора в UI. Это оптимально по усилиям: UX-формы копируем у эталона, а семантику и устойчивые контракты — у omp, не изобретая ни то, ни другое.

## Rust-маппинг

**Крейты workspace:** `titi-core` (события сессии/агента), `titi-providers` (стриминг), `titi-tools` (approval-тиры), `titi-tui` (весь UX-слой; при росте — новые `titi-tui-widgets` (overlay/панели) и `titi-tui-markdown` (transcript-рендеринг)), `titi-cli` (запуск, флаги `--mouse`, `--theme`).

**Ключевые типы в `titi-tui`:**

```rust
// Композер: bracketed paste + очередь
pub struct Composer { buf: EditorBuffer, paste: PasteState, queue: VecDeque<Queued> }
pub enum PasteState { None, Bracketed, Osc5522 { mime: Mime } } // crossterm keyboard-enhancement + свой OSC-парсер
impl Composer {
    pub fn push_queue(&mut self, msg: String, mode: QueueMode);          // Steer | FollowUp (аналог app.message.followUp)
    pub fn dequeue_last(&mut self) -> Option<Queued>;                    // аналог app.message.dequeue
    pub fn collapse_paste(&mut self, text: &str) -> Span;                // inline paste-collapse длинных вставок
}

// Slash-команды: пайплайн omp
pub struct SlashRegistry { builtins: Vec<BuiltinCmd>, files: Vec<FileCmd> } // builtins резервируют имена
pub enum Route { Builtin(Residual), Handled, Expanded(String), Passthrough } // Passthrough = /... уходит в LLM
impl SlashRegistry {
    pub fn route(&self, input: &str) -> Route;      // порядок: builtin -> custom -> file($ARGUMENTS) -> template
    pub fn complete(&self, prefix: &str) -> Vec<Completion>; // floating panel с описаниями
}

// Overlay-панели
pub trait Overlay { fn draw(&self, f: &mut Frame, area: Rect); fn on_key(&mut self, k: KeyEvent) -> OverlayFlow; }
pub enum OverlayFlow { Stay, Close, Consume } // Esc — cancel без удаления (как Ctrl+X/Esc в Hermes)

// Статус-линия
pub struct StatusLine { state: AgentState, timers: TurnTimers, badges: Badges } // starting/ready/thinking/running/interrupted
// Машина состояний: первый кадр рисуется до готовности провайдера (state = Starting), ввод всегда принят

// Тема
pub struct Theme { colors: ColorTokens, symbols: SymbolPreset }  // serde-валидация: обязательные токены -> ошибка загрузки
```

**Внешние крейты:** `crossterm` (raw mode, `EnableMouseCapture`/пресеты 1000/1002/1003+SGR 1006, bracketed paste, keyboard enhancement flags), `ratatui` (buffered-diff рендеринг = аналог «дифференциальных обновлений» alternate screen Hermes), `pulldown-cmark` + `syntect` либо `tree-sitter` (markdown + подсветка в transcript), `unicode-width` (ширина глифов busy-индикатора, чтобы статус-бар не дёргался), `serde`/`serde_yaml` (keybindings.yml, тема), `tokio` (background-очередь сообщений). Async-очередь steer/followUp — через `tokio::sync::mpsc` из `titi-core`.

## Definition of Done

- [x] Первый кадр: `titi` рисует баннер и статус-линию до завершения инициализации провайдера; замер `time-to-first-frame` < 150 мс на тестовом стенде; ввод в это время принимается в очередь и уходит после готовности (интеграционный тест: мок-провайдер с задержкой 2 с, промпт отправлен в очереди, доставлен после ready).
- [x] Bracketed paste: вставка многострочного текста не выполняет команды по строкам; вставка > N строк сворачивается в inline-collapse; вставка пути к `.png` возвращает аттачмент `[Image #N]` (unit-тесты на `Composer::collapse_paste` и парсер OSC 5522).
- [x] Slash-команды: built-in имена зарезервированы; `/unknown-xyz` уходит в LLM как текст (Passthrough); `$1`/`$ARGUMENTS` подставляются; автодополнение — floating panel с описаниями (снапшот-тест `SlashRegistry::route` + `complete`).
- [ ] Очередь: во время стрима `Steer`/`FollowUp` ставятся в очередь, `Alt+Up` возвращает последнее в редактор, Esc снимает подсветку не удаляя (тест на `Composer::queue`).
- [x] Overlay-панели: model picker, session switcher (`Ctrl+X`: Enter/Ctrl+D/Ctrl+N/Esc), approval-промпт реализуют `Overlay`; Esc всегда cancel-без-удаления.
- [x] Transcript: thinking и tools expanded, subagents collapsed, activity hidden по умолчанию; переключение `/details <section> <mode>`; floating-alert backstop при полностью скрытых секциях; markdown-рендер с токенами темы (golden-тесты рендера).
- [x] Статус-линия: машина состояний starting/ready/thinking/running/interrupted; таймер `⏱`/`⏲`; бейджи (компрессии, фоновые задачи, YOLO); busy-индикатор со сменным пресетом и постоянной шириной (тест: ширина строки не меняется на кадрах спиннера).
- [x] Мышь: пресеты `off|wheel|buttons|all` маппятся на режимы 1000/1002/1003 + SGR 1006 и персистятся в конфиге; drag-select рисует selection-фон (интеграционный тест на эмуляции SGR-mouse-событий).

## Deep-dive

Подсистемы-доки 2-го уровня (план; писать при детализации соответствующей подсистемы):

- `docs/research/agent-ux/composer.md` — композер: bracketed paste, OSC 5522, paste-collapse, очередь steer/followUp/dequeue, редактирование очереди (Hermes desktop).
- `docs/research/agent-ux/slash-commands.md` — реестр, приоритеты/дедуп, пайплайн роутинга, floating autocomplete.
- `docs/research/agent-ux/overlays.md` — модальные панели: model/sessions/approval/ask; фокус-модель, Esc-семантика.
- `docs/research/agent-ux/transcript.md` — markdown/LaTeX, tool-call partials при стриме, per-section accordion, floating-alert backstop, find-in-page.
- `docs/research/agent-ux/status-line.md` — машина состояний, таймеры, бейджи, busy-индикаторы, ширина глифов.
- `docs/research/agent-ux/session-switcher.md` — live switcher, автодетект светлого терминала, shared-сессии между фронтендами.
- `docs/research/agent-ux/first-frame.md` — прогрев, ранняя отрисовка, alternate-screen diff-рендеринг.
- `docs/research/agent-ux/mouse.md` — пресеты 1000/1002/1003, tmux-совместимость, drag-select.
