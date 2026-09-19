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
- Реализован `AgentSupervisor` с injectable `AgentRunner`, lifecycle events, progress, stop и revive state.
- `EngineRuntime::start_with_agents` подключает supervisor без зависимости engine от конкретного provider/tool implementation.
- `ProviderRegistry` резолвит model id → wire model + transport + credential; engine передаёт access material в `RequestCtx`.
- CLI стартует Tokio runtime, шлёт `SubmitPrompt`/`SwitchModel`/`FocusAgent` в engine и рисует `EngineEvent` в transcript.

## VERIFIED

- `cargo test -p titi-engine` — PASS, 3 integration tests.
- `cargo clippy` для `titi-engine` — PASS; warnings только в существующем `titi-providers`.
- `cargo test --workspace` — PASS после устранения двух environment/global-state зависимостей в CLI tests.
- `App` теперь дедуплицирует OSC 11 appearance per instance; parallel tests не вмешиваются через process-global theme state.
- Mouse selection test принимает truecolor и ANSI-256 background: оба режима являются корректным terminal output.
- `project check` — PASS после TUI engine-event integration.
- `engine_events` — 2 tests: rendering lifecycle и `/agents` live roster.
- `agents` — 2 integration tests: полный spawn lifecycle и остановка running agent.
- `project check` после AgentSupervisor — PASS.
- `registry` — 4 tests: resolve, missing credential, engine uses wire model+credential, fallback between providers.
- `project check` после CLI engine wiring — PASS.

## DECISIONS

- GPUI — отдельный surface; terminal `Component` API не поднимается в engine.
- Fallback запрещён после visible content.
- Permanent 4xx не retry/fallback.
- Не подключать CLI к fake/empty resolver: сначала сделать provider registry с transport + credential.
- Документация и STATE обновляются после каждого milestone, чтобы другая модель могла продолжить без истории чата.

## RISKS

- Текущий `titi-providers::FallbackChain` one-shot; новый engine loop поддерживает ordered список самостоятельно. Позже объединить политики, не держать две расходящиеся реализации.
- `TransportResolver` возвращает `ResolvedModel`; `ProviderRegistry` реализует этот trait.
- Cancellation signal существует, но отдельный pending-stream test ещё не добавлен.
- CLI descriptors пока hardcoded в `default_registry_config()`, не читаются из settings.
- Все Empryo fallback models используют один `subscriptions` provider, поэтому общий outage этого provider цепочка не переживёт.
- Research consolidation audit на `subscriptions/grok-4.6` завершён: `empryo-port` остаётся каноном; OMP — reference для TUI/tool UX; Hermes/Vellum — точечные источники идей.

## NEXT

1. Читать descriptors из config/settings, а не `default_registry_config()` в CLI.
2. Реализовать provider-backed `AgentRunner` на том же registry, tools и cancellation policy.
3. Добавить cancellation и queued-follow-up concurrency tests.
4. Подключить Hub revive/stop к `ReviveAgent`/`StopAgent`.

## Verification baseline

```bash
cargo fmt --check
cargo test -p titi-engine
cargo clippy -p titi-engine --all-targets
cargo test --workspace
```
