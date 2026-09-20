# STATE — Empryo port

Updated: 2026-09-20
Phase: E3 — Genome (index + prompt projection)
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
- Registry descriptors читаются из settings (`providers`/`models`), иначе fallback на `default_registry_config()`.
- `StreamingAgentRunner` исполняет spawn через тот же transport/credential resolver.
- Hub `r`/`x` шлют `ReviveAgent`/`StopAgent` без закрытия overlay.
- `titi-tools` содержит registry, approval tiers (`read`/`write`/`exec`) и режимы `always-ask`/`write`/`yolo`.
- Engine исполняет bounded tool loop: collect toolcall triplet → approve → invoke → replay tool messages, cap `max_tool_rounds`.
- Workspace tools: `read`/`write`/`edit`/`glob`/`grep`/`bash`, jailed to cwd.
- Tool calls/results пишутся в `TrajectoryRecorder`, если sink открыт.
- `LayeredCredentialSource`: process env → layered `.env` → `auth.db`.
- CLI создаёт session + `TrajectoryRecorder` на старте; UserMessage/ToolCall/ToolResult/TurnEnd пишутся на диск.
- `titi --headless` читает JSONL `{"command": EngineCommand}` со stdin и пишет `EngineEvent` в stdout.
- Engine шлёт `ToolApprovalNeeded` перед ожиданием `ApproveTool`.
- TUI открывает Approval overlay на exec-tier tool; Yes/Esc шлют `ApproveTool { approved }`.
- Session-close и tool-approval не смешиваются: `pending_close` остаётся отдельным от `pending_tool_approval`.
- Добавлен crate `titi-genome`: обход файлов (`scan`), парсер Rust/TS/Python (`parse`), граф + PageRank (`graph`), prompt-проекция (`project`).
- `scan` соблюдает `.gitignore` и `.empryoignore` (gitignore-семантика: `*` не пересекает `/`, `**` пересекает, `dir/` только каталоги), prune build/dot-каталогов, сорс-расширения, cap 1 MB/файл.
- `EngineConfig.genome_root`/`genome_limit` — индекс живёт в runtime и обновляется перед каждым turn; проекция уходит отдельным `Role::System` message перед user-prompt.
- `Genome::refresh` — incremental: re-parse только при смене size/mtime, исчезнувшие файлы выпадают, `RefreshStats { parsed, removed, total }`.
- Personalized rank: `TouchedSink` собирает пути из `read`/`write`/`edit` tool calls, `project_with` даёт им ×3 буст.
- CLI передаёт cwd как `genome_root`; `TITI_NO_GENOME=1` отключает.
- `cargo run -p titi-genome --example map -- [path] [limit]` печатает карту вручную.

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
- `loop` cancel + follow-up tests PASS.
- `StreamingAgentRunner` progress test PASS.
- Hub revive/stop overlay tests PASS.
- `project check` после config descriptors / hub commands / runner — PASS.
- `titi-tools` unit tests PASS.
- `titi-engine` tools tests: auto-approve read, exec waits for ApproveTool, round cap — PASS.
- `project check` после tool loop — PASS.
- `titi-tools` fs tests: roundtrip, glob/grep, jail escape — PASS.
- `project check` после workspace tools / trajectory / layered credentials — PASS.
- `session_trajectory_records_user_tools_and_turn_end` PASS.
- `engine_events_round_trip_as_jsonl` PASS.
- `project check` после session trajectory + headless JSONL — PASS.
- `exec_tool_waits_for_approval` ждёт `ToolApprovalNeeded` перед `ApproveTool` — PASS.
- `engine_events`: overlay Yes → `ToolApproval { approved: true }`; Esc не трогает session close — PASS.
- `overlays_paste` session-close gate — PASS.
- `titi-genome` unit tests: glob `*`/`**`/anchor/`dir/` семантика — PASS.
- `titi-genome` integration: Rust-граф + PageRank + проекция, `.empryoignore`, TS relative imports — PASS.
- `indexes_this_workspace`: реальный titi-репозиторий индексируется за 0.10s, `target/` отсечён, runtime.rs выше leaf-модуля — PASS.
- `genome_is_injected_as_system_message` / `no_genome_means_prompt_only`: mock transport получил ровно system+user (или только user) — PASS.
- `genome_refreshes_between_turns`: файл, созданный после старта, попадает в карту следующего turn — PASS.
- `titi-genome`: `refresh_reparses_only_changed_files`, `touched_files_are_boosted_in_projection`, `gitignore_anchoring_and_negation`, `hostile_file_names_cannot_forge_the_frame` — PASS.
- `titi-engine`: `genome_is_indexed_and_injected_as_system_message`, `no_genome_means_prompt_only`, `touched_file_leads_the_next_projection` — PASS.
- `cargo test --workspace` — 771 passed, 0 failed; `cargo fmt --check` чистый; `cargo clippy -p titi-genome -p titi-engine --all-targets` — 0 errors.
- Edge-тесты Genome: `zero_limit_still_emits_one_file`, `touched_path_outside_the_index_is_ignored`, `angle_bracket_names_cannot_forge_the_frame` — PASS.
- `queued_prompts_each_get_a_well_formed_frame`: два SubmitPrompt подряд → оба request несут закрытый `<genome>…</genome>` — PASS.
- `bcecd21 style: format workspace with rustfmt` — HEAD не проходил `cargo fmt --check` (498 диффов); теперь проходит.
- Реальный прогон: `example map` на titi — 110 файлов, 148 рёбер, `stream.rs:(→8)`, `width.rs:(→12)` наверху — PASS.

## DECISIONS

- GPUI — отдельный surface; terminal `Component` API не поднимается в engine.
- Fallback запрещён после visible content.
- Permanent 4xx не retry/fallback.
- Не подключать CLI к fake/empty resolver: сначала сделать provider registry с transport + credential.
- Документация и STATE обновляются после каждого milestone, чтобы другая модель могла продолжить без истории чата.

## RISKS

- Текущий `titi-providers::FallbackChain` one-shot; новый engine loop поддерживает ordered список самостоятельно. Позже объединить политики, не держать две расходящиеся реализации.
- `TransportResolver` возвращает `ResolvedModel`; `ProviderRegistry` реализует этот trait.
- Cancellation и follow-up покрыты engine tests; pending-stream stall watchdog ещё не отдельный test.
- Settings catalog читается, если `providers`/`models` валидны; иначе остаётся hardcoded default.
- Все Empryo fallback models используют один `subscriptions` provider, поэтому общий outage этого provider цепочка не переживёт.
- Research consolidation audit на `subscriptions/grok-4.6` завершён: `empryo-port` остаётся каноном; OMP — reference для TUI/tool UX; Hermes/Vellum — точечные источники идей.
- Genome знает Rust/TS/Python; прочие языки дают файл без рёбер (сознательно, до tree-sitter).
- `refresh` каждый turn заново обходит дерево (parse скипается по mtime, обход — нет) и пересчитывает PageRank целиком. На titi дёшево; на больших репо понадобится watcher + инкрементальный rank.
- Паника внутри `spawn_blocking` в `genome_system` превращается в `None` (`.ok().flatten()`) — turn продолжается без карты. Не покрыто тестом.
- Параллельные turn-ы делят один `Genome` под mutex: два `SubmitPrompt` подряд дают два `refresh` на одном индексе (покрыто тестом на форму фрейма, но не на гонку данных).
- Граф file-level, не symbol-level: `(→N)` считает файлы-импортёры, а не вызовы конкретного символа.

## NEXT

1. Genome: tree-sitter для остальных языков и symbol-level граф.
2. Restore/checkpoints и versioned RPC framing (остаток E2).
3. Background agents / file claims (E4).

## Verification baseline

```bash
cargo fmt --check
cargo test -p titi-engine
cargo clippy -p titi-engine --all-targets
cargo test --workspace
```
