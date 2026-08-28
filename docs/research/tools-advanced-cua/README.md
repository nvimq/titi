# Продвинутые инструменты и CUA-драйвер

Тема: скриптуемые инструменты поверх рантаймов (eval/notebook), web_search, browser, computer-use (CUA-драйвер: сначала локальный компьютер, удалённые машины — архитектурный крюк), inspect_image, generate_image, TTS, github, security scan.

## omp

**eval (py/js REPL).** Один вызов = одна ячейка; состояние сохраняется в рантайме между вызовами. Бэкенды `py` (retained IPython-style kernel) и `js` (retained Bun worker VM) включены по умолчанию, `rb`/`jl` — opt-in; ключи `eval.py`/`eval.js`/`eval.rb`/`eval.jl` и env-оверрайды `PI_PY`/`PI_JS`. Python-ядро — subprocess `python -u runner.py`, говорящий NDJSON по stdin/stdout (фреймы `started`/`stdout`/`stderr`/`display`/`result`/`error`/`done`); rich display через MIME-бандлы (`text/markdown` > `text/plain` > `text/html`; `image/png`, `application/json`, `application/x-omp-status`). Транформер переписывает IPython-магии (`%pip`, `%cd`, `%%bash`, `!cmd`, `%%capture`) в обычный Python. Timeout по умолчанию 30 с (`0` отключает), отмена — SIGINT в раннер с эскалацией до SIGKILL через 5 с (`INTERRUPT_ESCALATION_MS`), между запросами раннер ставит `SIG_IGN` на SIGINT. Источник: omp://tools/eval, omp://python-repl (`packages/coding-agent/src/eval/py/runner.py`, `src/eval/py/kernel.ts`).

**Notebook.** Критичное различие: поддержка `.ipynb` — это конвертация/редактирование файлов, а НЕ исполнение. `read` показывает ноутбук как редактируемый текст с маркерами `# %% [code|markdown|raw] cell:N`; edit-пайплайн через `serializeEditedNotebookText()` (src/edit/notebook.ts) делает round-trip обратно в notebook JSON, сохраняя `execution_count`/`outputs` и метаданные. Строки, похожие на маркеры, экранируются `# %%` → `# %%%`. Kernel-lifecycle в этом пути нет; исполнение ячеек — отдельные вызовы `eval`. Источник: omp://notebook-tool-runtime.

**web_search.** Один запрос через цепочку из 23 провайдеров (`SEARCH_PROVIDER_ORDER`: perplexity, gemini, anthropic, codex, xai, zai, exa, tinyfish, jina, kagi, tavily, firecrawl, brave, kimi, parallel, synthetic, searxng, startpage, duckduckgo, ecosia, google, mojeek, `public` — последний только explicit). Запрос парсится один раз `parseSearchQuery()` (Google-стиль `site:`, `after:`, `inurl:` и т.д.), провайдер выбирается лениво, fallback последовательный; после ответа `applyQueryConstraints()` мягко пост-фильтрует источники. Лимиты: сниппеты/цитаты обрезаются до 240 символов, таймаут транспорта провайдера `providers.webSearchTimeoutSeconds` (default 60, max 300); credential-free скраперы (Google/Ecosia/Mojeek) эскалируют failures к общему stealth-headless Chromium. Источник: omp://tools/web_search.

**browser.** Три действия `open|close|run`; вкладки — process-global Map по имени; браузерные kind: headless (проект-шаренный Chromium под daemon-brokerом + 14 stealth-патчей), spawned app (`app.path` + `--remote-debugging-port`), connected (`app.cdp_url` CDP attach), OMP Browser Relay (loopback + MV3-расширение, пользовательские залогиненные табы), cmux. `run` исполняет async-JS через общий `JsRuntime` в выделенном Bun Worker; API вкладки: `tab.observe()` (accessibility-снапшот с числовыми id), `tab.ariaSnapshot()` (Playwright ARIA-YAML с `[ref=eN]`, выполняется в main world страницы), `tab.click/fill/type/press/scroll/drag/select/uploadFile`, `tab.screenshot({selector?, fullPage?})` с ресайзом для модели (cap 1024×1024, 150 KiB, jpeg 70). Пер-op fail-fast: чтения ≤20 с, интерактивные действия ≤15 с внутри бюджета ячейки. Источник: omp://tools/browser.

**computer (CUA).** Персистентный JS-воркер (crash-isolated Bun worker, один native `DesktopSession`) c фасадом `desktop`: `windows()`, `displays()`, `capabilities()`, `clipboard`, скриншоты и ввод `click/move/drag/scroll/type/press` с `delivery: "background"|"foreground"`. Accessibility-first: `win.ax()` (текстовое AX-дерево с `[ref=eN]`), `win.find({role,title})`, `win.ref("e5")`, мутации `setValue/perform/press/focus` без скриншотов; координаты скриншота строго отделены от глобальных AX-координат (`InvalidCoordinateFrame`). `read_only: true` разрешает только чтение (это НЕ песочница). Нативный бэкенд — `crates/pi-natives/src/desktop/` (нативный аддон): macOS ScreenCapture/Quartz + AX + ввод; Linux X11 (XTEST) и Wayland portal; Windows UI Automation. Гейтинг `computer.enabled=false` по умолчанию, `/computer` переключает на сессию. Источник: omp://tools/computer, omp://computer-use.

**inspect_image / generate_image / tts.**
- `inspect_image`: локальный файл или `Image #N` вложение уходит на vision-модель (роли `@vision`→`@default`→активная модель→первая доступная); форматы по сниффингу заголовка PNG/JPEG/GIF/WEBP, cap 20 MiB, авторесайз-лестница (1568×1568, 500 KiB, качества 70/60/50/40, скейлы 1.0→0.25). Режим `inspect_image.mode: auto|on|off` — в `auto` регистрируется только если активная модель не умеет есть картинки. Источник: omp://tools/inspect_image.
- `generate_image`: структурированный промпт (`subject/action/scene/composition/lighting/style/text`), edit через `input[]` + `changes`, провайдеры openai/openai-codex/antigravity/xai/openrouter/gemini с fallback по credentialed HTTP-ошибкам; выход — временные файлы `omp-image-<snowflake>.<ext>`. Включается только через `generate_image.enabled=true`. Источник: omp://tools/generate_image.
- `tts`: маршрутизация `providers.tts: local|xai|auto`; local — Kokoro-82M (onnx-community/Kokoro-82M-v1.0-ONNX q8) через ONNX-воркер, всегда WAV/PCM16, без сети после загрузки весов; xAI Grok Voice — MP3/WAV, голоса `ara/eve/leo/rex/sal`; cap 15000 символов. Источник: omp://tools/tts.

**github / security scan:** omp как харнесс отдельными инструментами `github` и `security scan` тему НЕ покрывает; ближайший аналог — `bash` (вызов `gh` CLI) и чтение/поиск по файлам репозитория (grep/read). Security-сканирования зависимостей/кода в перечисленных доках нет вовсе.

## Hermes

**Computer use (CUA).** Встроенный toolset `computer_use` говорит MCP over stdio к внешнему драйверу [`cua-driver`](https://github.com/trycua/cua) (open-source background computer-use driver). Фундаментальный принцип — «no-foreground contract»: агент читает AX-дерево видимого окна И шлёт синтезированные события, не выводя окно на передний план и не двигая реальный курсор; по платформам — macOS: AX через приватные SkyLight SPI, ввод `SLPSPostEventRecordTo` (pid-scoped, без warp курсора); Windows: UIAutomation + `SendInput`/`PostMessage`; Linux: AT-SPI + XTest/virtual-keyboard. Виден «agent cursor» — тонированный overlay-курсор, реальный курсор не двигается. Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/computer-use. Безопасность: immutable runtime-режимы — `standard` (обычные Hermes-аппрувалы), `bounded` (capability-manifest, ревью один раз при запуске, всё вне манифеста fail-closed), `unrestricted` (YOLO); доступ к залогиненному браузерному профилю требует отдельного config-гранта `computer_use.grant_existing_profile: true` (YOLO его НЕ заменяет). Диагностика — `hermes computer-use doctor` (structured `health_report` MCP-тул, exit 0/1/2). Источник: тот же URL.

**Browser.** Страницы представлены accessibility-деревьями (текстовые снапшоты), интерактивные элементы получают ref-id `@e1`, `@e2` для клика/ввода. Бэкенды: Browser Use cloud (stealth + residential proxies + CAPTCHA), Browserbase, Firecrawl cloud, Camofox (локальный Camoufox/Firefox с фингерпринт-спуфингом), Lightpanda (headless-браузер на Zig), local Chromium-family CDP (`/browser connect`), и дефолтный режим `browser_exec` — агент пишет и исполняет Python через Browser Use CLI 3.0; если CLI не запускается, автоматический fallback на встроенные browser-тулзы. Гибридная маршрутизация (по умолчанию on): URL, резолвящиеся в private/loopback/LAN, идут в локальный Chromium-sidecar, публичные — в облако (SSRF-guard, редирект-трюки блокируются). Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/browser.

**Web search / image gen / TTS (Tool Gateway + прямые провайдеры).** Tool Gateway (Nous Portal) роутит 4 категории одним OAuth: web search & extract (Firecrawl), image gen (9 моделей FAL.ai: FLUX 2 Klein 9B дефолт, FLUX 2 Pro, Nano Banana Pro, GPT Image 2, Ideogram V3, ...), TTS (OpenAI voices в тулзе `text_to_speech`), cloud browser (Browser Use). Маршрутизация — по одному selection-ключу на категорию (`web.backend`, `image_gen.provider`, `tts.provider`, `browser.cloud_provider`); ключ выбирается раз и навсегда: наличие credentials в `.env` НЕ переключает маршрут (явный отказ от silent-fallback, ошибка с подсказкой `hermes tools`). Есть альтернатива SearXNG self-hosted (`web.backend: searxng`) и BYOK (FAL, OpenAI, ElevenLabs). Источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/tool-gateway, https://hermes-agent.nousresearch.com/docs/user-guide/features/web-search.

**execute_code (аналог eval).** `execute_code` — programmatic tool calling: агент пишет Python-скрипт с `from hermes_tools import ...`, Hermes генерирует RPC-стаб, скрипт исполняется в child process, тул-коллы ходят по Unix domain socket обратно в хост; в контекст модели возвращается только итоговый `print()` — промежуточные результаты не входят в окно. Лимиты: timeout 300 с, stdout 50 KB, 50 tool-call'ов на исполнение; env скрабится (переменные с KEY/TOKEN/SECRET/PASSWORD/AUTH вырезаются), tool-whitelist (нельзя рекурсивно `execute_code`/`delegate_task`/MCP). Важно: это НЕ персистентный REPL — новый child process на вызов, состояния между вызовами нет. Отдельной поддержки `.ipynb` (редактирование ячеек) в доках не видно; ближайший аналог — Document Extraction (`read_file` конвертирует офис/ноутбуки в текст) — https://hermes-agent.nousresearch.com/docs/user-guide/features/code-execution.

**inspect_image / vision:** Hermes покрывает vision через вставку картинок в контекст (Vision, https://hermes-agent.nousresearch.com/docs/user-guide/features/vision) — отдельной тулзы «отправить локальный файл на отдельную vision-модель с вопросом» в изученных доках нет; ближайший аналог — vision-анализ скриншотов внутри browser-тулзов. Отдельных тулз github / security scan не покрыто (gh-подобная работа — через терминал/веб-хуки: https://hermes-agent.nousresearch.com/docs/guides/github-pr-review-agent).

## Vellum

Vellum-материал (local://vellum-summary.md) тему продвинутых инструментов покрывает фрагментарно; ключевые факты:

1. **CUA-приоритет пользователя**: «CUA-драйвер: сначала только локальный компьютер, удалённые — позже» — прямо зафиксировано в разделе «Приоритеты пользователя для titi» (local://vellum-summary.md). Удалённые машины — отложенная фаза, но архитектура должна оставить крюк.
2. **Безопасность инструментария**: «Каждый вызов инструмента — в песочнице. По умолчанию — deny»; учётные данные живут в отдельном процессе и никогда не попадают в модель; actor identity (guardian/trusted/unknown) резолвится один раз и соблюдается всюду — unknown-акторы не могут триггерить инструменты. Это напрямую ложится на гейтинг dangerous-инструментов типа computer-use (local://vellum-summary.md, раздел «Безопасность»).
3. **Каналы и Voice**: канал Voice в списке (macOS, iOS, Web, Voice, Email, Telegram, Slack, Twilio) — TTS не декорация, а полноценный канал доставки ответов (local://vellum-summary.md, раздел «Каналы»).

Чего Vellum-материал не даёт: конкретных механизмов eval/REPL, провайдеров web_search, деталей browser-автоматизации и image-gen — по этим подсистемам опираемся на omp и Hermes.

## Решение (одно/комбо)

Комбо: **скриптуемые инструменты — модель omp, CUA-драйвер — архитектура Hermes, приоритет — Vellum.** (1) Персистентные eval-ядра (py/js) как отдельные subprocess-раннеры с NDJSON-протоколом omp — самый эффективный по токенам путь (состояние живёт между вызовами, в контекст идёт только вывод ячейки); notebook — только редактирование маркеров, без исполнения, как в omp. (2) web_search — цепочка провайдеров по трейту с sequential fallback, 2–3 стартовых провайдера (Brave/SearXNG/DuckDuckGo), без 23-провайдерного зоопарка. (3) browser — единый CDP-клиент (headless Chromium + attach к работающему браузеру) с a11y-снапшотами и ref-id, релей в пользовательский Chrome отложен. (4) CUA — изолированный привилегированный sidecar-процесс `titi-cua` (как cua-driver у Hermes, а не нативный аддон в хост-процессе): AX-first, фоновой ввод без кражи фокуса, `read_only`-режим, capability-манифест; trait `CuaBackend` с единственной реализацией `LocalMacos` на первой фазе и заготовкой `Remote` — это и есть требуемый архитектурный крюк. (5) inspect_image/generate_image/TTS — тонкие провайдерные тулзы в titi-providers: vision/generation — облачные модели, TTS — local-first (Kokoro) с cloud-fallback. Эффективность: меньше вызовов LLM (персистентные ядра, execute-in-script), меньше кода (CDP вместо puppeteer-стека, sidecar вместо аддона), безопасность deny-by-default по Vellum.

## Rust-маппинг

**Workspace-крейты (существующие + предлагаемые новые):**

| Крейт | Ответственность |
|---|---|
| `titi-tools` | Регистрация и контракт инструментов: `EvalTool`, `WebSearchTool`, `BrowserTool`, `CuaTool`, `InspectImageTool`, `GenerateImageTool`, `TtsTool`, `NotebookEdit` |
| `titi-providers` | Провайдеры облачных медиа: vision-вызовы, image-gen, облачный TTS, цепочка search-провайдеров |
| `titi-core` | Общие типы tool-call'ов, approvals, гейтинг (`enabled: false` по умолчанию для CUA) |
| `titi-tui` | Рендер ячеек eval, скриншотов, результатов браузера |
| `titi-cli` | CLI-команды `titi computer on|off|status`, `titi cua doctor` (аналог `hermes computer-use doctor`) |
| **новый `crates/titi-eval`** | Оркестрация eval-ядер: спавн/пул/рестарт subprocess-раннеров, NDJSON-транспорт, timeout/отмена |
| **новый `crates/titi-cua`** | CUA-sidecar (бинарь `titi-cua`) + host-клиент: AX-снапшоты, ввод, скриншоты, capability-отчёт |

**Ключевые типы/трейты (эскизы):**

```rust
// titi-core: единый контракт инструмента
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema(&self) -> ToolSchema;              // JSON Schema для модели
    fn approval(&self, params: &serde_json::Value) -> Approval; // read_only -> Read, иначе Exec
    async fn execute(&self, ctx: &SessionCtx, params: serde_json::Value,
                     signal: CancellationToken) -> Result<ToolResult, ToolError>;
}

// titi-eval: бэкенд языка; состояние ядра живёт между вызовами
#[async_trait]
pub trait EvalBackend: Send + Sync {
    fn language(&self) -> Lang;                  // Py | Js
    async fn ensure_kernel(&mut self, key: &KernelKey) -> Result<(), EvalError>;
    async fn run_cell(&mut self, cell: &Cell, sink: &mut OutputSink,
                      signal: CancellationToken) -> Result<CellResult, EvalError>;
    async fn reset(&mut self) -> Result<(), EvalError>;   // деструктивно, только этот язык
}

// titi-providers: поиск и медиа
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn available(&self) -> bool;                 // credentials и т.п., лениво
    async fn search(&self, q: &ParsedQuery, cap: usize,
                    signal: CancellationToken) -> Result<SearchResponse, ProviderError>;
}
pub struct SearchChain { providers: Vec<Box<dyn SearchProvider>> } // sequential fallback

#[async_trait]
pub trait ImageGenProvider { async fn generate(&self, p: &ImagePrompt) -> Result<Vec<GeneratedImage>, ProviderError>; }
#[async_trait]
pub trait TtsProvider     { async fn synthesize(&self, t: &TtsRequest) -> Result<AudioFile, ProviderError>; }

// titi-cua: крюк для удалённых машин; на фазе 1 реализация одна — локальная
#[async_trait]
pub trait CuaBackend: Send + Sync {
    async fn capabilities(&self) -> Capabilities;            // capture/input/ax/permission-стейт
    async fn windows(&self, f: &WindowFilter) -> Result<Vec<WindowInfo>, CuaError>;
    async fn screenshot(&self, t: TargetId, cap: SizeCap) -> Result<Screenshot, CuaError>;
    async fn ax_tree(&self, w: WindowId, depth: u8) -> Result<AxSnapshot, CuaError>;  // [ref=eN]
    async fn resolve_ref(&self, r: &str) -> Result<AxElement, CuaError>;
    async fn act(&self, a: &AxAction) -> Result<(), CuaError>; // setValue/press/click/focus
    async fn input(&self, e: &InputEvent, d: Delivery) -> Result<(), CuaError>; // background|foreground
}
pub struct LocalMacosBackend;   // фаза 1
pub struct RemoteBackendStub;   // NOT IMPLEMENTED — архитектурный крюк (SSH/VNC-транспорт позже)

// Инструмент CUA поверх бэкенда; read_only гонится в sidecar с каждым вызовом
pub struct CuaTool { backend: Arc<dyn CuaBackend> }
```

**Внешние крейты:**
- `tokio` (+ `tokio-util` для `CancellationToken`) — все async-пути; `serde`/`serde_json` — NDJSON-протоколы.
- CUA sidecar (здесь неизбежен `unsafe` для FFI — sidecar-крейт локально переопределяет `#![allow(unsafe_code)]`, workspace-запрет остаётся для остальных): `enigo` (ввод: клавиатура/мышь, кроссплатформенно), `xcap` (скриншоты дисплеев и окон — предпочтительнее одиночного `screenshots` из-за window-capture и мульти-монитора), `accessibility` + `accessibility-sys` (macOS AXUSritable-обёртки над AX API), `core-graphics`/`objc2` (бонды для координат/окон). Ограничения: фоновый pid-scoped ввод как у Hermes (`SLPSPostEventRecordTo`) — приватные SPI; фаза 1 может принять ограничение «input требует Accessibility-разрешения + foreground-доставку через enigo» с флагом capabilities, честно отражая `desktop.capabilities()`-подход omp.
- Browser: `chromiumoxide` (CDP-клиент, async, tokio) — headless-запуск Chromium + attach к работающему; ARIA-подобный снапшот строится по CDP `Accessibility.getFullAXTree` (не тащим puppeteer); скриншоты — CDP `Page.captureScreenshot`.
- Eval: питон-раннер — собственный `runner.py`-аналог (NDJSON поверх stdin/stdout, без Jupyter-зависимости, как в omp); js-бэкенд фазы 1 — embed `rquickjs`/`boa` ИЛИ вынести JS на второй этап (сначала только `py`).
- Notebook: `serde_json` + собственный парсер маркеров `# %% [code] cell:N` (~200 строк, round-trip как в omp).
- TTS: local-first — `ort` (ONNX Runtime) + веса Kokoro-82M; фаза 1 допустима cloud-only с trait-заготовкой. Vision/image-gen — чистые HTTP-клиенты (`reqwest`).
- Search: `reqwest` + `scraper` для credential-free HTML-провайдеров (DuckDuckGo/SearXNG).
- TUI: `ratatui`/`crossterm` (уже в стеке titi-tui).

## Definition of Done

- [ ] `titi-eval`: тест интеграционного уровня «два последовательных вызова `EvalTool` с `language=py» — переменная, определённая в первом, видна во втором (persistance); `reset=true` очищает только python-ядро; timeout-тест: бесконечный цикл в ячейке обрывается за `timeout` секунд, ядро после SIGINT остаётся живым для следующего вызова.
- [ ] Notebook: round-trip-тест `read → edit → write` на fixture-ноутбуке сохраняет `execution_count` и `outputs` исходных ячеек; строка-маркер внутри ячейки экранируется `# %%` → `# %%%` и обратно.
- [ ] `WebSearchTool`: unit-тест цепочки — первый провайдер кидает `ProviderError`, результат приходит от второго; тест парсера `parseQuery` покрывает `site:`, `-term`, `after:`, `"фразу"`, `OR`.
- [ ] `BrowserTool` smoke: headless-запуск chromium (feature-gated тест), `open` → a11y-снапшот страницы-фикстуры содержит интерактивные элементы с `ref=eN` → `click` по ref меняет состояние страницы → скриншот пишется на диск.
- [ ] `titi-cua` sidecar: бинарь стартует, отвечает на RPC `capabilities`; интеграционный тест на macOS (требует granted Accessibility/Screen Recording, `#[ignore]` без них): `ax_tree` возвращает узлы с `[ref=eN]`, `resolve_ref` после мутирующего снапшота кидает `StaleRef` для старого ref, `read_only`-режим отвергает `InputEvent` ошибкой `ReadOnly`.
- [ ] `CuaTool` отключён по умолчанию: без `cua.enabled=true` в конфиге тулза не регистрируется в списке схем (тест на registry); `titi cua doctor` печатает чек-матрицу (bundle/TCC/AX/capture) и exit-код 1 при degraded.
- [ ] `InspectImageTool`: тесты на cap 20 MiB и на отказ для файла с несниффнутым заголовком; `TtsTool`: local-путь пишет корректный WAV/PCM16 (проверка RIFF-заголовка), текст > 15000 символов — schema-error.
- [ ] Удалённый крюк: `RemoteBackendStub` компилируется, `todo`-нет — вместо него метод возвращает `CuaError::Unsupported("remote backend planned")`, и тест фиксирует, что registry принимает произвольный `Arc<dyn CuaBackend>` (подмена на mock-бэкенд проходит без изменений в `CuaTool`).

## Deep-dive

Подсистемные доки (план 2-го уровня; писать по мере проработки реализации):

- `docs/research/tools-advanced-cua/eval-kernels.md` — NDJSON-протокол раннера, lifecycle ядер, магии, MIME-дисплей, отмена/рестарт; маппинг на titi-eval.
- `docs/research/tools-advanced-cua/browser-backends.md` — CDP-клиент, a11y-снапшоты и ref-id, headless vs attach, stealth-минимум, скриншот-пайплайн.
- `docs/research/tools-advanced-cua/cua-local-macos.md` — macOS AX/SkyLight, TCC-разрешения, фоновый ввод, enigo/xcap/accessibility бонды, RPC-протокол sidecar, capability-манифест и doctor.
- `docs/research/tools-advanced-cua/web-search-providers.md` — трейт провайдера, парсер запросов, пост-фильтрация, выбор стартового набора.
- `docs/research/tools-advanced-cua/media-tools.md` — inspect_image (vision-роутинг, ресайз-лестница), generate_image (промпт-схема, провайдеры), TTS (Kokoro local, cloud-fallback).
