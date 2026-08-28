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

titi-tui: 348 тестов (333 unit + 15 integration); workspace: 536. Сборка 0 warnings; новые файлы clippy-clean.
Остаток волны 3: transcript/overlays-рендер в titi-cli (интеграция), snapshot-тесты slash в golden-файлы.

## Следующие шаги

1. Первый кадр (DoD): banner + status line before provider ready, input queued during init, time-to-first-frame < 150ms. Требует: tokio, crossterm, ratatui в titi-cli; integration test с mock provider + 2s delay.
2. Интеграция transcript в titi-cli: компонент Transcript, секции accordion, floating-alert.
3. PTY-smoke: kitty+resize+input, drag-select selection background.
4. Каждый шаг обновляет этот файл — точка возобновления.
