# Empryo → titi: функциональный порт на Rust + GPUI

## Цель

`titi` переносит продуктовую модель Empryo, а не его Electron-реализацию: один UI-независимый engine обслуживает TUI, headless/RPC и нативный desktop на Zed GPUI. Работа ведётся вертикальными срезами с проверяемым Definition of Done.

## Источники текущего Empryo

Актуальная документация проверена 2026-09-19:

- архитектура: https://empryo.com/docs/concepts/architecture
- goal loop: https://empryo.com/docs/agents/goal-loop
- model fallback: https://empryo.com/docs/recipes/model-fallback
- task router: https://empryo.com/docs/recipes/task-router
- desktop: https://empryo.com/docs/surfaces/desktop
- workbench: https://empryo.com/docs/surfaces/workbench
- Genome: https://empryo.com/docs/concepts/genome

Документация Empryo меняется быстро. Перед реализацией нового subsystem повторно читать точную страницу, а дату и решения фиксировать здесь.

## Неподвижные архитектурные правила

1. `titi-engine` не зависит от crossterm, GPUI, Monaco/WebView или конкретного UI.
2. Все surfaces используют один сериализуемый protocol: `EngineCommand` → `EngineEvent`.
3. TUI ANSI `Component` остаётся внутренним API `titi-tui`; GPUI не эмулирует строки терминала.
4. Provider transport нормализует wire events, engine владеет turn lifecycle, retries, fallback, cancellation и tool rounds.
5. Retry разрешён только для transient failures. После видимого delta turn не переигрывается на другой модели.
6. Auth/validation/unknown-model ошибки не повторяются. `429`, `408`, `5xx`, timeout и connection failures повторяются с backoff, затем переключают модель.
7. Любой автономный цикл bounded: iteration cap, token cap, cancellation, oscillation/stuck detection.
8. Один goal = один проверяемый milestone. Цель «закончить весь проект» запрещена: она не имеет сходящегося judge contract.
9. Перед следующим milestone `cargo test --workspace` должен быть зелёным или известный baseline failure явно записан в STATE.
10. После каждого логического этапа обновляются этот README и `STATE.md`: решения, проверки, следующий точный шаг, риски.

## Целевая карта crates

```text
titi-config       layered settings, profiles, model/provider descriptors
titi-secrets      credential actor/store; refresh material не попадает в model layer
titi-providers    HTTP/SSE transports и normalized StreamEvent
titi-engine       agent loop, commands/events, retries, fallback, tools, compaction
titi-core         sessions, context, trajectory и доменные структуры
titi-tools        tool registry, approvals, execution policies
titi-genome       dependency/symbol/git graph, ranking, live indexing (позже)
titi-agents       dispatch/shared bus/claims/steering (позже)
titi-tui + cli    terminal surface
titi-desktop      Zed GPUI workbench surface (после engine/headless DoD)
```

## Engine contract v0

Команды: submit/follow-up, cancel, switch model, approve tool, shutdown.

События: turn start, text/thinking delta, tool start/finish, model switch, finish, failure, cancellation.

Инварианты:

- `TurnId` коррелирует весь stream;
- surface никогда не вызывает provider напрямую;
- fallback происходит только до user-visible content;
- follow-up во время активного turn ставится в FIFO;
- `Cancel` имеет приоритет над retry/fallback;
- headless/RPC сериализует тот же protocol без отдельной модели состояния.

## Автономный loop для разработки

Empryo используется как внешний supervisor разработки titi. Лучший режим — milestone goal loop:

1. Coder реализует одну цель.
2. Бесплатные гейты: format, typecheck, targeted tests, workspace tests, runtime smoke.
3. Fresh reviewer читает текущее repo-состояние, а не рассказ coder-а.
4. `FAIL/PARTIAL` возвращается следующему coder round.
5. Повтор одинаковой причины дважды = oscillation; остановка и re-plan.
6. На поздних rounds меняется reviewer, затем coder; нельзя бесконечно повторять тот же prompt той же модели.
7. Provider failure обрабатывает model fallback отдельно от goal-loop judge logic.

Текущая project-конфигурация Empryo:

- основной model: `subscriptions/gpt-5.6-sol`;
- cheap exploration/semantic: `subscriptions/gpt-5.6-luna`;
- code agent: `subscriptions/cursor/gpt-5.3-codex`;
- verify: `subscriptions/gpt-5.6-terra`;
- goal reviewer: `subscriptions/cursor/claude-sonnet-5-thinking-high`;
- compact: `subscriptions/cursor/gemini-3.6-flash`;
- transient retries: 2; goal iterations: 8; token cap: 2,000,000; runtime probe: on; stand-in: conservative.

Все доступные модели сейчас идут через provider `subscriptions`; поэтому цепочки меняют model/backend, но не credential provider. Для настоящей provider diversity потребуется подключить второй независимый provider/key.

## Milestones и DoD

### E0 — Engine protocol и bounded provider loop

- [x] Новый `titi-engine` workspace crate.
- [x] `EngineCommand`, `EngineEvent`, `TurnId`.
- [x] Async runtime, FIFO follow-ups, cancellation signal.
- [x] Per-model transient retry и ordered fallback.
- [x] Контрактные tests: stream success, transient fallback, permanent stop.
- [x] Provider registry возвращает transport + credential + descriptor для model id.
- [x] CLI переводит submit и stream rendering на engine.
- [ ] Cancellation и queued-follow-up имеют отдельные concurrency tests.

DoD: реальный prompt проходит TUI → engine → configured provider → streaming transcript; Ctrl+X отменяет turn; mock `429` переключает модель; `401` не переключает.

### E1 — Tool loop

Tool registry, schemas, approval tiers, bounded tool rounds, tool-result replay, trajectory.

### E2 — Session/headless

JSONL/SQLite persistence, restore, checkpoints, JSON/events RPC, stable exit codes.

### E3 — Genome

37-language parsing по мере необходимости, import/symbol graph, PageRank, git co-change, incremental updates, prompt projection.

### E4 — Agents

Background dispatch, shared read cache, per-file write claims, findings bus, steering, fresh-context reviewer.

Готовый foundation: engine protocol содержит lifecycle-команды/события агентов, TUI показывает progress/thinking/tools и живой roster через `/agents`, а `AgentSupervisor` исполняет spawn/stop/revive через injectable `AgentRunner`. Следующий слой — provider-backed runner, shared tools и file claims.

### E5 — GPUI desktop

Workbench destinations: Work, Files, Changes, Genome, My tools, Costs, Settings, Help. GPUI читает только engine/storage APIs.

## Как продолжить в новой сессии

1. Прочитать `docs/research/empryo-port/STATE.md`.
2. Проверить `git diff` и не откатывать незнакомые изменения.
3. Запустить команду `Verification baseline` из STATE.
4. Взять только `NEXT`, не перескакивать к GPUI.
5. После изменения обновить `DONE`, `VERIFIED`, `DECISIONS`, `RISKS`, `NEXT`.
6. Если решение меняет protocol или crate boundaries — сначала обновить этот README.
