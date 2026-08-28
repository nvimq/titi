# Input, capabilities, graphics: ввод, capability-пробы, synchronized output, kitty graphics

Слой терминала: разбор ввода, детект возможностей, атомарные кадры, инлайн-изображения и их память.

## omp

1. **Путь ввода и сборка фрагментов.** `stdin -> ProcessTerminal -> StdinBuffer -> TUI.#handleInput -> focusedComponent.handleInput`; `StdinBuffer` собирает фрагментированные CSI/OSC/DCS/APC/SS3-последовательности и bracketed paste до dispatch. Фильтрация key release, если компонент не выставил `wantsKeyRelease = true`; фокус-маршрутизация через `TUI.setFocus`, курсор — `CURSOR_MARKER`, который frame writer вырезает, запоминая физическую позицию (omp://tui-runtime-internals, omp://tui).
2. **Capability-пробы с типизированными sentinel-владельцами.** Детект терминала выбирает оптимизации (synchronized output, DECCARA, image-протоколы), не меняя history-семантику. `ProcessTerminal` парирует запросы с typed DA1 sentinel owners: приватные CSI-ответы могут приходить разрезанными по stdin-флешам, поэтому реassembly держит частичные ответы до терминатора и не должен утекать probe-байты как пользовательский ввод; каждая новая проба требует typed sentinel owner и побайтовое покрытие разрезанных ответов (omp://tui-core-renderer).
3. **Synchronized output и запись кадра.** Writer нормализует/width-fit строки с выключенным autowrap, пишет viewport, чистит stale-строки ниже, восстанавливает autowrap и synchronized-output-состояние; курсорные записи идут внутри synchronized output, до ESU, чтобы не было второго видимого кадра (omp://tui-core-renderer, omp://tui-runtime-internals).
4. **Kitty graphics и image budget.** Kitty-картинки — transmit-once, place-many: полные base64-данные никогда не ретрансмиссируются каждый кадр. `ImageBudget` удерживает только самые свежие изображения; демоушен удаляет пиксели по id и перекрашивает затронутые viewport-строки height-preserving текстовым фоллбеком, history не реплеится (пиксели исторических строк теряются, т.к. history иммутабельна). Kitty Unicode placeholders остаются capability-gated и переопределяются image-настройками окружения (omp://tui-core-renderer).
5. **Keybindings как действия.** Компоненты матчат ввод через `matchesKey(data, "...")` и менеджер биндингов: `keybindings.matches(data, "app.interrupt")`; пользовательские ремапы — `~/.omp/agent/keybindings.yml` (action ID → chord/список chords, пустой массив отключает), с наследованием профиля и миграцией legacy-имён (omp://tui, omp://keybindings).

## Hermes

1. **Mouse-tracking пресеты.** `/mouse [on|off|toggle|wheel|buttons|all]` в рантайме, персист в `display.mouse_tracking`: `wheel` = 1000+1006 (скролл+клик без hover — рекомендуется в tmux, чтобы hover-события не спамили у промпта), `buttons` = +1002 (terminal-side drag selection), `all` = +1003 (hover UI) (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
2. **Paste-цепочка и capability-детект света.** `Cmd+V`/`Ctrl+V` пробует normal text paste → OSC52/native clipboard reads → image attach; bracketed-paste safety; терминальный фон определяется трёхслойно: `HERMES_TUI_THEME` → `COLORFGBG` → OSC 11 probe (Ghostty, Warp, iTerm2, WezTerm, Kitty) (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
3. **Плоскость keybindings.** Keybindings TUI идентичны classic CLI; `/terminal-setup` ставит локальные биндинги VS Code/Cursor/Windsurf для `Cmd+Enter`; `Ctrl+X` — live session switcher (https://hermes-agent.nousresearch.com/docs/user-guide/tui). Kitty graphics / image protocols в терминале Hermes не документирует — «не покрывает»; ближайший аналог — clipboard-image attach в чат (https://hermes-agent.nousresearch.com/docs/user-guide/tui).

## Vellum

Vellum тему не покрывает. Ближайший аналог — модель безопасности: «Каждый вызов инструмента — в песочнице. По умолчанию — deny» и резолв actor identity один раз с принудительным соблюдением всюду — тот же принцип для терминала: capability-проба, получившая неопознанный ответ, не расширяет возможности (fail-closed), а разбор ввода не должен путать probe-ответы с пользовательским вводом (инъекция через stdin — та же поверхность атаки, что unknown-актор) (local://vellum-summary.md).

## Решение (одно/комбо)

Комбо на базе crossterm: `crossterm` даёт raw mode, event-decode (включая bracketed paste и mouse 1000/1002/1003) и synchronized output примитивы, но probe-слой (DA1-ответы, DSR/CPR, OSC 11, kitty-квитирование) пишем сами по паттерну omp — типизированные sentinel-owners с реassembly частичных ответов, fail-closed: нераспознанный ответ отбрасывается, probe-байты не доходят до компонентов как ввод. Mouse-пресеты берём из Hermes (дефолт `wheel` — лучший компромисс для tmux-пользователей). Kitty graphics включаем transmit-once/place-many с `ImageBudget` (LFU/свежесть + height-preserving текстовый фоллбек при демоушене) — это единственный неизбыточный путь к инлайн-картинкам; все другие протоколы (Sixel/iTerm2) — за фичефлагом позже. Эффективность: synchronized output (CSI ?2026) и diffing вместе дают один видимый кадр и минимальный трафик.

## Rust-маппинг

Крейт `titi-tui`, модули `input.rs`, `caps.rs`, `image.rs`. Зависимости: `crossterm = "0.29"` (event stream, raw mode, `SetBackgroundColor`), `unicode-width` (см. width-model.md), `base64` (kitty transmit), `image` (декод → RGBA для пересчёта размеров), `tokio` (event stream уже в workspace-плане провайдеров; для CLI-потока достаточно `crossterm::event::poll`).

```rust
// titi-tui/src/input.rs
pub enum InputEvent {
    Key(KeyEvent), KeyRelease(KeyEvent), Paste(String),
    Mouse(MouseKind, u16, u16), Resize(u16, u16),
}
pub struct InputBuffer { /* сборка разрезанных CSI/SS3/OSC + bracketed paste */ }
impl InputBuffer {
    /// Прогон байтов; возвращает готовые события. Probe-ответы уходят в probe-реестр, не сюда.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<InputEvent>;
}

// titi-tui/src/caps.rs — типизированные sentinel-owners (omp://tui-core-renderer)
pub trait ProbeOwner { fn sentinel(&self) -> &'static str; fn parse(&self, reply: &[u8]) -> Option<Cap> }
pub enum Cap { SyncOutput2026, Deccara, KittyGraphics, Cpr(CursorPos), Bg(Rgb) }
pub struct Capabilities { sync_output: bool, deccara: bool, kitty: bool }
impl Capabilities {
    pub fn probe<W: Write>(out: &mut W, owners: &mut [Box<dyn ProbeOwner>]) -> Self; // + таймаут
}

// titi-tui/src/image.rs
pub struct ImageBudget { max_pixels: usize /* fresh-first eviction */ }
impl ImageBudget {
    /// Transmit-once: возвращает transmit-команду (base64) для новых id, placement — каждый кадр.
    pub fn frame(&mut self, wanted: &[Placement]) -> Vec<KittyCmd>;
    /// Демоушен: DELETE по id + текстовый фоллбек той же высоты для затронутых строк.
    pub fn demote(&mut self, id: u32) -> Vec<KittyCmd>;
}
```

Keybindings: `titi-tui/src/keys.rs` — `matches_key(data, "Ctrl+P")` + реестр action ID (YAML/serde), схема совместима с `~/.omp/agent/keybindings.yml` (omp://keybindings) как формат-ориентир.

## Definition of Done

- [ ] Тест сборки ввода: CSI-ответ/клавиша, разрезанная на два stdin-флеша, собирается и эмитится как одно событие; частичный префикс не эмиссится как юзер-ввод.
- [ ] Тест probe-изоляции: ответ DA1/DSR внутри потока с пользовательскими байтами не попадает в `InputEvent::Key` (ни один байт пробы не утекает).
- [ ] Тест fail-closed: нераспознанный probe-ответ игнорируется, capability остаётся `false`, дефолтный путь рендера работает.
- [ ] Тест synchronized output: кадр оборачивается в `CSI ?2026h ... CSI ?2026l`, курсорные записи до ESU (golden по ANSI-выводу).
- [ ] Тест mouse-пресетов: `wheel|buttons|all` эмитят ровно 1000+1006 / +1002 / +1003 enable/disable-последовательности.
- [ ] Тест kitty budget: (а) transmit-команда для одной картинки эмитится один раз при N кадрах; (б) после demote — DELETE по id и текстовый фоллбек той же высоты; (в) history не реплеится.
- [ ] Тест keybindings: YAML-ремап `app.interrupt: Ctrl+X` меняет маршрут ввода; пустой массив отключает действие.
- [ ] Смоук в PTY: изображения kitty + поток ввода (в т.ч. внутри tmux с `wheel`-пресетом) без мусора в промпт-строке.

## Deep-dive

- [docs/research/tui-renderer/frame-history.md](frame-history.md) — ack/write порядок, в который вписаны synchronized output и kitty-плейсы.
- [docs/research/tui-renderer/width-model.md](width-model.md) — расчёт высоты плейсмента kitty по ширине терминала.

Возможные подсистемы 2-го уровня (план): `tui-renderer/kitty-protocol.md` (transmit/place/delete-команды, Unicode placeholders), `tui-renderer/probe-reassembly.md` (sentinel-owners, таймауты, fuzzing stdin).
