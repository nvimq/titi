# STATE — точка возобновления конвейера titi

Обновляется после каждого шага. Новая сессия начинает отсюда.

## Артефакты Research Map — все done (2026-08-27)

README карта + граф, PLAN.md, CONVEYOR.md (с цепочкой воркеров), scripts/check-research.sh (exit 0), 20 тем-доков + 9 подсистем.

## Реализация (тодо-конвейер 68 задач)

| Узел | Статус | Примечание |
|------|--------|------------|
| M0: titi-config (слои, merge, карантин, get/set/reset) | done | 8/8 тестов |
| M0: titi-secrets (dotenv-слои + auth.db SQLite, 0600) | done | 12/12 тестов |
| M1: session JSONL+FTS (titi-core::session) | done | 13/13 тестов |
| M4: titi-tui (width/history/viewport) | done | 30/30 тестов |
| Волна 1 интеграция | done | cargo test --workspace: 63 passed |
| Волна 2: trajectory, compaction, system-prompt-soul, provider-каркас | todo | по графу, после деплоя волны 1 |
| Волна 2 TUI: Component/overlays/keybindings/theme/kitty/resize | done | см. волну 3 TUI ниже |
| Fallback-пул titi (glm-5.3-flash + deepseek-flash, лимиты) | todo | фаза Providers |

## omp-ротация моделей (текущая сессия)

fallbackChains настроены: default = opencode-go/glm-5.3-flash → clinepass/glm-5.3 → opencode-go/deepseek-v4-flash → clinepass/deepseek-v4-flash → bai/glm-5.3-flash; revert cooldown-expiry; smol = bai/glm-5.3-flash → bai/qwen3.8-flash; task = clinepass/glm-5.3 → deepseek-v4-pro → qwen3.8-max.

## Волна 3 TUI (движок titi-tui) — 5/6

| Подсистема | Коммит | Статус |
|------------|--------|--------|
| 1. Ввод + keybindings (input.rs, keys.rs) | 8cd4138 | done |
| 2. Тема (theme/) | 837226a | done |
| 3. Kitty graphics (caps.rs, image.rs, input.rs) | 8cd4138 | done, 18+10+15 тестов |
| 4. Renderer/history/ack/resize (renderer.rs) | f416be6 | done, 16 тестов |
| 5. Agent UX: composer.rs, slash.rs, status.rs, panels.rs | d899189 | done, +62 теста |
| 6. Agent UX: overlay panels (SelectionPanel, ApprovalPanel, SessionSwitcher) + slash snapshot
| 7. Transcript markdown renderer + section visibility model + golden-тесты | b9d3b53 | done, +31 тест | | 3ad727b | done, +20 тестов |
| 8. mouse Off preset + /mouse parse | 17df97a | done, +2 теста |
| 9. Первый кадр в titi-cli: banner + status line до ready провайдера, queue ввода во время init, time-to-first-frame < 150ms | 0742f0a | done, +2 интеграционных теста (mock provider, 2s delay) |
| 10. Transcript в titi-cli: компонент Transcript (accordion-секции, /details, floating-alert backstop) | f55896f | done, +6 unit (titi-tui) + 4 интеграционных (titi-cli) |
| 11. Mouse drag-select: Selection model, SGR-mouse decoding (Drag/ScrollUp/Down), selection background, SGR-mouse integration test, /mouse + --mouse пресеты, персист display.mouse_tracking | 86255c0 | done, +9 unit (selection) + 3 input (Drag/Scroll) + 3 интеграционных (mouse_selection) + 3 App/config (titi-cli) |
| 12. Bracketed paste + overlay-панели (DoD): `Event::Paste` — вставка одним блоком, inline-collapse >6 строк, `[Image #N]` аттачменты; `Ctrl+X` session switcher, `/model`+`Ctrl+M` model picker, approval-gated close сессии (удаление JSONL только по Yes), Esc везде cancel-без-удаления | f4f4836 | done, +1 unit (composer, счётчик аттачментов) + 10 интеграционных (overlays_paste) |

titi-cli: FirstFrame core (banner, StatusLine Starting→Ready, очередь ввода, замер ttff) + App (FirstFrame + Transcript + theme) + бинарник на crossterm (raw mode, alternate screen, event loop). Transcript: секции thinking/tools expanded, subagents collapsed, activity hidden; `/details <section> <mode>`; backstop-алерт при all_hidden. Реализовано на std threads, БЕЗ tokio/ratatui — хватило crossterm.
titi-tui: 392 тестов (374 unit + 18 integration); titi-cli: 30 интеграционных; workspace: 598. Сборка 0 warnings; новые файлы clippy-clean.

## Slash DoD (2026-08-28)

- [x] Builtin names reserved (help, details, model, sessions, mouse)
- [x] `/unknown-xyz` → Passthrough (LLM as text)
- [x] `$1`/`$ARGUMENTS` expand in file templates (unit-tested in slash.rs)
- [x] Floating autocomplete panel (CompletionPanel, 6 unit + 11 integration tests)
- [x] Tab inserts highlighted command name, Esc hides, arrows navigate
- [x] Enter dispatches via Route::Builtin / Expanded / Passthrough
- [x] Snapshot test for route + complete (slash_snapshot.rs, 2 passed)
- [x] TopCenter anchor added to overlay::Anchor for floating panel placement
- [x] PTY-smoke: `/m` → panel shows `/model  Switch the active model`, Enter → model picker opens

3. ✅ Slash-команды (DoD): реестр `SlashRegistry` (builtin резерв, file-команды с `$1`/`$@`/`$ARGUMENTS`, Passthrough для `/unknown`); floating `CompletionPanel` (non-modal: Tab вставляет имя, Esc прячет, стрелки двигают подсветку); Enter диспатчит через `Route` (Builtin: help/details/model/sessions/mouse; Expanded: submit развёрнутого шаблона; Passthrough: submit как текст). Снапшот-тест route+complete; 6 unit (панель) + 11 интеграционных (titi-cli). `Anchor::TopCenter` добавлен для floating-размещения.
4. ⏳ Очередь (DoD): во время стрима `Steer`/`FollowUp` ставятся в очередь, `Alt+Up` возвращает последнее в редактор, Esc снимает подсветку не удаляя (тест на `Composer::queue`).
5. PTY-smoke: full terminal test — kitty+resize+input, drag-select selection background, `/mouse` switching.
6. Каждый шаг обновляет этот файл — точка возобновления.
