# Frame history: history-batch + ack handshake, viewport diffing, resize

Как терминальный движок разделяет неизменяемую историю (scrollback) и мутабельный viewport, кто решает финальность строки и что происходит при resize.

## omp

1. **HistoryBatch и финальность решает приложение.** Провайдер возвращает `TerminalFramePlan { history?: { id: number; rows: readonly string[]; kind?: "append" | "replay" }, viewport }`. `append`-batch содержит финализированные строки или стабильную append-only голову; `replay` — полный логический ledger. Финальность — «an application decision, never an inference from a row crossing the top of the terminal»; renderer никогда не сравнивает новый транскрипт со scrollback терминала (omp://tui-core-renderer).
2. **Ack-handshake.** TUI пишет каждый принятый batch ровно один раз и подтверждает монотонный `id` провайдеру после того, как запись принята in-process; провайдер удерживает pending-batch до ack и предлагает тот же id повторно, пока `acknowledgeFinalizedBatch()` не succeeded (omp://tui-core-renderer, omp://tui-runtime-internals).
3. **Три состояния блока.** `TranscriptContainer` держит блоки как active (мутабельный, viewport-resident) → settled (финализирован, но живой: перерендеривается на текущей ширине каждый кадр, поэтому рефловится на resize) → committed (подтверждён writer'ом, выгружен из кешей). `peekFinalizedBatch(width, capacity)` уводит «кратчайший settled-префикс, чтобы живой хвост влез в остаток», и останавливается на первом active-блоке; пока экран не переполнен — ничего не уходит, submitted message виден сразу (omp://tui-runtime-internals).
4. **Viewport diffing и overlays.** Viewport — полная заменяемая картина кадра: нормализация, width-fit, композит overlays, эмиссия только изменившихся строк. Overlays — screen-coordinate контент поверх viewport, никогда не history; показ/обновление/закрытие перекрашивает только viewport. Viewport-only кадры не могут создать history (omp://tui-core-renderer, omp://tui-runtime-internals).
5. **Resize-политики.** Во время ресайза TUI занимает alternate buffer (history-offers там не подтверждаются), после тихого окна восстанавливает normal buffer и восстанавливает якорь через DSR (CSI 6n) round-trip: каждый кадр паркует hardware-курсор на известный offset, формула якоря `min(reported − parkOffset, height − staleReflowedRows)`, таймаут CPR 200 мс (omp://tui-runtime-internals). Затем применяется `ResizeScrollbackMode`: `rebuild` — очистить native history (ED3) и replay транскрипта с новой шириной под свежими id; `append` — history сохраняется, replay идёт ниже неё; `preserve` — только перекраска viewport. Raw TUI по умолчанию `preserve` (env `PI_TUI_RESIZE_SCROLLBACK`), coding agent — `rebuild` (omp://tui-core-renderer).
6. **Деструктивный reset — только жест.** `resetDisplay()` (session replacement, Ctrl+L, Alt+L `app.display.reset`) очищает history и переоферт финализированный префикс под новыми id; обычные кадры, анимация и финализация тулов не могут его вызвать (omp://tui-runtime-internals, omp://keybindings).

## Hermes

Hermes тему history/finality не покрывает — Ink-рендер работает в alternate screen и не имеет эквивалента batch/ack-контракта. Ближайшие аналоги:

1. **Alternate-screen дифф-рендеринг.** «Alternate-screen rendering — differential updates mean no flicker when streaming, no scrollback clutter after you quit» — т.е. Hermes вообще не пишет историю в scrollback терминала: весь экран мутабельный, скролл-история живёт внутри приложения. Это упрощает движок (нет handshake) ценой невозможности native-скролла/поиска терминала (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
2. **Мгновенный первый кадр и неблокирующая очередь.** «Instant first frame — the banner paints before the app finishes loading» и сообщения встают в очередь до готовности агента — аналог omp-инварианта «submitted message виден сразу» (`renderNow()` перед dispatch; omp://tui-runtime-internals).
3. **Живые сессии в одном процессе.** `/sessions` (Ctrl+X) переключает live TUI-сессии поверх одного рендера — ближайший аналог переключения «продуктового состояния», при котором в omp требуется `resetDisplay()` с переофером истории (https://hermes-agent.nousresearch.com/docs/user-guide/tui).

## Vellum

Vellum тему не покрывает (материал — память/SOUL/безопасность/каналы). Ближайший аналог: канальная модель — «Один ассистент, одна память, каждый канал»: у Vellum вывод адаптируется под канал (терминал vs Telegram vs voice) поверх общего ядра, что соответствует разделению «history-политика — свойство интерфейсного адаптера, а не ядра агента». Плюс принцип проактивности «уведомления … не прерывают активный диалог» — тот же инвариант, что «renderer never probes the user's scroll position»: не мешать пользователю, читающему историю (local://vellum-summary.md).

## Решение (одно/комбо)

Берём модель omp полностью: inline-режим (не alternate screen) с history-batch + ack, потому что он сохраняет native scrollback (поиск, копирование, мышь терминала) и при этом корректен при рестриме/реплеях — единственная из трёх систем модель, где повторный кадр и ресайз не порождают дубли в истории. Для titi-cli по умолчанию `Rebuild` (как coding agent: консистентная ширина всей истории после ресайза), для raw-TUI-демо — `Preserve` (дешевле: ноль replay-трафика). Overlays — viewport-only; live-switcher сессий из Hermes реализуем как `resetDisplay()`-транзакцию (reset-and-reoffer), не как отдельный механизм. Эффективность: ack-handshake позволяет провайдеру дропать кеши committed-блоков сразу после подтверждения — память O(live tail), а не O(вся история).

## Rust-маппинг

Крейт `titi-tui` (движок) и `titi-cli` (провайдер). Зависимости: `crossterm` (queue/flush, alternate screen, `crossterm::cursor::MoveTo`, `EnableMouseCapture`), `unicode-width` для width-fit.

```rust
// titi-tui/src/frame.rs
pub struct HistoryBatch { pub id: u64, pub rows: Vec<String>, pub kind: BatchKind }
pub enum BatchKind { Append, Replay }
pub struct FramePlan { pub history: Option<HistoryBatch>, pub viewport: Vec<String> }

pub trait FrameProvider {
    fn plan(&mut self, size: (u16, u16)) -> FramePlan;
    /// Вызывается Renderer'ом только после успешной записи batch.
    fn acknowledge(&mut self, id: u64);
}

// titi-tui/src/renderer.rs
pub enum ResizeScrollbackMode { Preserve, Append, Rebuild }
pub struct Renderer<W: Write> {
    out: W,
    prev_viewport: Vec<String>,   // для диффа
    last_acked: Option<u64>,      // никогда не предлагает/не подтверждает дважды
    resize_mode: ResizeScrollbackMode,
}
impl<W: Write> Renderer<W> {
    /// 1) append unacked history; 2) композит overlays; 3) diff viewport —
    /// перерисовать только изменившиеся строки; 4) cursor park; 5) ack.
    pub fn draw(&mut self, plan: FramePlan) -> io::Result<()>;
    /// Alternate-buffer на время ресайза, DSR-якорь, затем политика mode.
    pub fn on_resize(&mut self, new: (u16, u16)) -> io::Result<()>;
    /// Деструктивный reset: ED3 + reset-and-reoffer (только user gesture).
    pub fn reset_display(&mut self) -> io::Result<()>;
}

// titi-tui/src/overlay.rs
pub struct OverlayHandle(/* id */);
impl Renderer<W> { pub fn show_overlay(&mut self, c: Box<dyn Component>, anchor: Anchor) -> OverlayHandle; }

// titi-cli/src/transcript.rs — финальность здесь, не в движке
pub enum BlockState { Active, Settled, Committed }
impl Transcript {
    /// Кратчайший settled-префикс, освобождающий `capacity` строк на ширине `width`.
    pub fn peek_finalized_batch(&mut self, width: u16, capacity: u16) -> Option<HistoryBatch>;
}
```

Тесты: golden-файлы последовательностей ANSI для diffing/resize (крейт `goldie`-подобный подход или инлайн-строки).

## Definition of Done

- [ ] Тест handshake: провайдер возвращает batch id=1; при повторном кадре до ack тот же id не пишется второй раз; после `acknowledge(1)` следующий batch id=2 пишется.
- [ ] Тест диффа: viewport из 10 строк, изменились строки 3 и 9 → writer-лог содержит ровно две перерисовки (плюс обёртка synchronized output).
- [ ] Тест «viewport-only кадр не пишет history»: кадр с `history: None` не эмитит ни одной строки в scrollback.
- [ ] Тест overlay: показ и закрытие overlay не меняют счётчик записанных history-строк; строки overlay удаляются из кадра при закрытии.
- [ ] Тесты resize: `Preserve` — 0 history-batch; `Append` — один replay-batch ниже существующей истории; `Rebuild` — ED3 + один replay-batch; во всех случаях ack строго после записи.
- [ ] Тест «финальность не выводится движком»: блок, пересёкший верх viewport без финализации провайдером, не попадает в history.
- [ ] Тест reset: `reset_display` переоферит уже подтверждённый префикс под новыми монотонными id.
- [ ] Смоук в PTY (titi-cli): «10 потоковых ответов → resize 80→120 колонок» — на экране ровно одна копия каждой строки после стабилизации.

## Deep-dive

- [docs/research/tui-renderer/width-model.md](width-model.md) — от width-fit зависит, что считается «той же строкой» при диффе.
- [docs/research/tui-renderer/input-capabilities-graphics.md](input-capabilities-graphics.md) — DSR/CPR round-trip и synchronized output реализуются на слое capability-проб.

Возможные подсистемы 2-го уровня (план): `tui-renderer/resize-anchor.md` (формула якоря, мультиплексеры, height-shrink push), `tui-renderer/replay-transactions.md` (bottom-first replay, нумерация id).
