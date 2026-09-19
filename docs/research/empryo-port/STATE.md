# STATE — Empryo port

Updated: 2026-09-19
Phase: E0 — Engine protocol and bounded provider loop
Status: in-progress
Plan: `.empryo/plans/plan-211e3ec9-de18-487f-b75c-8430855aecd0.md`

## DONE

- Добавлен workspace crate `titi-engine`.
- Добавлен сериализуемый surface protocol: `EngineCommand`, `EngineEvent`, `TurnId`.
- Реализован async scheduler: один active turn, FIFO follow-ups, cancellation flag, switch-model для следующих turns.
- Реализованы retry только transient failures и ordered model fallback до visible content.
- Добавлены contract tests success/fallback/permanent failure.
- Настроены project-level Empryo task router, fallback chains и bounded goal loop.
- Surface protocol расширен lifecycle-командами и событиями агентов (`Spawn/Focus/Revive/Stop`, started/progress/status/finished).
- `titi-cli::App` отображает streaming answer, thinking, tool activity и subagent progress из `EngineEvent`.
- `/agents` открывает живой Agent Hub; roster обновляется engine-событиями.

## VERIFIED

- `cargo test -p titi-engine` — PASS, 3 integration tests.
- `cargo clippy` для `titi-engine` — PASS; warnings только в существующем `titi-providers`.
- `cargo test --workspace` — PASS после устранения двух environment/global-state зависимостей в CLI tests.
- `App` теперь дедуплицирует OSC 11 appearance per instance; parallel tests не вмешиваются через process-global theme state.
- Mouse selection test принимает truecolor и ANSI-256 background: оба режима являются корректным terminal output.
- `project check` — PASS после TUI engine-event integration.
- `engine_events` — 2 tests: rendering lifecycle и `/agents` live roster.

## DECISIONS

- GPUI — отдельный surface; terminal `Component` API не поднимается в engine.
- Fallback запрещён после visible content.
- Permanent 4xx не retry/fallback.
- Не подключать CLI к fake/empty resolver: сначала сделать provider registry с transport + credential.
- Документация и STATE обновляются после каждого milestone, чтобы другая модель могла продолжить без истории чата.

## RISKS

- Текущий `titi-providers::FallbackChain` one-shot; новый engine loop поддерживает ordered список самостоятельно. Позже объединить политики, не держать две расходящиеся реализации.
- `TransportResolver` пока возвращает только transport; реальный registry должен также вернуть credential и provider metadata.
- Cancellation signal существует, но отдельный pending-stream test ещё не добавлен.
- `titi-cli` всё ещё печатает delivered prompts через `eprintln!`; engine ещё не подключён.
- Все Empryo fallback models используют один `subscriptions` provider, поэтому общий outage этого provider цепочка не переживёт.
- Research consolidation audit на `subscriptions/grok-4.6` завершён: `empryo-port` остаётся каноном; OMP — reference для TUI/tool UX; Hermes/Vellum — точечные источники идей.

## NEXT

Реализовать provider registry перед CLI integration:

1. Ввести `ResolvedModel { model, transport, credential }` или эквивалент без утечки refresh token.
2. Читать descriptors из config, а не hardcode `model_choices()`.
3. Передавать resolved access material в `RequestCtx`.
4. Добавить tests: missing credential, unknown model, transient primary → backup с разными transports.
5. Затем заменить `init_provider`/`eprintln!` в `crates/titi-cli/src/main.rs` на реальный `Engine`.
6. Реализовать `AgentSupervisor`: команды Spawn/Focus/Revive/Stop сейчас явно возвращают `agent supervisor is not configured`; после подключения они должны порождать lifecycle events, которые TUI уже умеет показывать.

## Verification baseline

```bash
cargo fmt --check
cargo test -p titi-engine
cargo clippy -p titi-engine --all-targets
cargo test --workspace
```
