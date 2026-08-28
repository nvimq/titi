# Width model: ANSI-aware UAX#11 ширина, обрезка, перенос

Единая модель видимой ширины для измерения, нарезки, обрезки и переноса строк с ANSI-последовательностями. От неё зависит корректность диффа viewport, status-бара и рамок.

## omp

1. **Один набор хелперов на все операции.** `visibleWidth`, `truncateToWidth`, `sliceByColumn`, `wrapTextWithAnsi` разделяют одну ANSI-aware UAX#11 width-модель в `packages/tui/src/utils.ts`; измерение, нарезка, обрезка и перенос обязаны идти через них, чтобы escape-последовательности были zero-width и колоночные границы совпадали (omp://tui-core-renderer).
2. **Детали модели.** Printable ASCII — быстрый путь «одна ячейка на code unit»; не-ASCII — общая narrow-ambiguous модель (East Asian Ambiguous считаются узкими); табы — `DEFAULT_TAB_WIDTH`; OSC 66 sized spans дают заявленную ширину в ячейках; слишком широкие строки обрезаются до ширины viewport, и render hot path не должен падать на косметическом несоответствии ширины — кламп, не исключение (omp://tui-core-renderer).
3. **Нормализация ANSI на границах строк.** ANSI-состояние нормализуется на границах каждой строки, чтобы независимо обновляемые строки валидны сами по себе (иначе дифф-перерисовка одной строки ломает цвет последующих) (omp://tui-core-renderer).
4. **Контракт компонентов.** `render(width)` не должен намеренно превышать `width`; измерять надо `visibleWidth()`, обрезать/переносить — `truncateToWidth()`/`wrapTextWithAnsi()`, табы из внешнего ввода — `replaceTabs()`; renderer обрезает overwide не-image строки как last-resort (omp://tui).

## Hermes

Специальной width-модели Hermes не документирует — Ink/Node-стек опирается на библиотечную ширину без публичного контракта. Ближайшие аналоги:

1. **Matched glyph widths у индикаторов.** Busy-индикаторы статус-бара (`kaomoji | emoji | unicode | ascii`, `display.tui_status_indicator`) ship «with matched glyph widths so the rest of the status bar doesn't jitter on rotation» — то же требование стабильности ширины, что у omp для неизменных строк viewport (https://hermes-agent.nousresearch.com/docs/user-guide/tui).
2. **Выбор глифов по ширине символов.** Требование проверять тему на обоих symbol-пресетах, если «your theme depends on glyph width/appearance», и mouse-drag selection унифицированным фоном вместо SGR inverse (чтобы выделение корректно ложилось на широкие/нуль-ширинные ячейки) — косвенные подтверждения, что ширина глифов первоклассная забота рендера (https://hermes-agent.nousresearch.com/docs/user-guide/tui).

## Vellum

Vellum тему не покрывает. Ближайший аналог — канальная модель: один и тот же контент адаптируется под канал (терминал, Telegram, voice) поверх общего ядра, т.е. модель ширины — деталь терминального адаптера, изолированная от ядра; ядро оперирует семантическим контентом, а не ячейками экрана (local://vellum-summary.md).

## Решение (одно/комбо)

Единый width-модуль в `titi-tui` по образцу omp: одна структура `AnsiCellIter` (ANSI-токенизатор + `unicode-width`) под всеми четырьмя операциями, narrow-ambiguous по умолчанию, ASCII fast-path, таб-филлинг константой, OSC 66 — announced width. Критично для эффективности: диффинг viewport и мемоизация компонентов сравнивают строки байт-в-байт, поэтому все производители строк обязаны проходить через одни хелперы — иначе одинаковая логика с разными width-решениями ломает кеш. Rust даёт преимущество перед JS: `unicode-width` — та же таблица UAX#11, но zero-cost статические таблицы без рантайм-зависимости от Node. Escape-состояние нормализуем на границе строки (сброс SGR в конец), чтобы дифф не портил соседние строки.

## Rust-маппинг

Крейт `titi-tui`, модуль `width.rs`. Зависимости: `unicode-width = "0.2"` (`UnicodeWidthStr::width` — narrow-ambiguous по умолчанию; `width_cjk` — не используется, но доступен флагом), `crossterm::style` для генерации SGR.

```rust
// titi-tui/src/width.rs
pub const DEFAULT_TAB_WIDTH: usize = 8;
pub const TAB: &str = "\t";

/// Один проход: токены (Text(s), Escape(seq), Osc66 { cells }) —
/// escape zero-width, Osc66 даёт announced cells.
pub enum Span<'a> { Text(&'a str), Escape(&'a str), Sized { cells: u16 } }
pub fn spans(line: &str) -> impl Iterator<Item = Span<'_>>;

/// Видимая ширина в ячейках (UAX#11, ambiguous = narrow), escape = 0.
pub fn visible_width(line: &str) -> usize;

/// Обрезка до `width` ячеек с сохранением/закрытием активного SGR.
pub fn truncate_to_width(line: &str, width: usize) -> String;

/// Подстрока по колонкам [start, end) в ячейках, ANSI-состояние переносится.
pub fn slice_by_column(line: &str, start: usize, end: usize) -> String;

/// Перенос по ширине с сохранением escape; долгие слова клампятся.
pub fn wrap_text_with_ansi(line: &str, width: usize) -> Vec<String>;

/// Замена табов на пробелы до DEFAULT_TAB_WIDTH (санитизация внешнего ввода).
pub fn replace_tabs(line: &str) -> String;

/// Нормализация ANSI на границе строки: сброс активного SGR в конец.
pub fn normalize_ansi_boundary(line: &mut String);
```

Инварианты для тестов: `visible_width(truncate_to_width(s, w)) <= w`; `wrap_text_with_ansi(s, w)` строки все `<= w`; escape-последовательности никогда не увеличивают ширину; hot path не паникует (только кламп) — соответствует workspace-линту `unsafe_code = "forbid"` и `unwrap_used = "warn"` из корневого `Cargo.toml`.

## Definition of Done

- [ ] Таблица-тест `visible_width`: ASCII, CJK (2 ячейки), emoji, combining-акценты (0 ячеек), ambiguous (1 ячейка — narrow-модель), ANSI SGR (0), OSC 66 span (announced cells).
- [ ] Свойство-тест (proptest): для любых `s`, `w` — `visible_width(&truncate_to_width(s, w)) <= w`, паник невозможен.
- [ ] Тест `wrap_text_with_ansi`: перенесённые строки сохраняют цвет активного SGR и каждая `<= width`.
- [ ] Тест `slice_by_column`: нарезка CJK-строки по колонкам совпадает с тем, что терминал показывает в этих колонках.
- [ ] Тест `replace_tabs` и нормализации ANSI на границе: независимо перерисованная строка не наследует SGR соседа.
- [ ] ASCII fast-path покрыт бенчем (критерий: не медленнее чем `str::chars().count()` в константный фактор) — подтверждение требования эффективности.
- [ ] Все компоненты `titi-tui`/`titi-cli` используют только хелперы модуля (grep-тест CI: нет прямого `str::chars().count()` в render-путях).

## Deep-dive

- [docs/research/tui-renderer/frame-history.md](frame-history.md) — диффинг использует width-модель для стабилизации append-only головы.
- [docs/research/tui-renderer/input-capabilities-graphics.md](input-capabilities-graphics.md) — kitty-плейсменты резервируют строки по высоте, вычисленной этой же моделью.

Возможные подсистемы 2-го уровня (план): `tui-renderer/ansi-parser.md` (быстрый токенизатор, fuzzing на злонамеренные последовательности), `tui-renderer/markdown-wrapping.md` (рефлов markdown-блоков при resize).
