# TUI-движок

Обзор терминального движка titi: контракт history/viewport, диффинг экрана, overlays, компонентная модель, модель ширины, capability-пробы и инлайн-графика. Подсистемы: [frame-history.md](frame-history.md), [width-model.md](width-model.md), [input-capabilities-graphics.md](input-capabilities-graphics.md).

## omp

omp — эталон TUI-движка в этом исследовании: отдельный пакет рендеринга (`packages/tui`) и интеграционный слой продукта (`packages/coding-agent`).

1. **Двухканальная модель кадра.** Продукт ставит `TerminalFrameProvider` через `TUI.setFrameProvider()`; на каждый кадр провайдер получает `ViewportSize` и возвращает `TerminalFramePlan { history?: HistoryBatch, viewport: string[] }`, где `HistoryBatch = { id: number; rows: string[]; kind?: "append" | "replay" }`. Финальность строки — решение приложения: renderer никогда не выводит history из того, что строка уехала за верх экрана (omp://tui-core-renderer).
2. **Handshake batch→ack.** TUI пишет каждый принятый batch ровно один раз и подтверждает монотонный `id` провайдеру; провайдер держит pending-batch до ack и не переиспользует/не переупорядочивает id. Это делает повторы и коалесцированные рендеры безопасными без сверки с scrollback терминала (omp://tui-core-renderer).
3. **Компонентный контракт.** `Component { render(width): readonly string[]; handleInput?; invalidate?; dispose? }` + отдельный `Focusable`; курсор передаётся через `CURSOR_MARKER` в отрендеренном тексте, а не через `getCursorPosition`. Неизменённый компонент обязан возвращать тот же array reference — на этом держится мемоизация контейнеров (omp://tui).
4. **Разделение владения.** `packages/tui` владеет терминальным lifecycle, вводом, фокусом, overlays, image-протоколами и курсором; продукт владеет порядком транскрипта, финальностью блоков и `TerminalFrameProvider` (omp://tui-runtime-internals).

## Hermes

Hermes — Python-агент с Node/Ink-фронтом; TUI существенно проще omp по контрактам, но даёт полезные UX-идеи.

1. **Ink-подход: alternate screen + дифф-рендер.** TUI — Node-сабпроцесс (Node ≥ 20), запускаемый из Python CLI; рендерит в alternate screen с дифференциальными обновлениями («no flicker when streaming, no scrollback clutter after you quit»), мгновенный первый кадр и неблокирующий ввод с очередью сообщений до готовности агента (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
2. **Overlays и живой switcher.** Модельный/сессионный пикеры, approval-промпты — модальные панели; `/sessions` (Ctrl+X) — живой переключатель нескольких TUI-сессий в одном процессе с mouse-кликами и `Ctrl+D` закрытием (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
3. **Мышь и статус-бар.** Пресеты mouse-tracking (`/mouse wheel|buttons|all` → `display.mouse_tracking`; `wheel` = 1000+1006, `buttons` = +1002, `all` = +1003); статус-бар в реальном времени (git-ветка с mtime-кешем, `⏱ 12s/3m 45s`, счётчик компрессий, бейдж YOLO) (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
4. **Скины вместо тем.** Визуал управляется YAML-скинами `~/.hermes/skins/*.yaml` с наследованием от `default`: секции `colors`, `spinner`, `branding`, `tool_prefix`; переключение `/skin` с live-preview (https://hermes-agent.nousresearch.com/docs/user-guide/features/skins).

Историю/финальность и width-модель Hermes не документирует — ближайшие аналоги: alternate-screen diffing (вместо history-batch) и требование «matched glyph widths» для индикаторов, чтобы статус-бар не джиттерил (https://hermes-agent.nousresearch.com/docs/user-guide/tui).

## Vellum

Vellum тему TUI-рендеринга не покрывает: материал описывает память, SOUL, проактивность, безопасность и каналы, но не терминальный вывод. Ближайшие аналоги: (1) мультиканальность — «один ассистент, одна память, каждый канал» (macOS, iOS, Web, Voice, Email, Telegram, Slack) — это модель «рендерер — просто один из каналов поверх общего ядра», что подтверждает разделение движка (titi-tui) и продукта (агентное ядро); (2) проактивность с уведомлениями «в правильный канал и без прерывания активного диалога» — аналог требования не трогать scrollback во время чтения пользователем (local://vellum-summary.md).

## Решение (одно/комбо)

Комбо с приматом omp. Берем контракт omp целиком: `HistoryBatch` + ack-handshake, финальность на стороне приложения, viewport-only diffing для обычных кадров, overlays как viewport-local, resize-политики `preserve|append|rebuild` — это единственная из трёх систем модель, которая корректно решает проблему «финализированная строка ушла в scrollback» без перепроверки терминала. У Hermes берём только UX-слой: живой switcher сессий, mouse-tracking-пресеты (особенно `wheel`-режим для tmux) и статус-бар с набором живых метрик. Vellum добавляет архитектурный принцип: рендерер — один из адаптеров канала, ядро агента о нём не знает (как headless/RPC-режим omp, где `hasUI === false`). Эффективность: диффинг только viewport-строк и reference-мемоизация компонентов дают O(changed rows) на кадр вместо полного перерийса.

## Rust-маппинг

Крейты: `titi-tui` — движок; `titi-cli` — интеграция (frame provider поверх агента). Новые зависимости в `[workspace.dependencies]`: `crossterm = "0.29"` (raw mode, alternate screen, event, ANSI-вывод), `unicode-width = "0.2"` (UAX#11 East Asian Width).

Эскиз ключевых типов в `titi-tui`:

```rust
// core.rs — компонентный контракт (аналог Component/Focusable из omp://tui)
pub trait Component {
    fn render(&mut self, width: u16) -> Cow<'_, [Line]>; // Line = String c ANSI
    fn handle_input(&mut self, ev: &InputEvent) -> Consume;
    fn invalidate(&mut self);
    fn dispose(&mut self) {}
}
pub trait Focusable: Component { fn set_focused(&mut self, focused: bool); }

// frame.rs — контракт кадра (аналог TerminalFrameProvider, omp://tui-core-renderer)
pub struct HistoryBatch { pub id: u64, pub rows: Vec<String>, pub kind: BatchKind }
pub enum BatchKind { Append, Replay }
pub struct FramePlan { pub history: Option<HistoryBatch>, pub viewport: Vec<String> }
pub trait FrameProvider {
    fn plan(&mut self, size: ViewportSize) -> FramePlan;
    fn acknowledge(&mut self, id: u64);
}

// renderer.rs — writer: diff viewport, писать history ровно один раз, ack после записи
pub struct Renderer<W: Write> { /* prev viewport rows, last_acked: u64, caps: Capabilities */ }
impl<W: Write> Renderer<W> {
    pub fn draw(&mut self, plan: FramePlan) -> io::Result<()>;   // synchronized output
    pub fn resize(&mut self, mode: ResizeScrollbackMode) -> io::Result<()>;
}
pub enum ResizeScrollbackMode { Preserve, Append, Rebuild }

// overlay.rs — overlays композитятся поверх viewport и никогда не становятся history
pub struct OverlayHost { /* stack, anchor: BottomCenter */ }
```

История/финальность живёт в `titi-cli` (`transcript.rs`: active/settled/committed блоки, `peek_finalized_batch(width, capacity)`), движок о ней знает только через `FramePlan`.

## Definition of Done

- [ ] `titi-tui` собирается; `Renderer::draw` пишет history-batch ровно один раз и вызывает `FrameProvider::acknowledge(id)` только после успешной записи (unit-тест с mock-провайдером и mock-writer фиксирует порядок и single-write).
- [ ] Обычный кадр не изменяет history: тест «два последовательных кадра с одинаковым viewport эмитят 0 байт ANSI-вывода, кроме synchronized-output обёртки».
- [ ] Viewport diffing: тест на матрицу «изменились строки 0, N-1 → записаны только они».
- [ ] Overlays: тест «показ/обновление/закрытие overlay не увеличивает счётчик записанных history-строк».
- [ ] Resize-политики: по тесту на каждый режим (`Preserve` — 0 replay-batch; `Append`/`Rebuild` — ровно один replay-batch с новым id, ack после записи).
- [ ] `Component::render` мемоизация: unchanged-компонент, возвращающий те же строки, не перерисовывается (reference/hashe-сравнение в тесте).
- [ ] Интеграционный smoke в `titi-cli`: сценарий «запуск → потоковый ответ → resize → выход» прогоняется в PTY без искажений (golden-сравнение снятого экрана).
- [ ] Ни один модуль `titi-tui` не зависит от `titi-providers`/`titi-tools` (проверка правилом в `Cargo.toml`/тестом графа зависимостей).

## Deep-dive

- [docs/research/tui-renderer/frame-history.md](frame-history.md) — history-batch + ack, viewport diffing, overlays, resize-политики, финальность.
- [docs/research/tui-renderer/width-model.md](width-model.md) — ANSI-aware UAX#11 ширина, обрезка, перенос, табы.
- [docs/research/tui-renderer/input-capabilities-graphics.md](input-capabilities-graphics.md) — capability-пробы, synchronized output, kitty graphics, image budget, ввод/фокус/keybindings.

Возможные подсистемы 2-го уровня (план, не написаны): `tui-renderer/component-contract.md` (жизненный цикл компонентов, фокус, CURSOR_MARKER), `tui-renderer/statusline-theming.md` (статус-бар и темы/скины).
