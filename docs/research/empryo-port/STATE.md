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
- **Compaction подключена end-to-end** — до этого механика была, а `Summarizer` существовал только в тестах, так что длинный turn рос безгранично. Добавлены `StructuredSummarizer` (model-free: `SnapCompact`/`Handoff` дают структурный дайджест, `Remote`/`Soft` возвращают `StrategyFailed` и цепочка падает на следующие), `titi-engine::compaction::compact` (порог → цель → дайджест вместо префикса, хвост никогда не начинается с tool-результата), `EngineEvent::Compacted`, `EventKind::Compaction` в траектории, `EngineConfig.context_window` + `compaction`, и окно берётся из `ModelDescriptor.context_window`.
- **Выход по двойному Ctrl+C**: одиночный нажатие теперь только вооружает выход (`EXIT_CONFIRM_WINDOW` = 2с, подсказка «press Ctrl+C again to exit»), любая другая клавиша снимает вооружение. Активный turn по-прежнему отменяется одним Ctrl+C.
- **FIX**: `/pause` обещал «pause the agent», но только показывал модалку — агент продолжал стримить за ней. Теперь `/pause` возвращает `SubmitEffect::Pause`, main шлёт `Cancel` (движок не умеет suspend, поэтому честная остановка — отмена на текущей границе), модалка держит ввод и говорит «agent stopped, input held».
- **Recap**: `titi-tui::recap` — панель сворачиваемых секций (Session / Turns / Tools / Files / Problems / Trajectory); `titi-cli::recap::build` собирает их из session store + trajectory (счётчики тулов, суммарные и средние длительности, падения, тронутые файлы, промпты). Открывается `/recap`, `Ctrl+O` внутри панели раскрывает/сворачивает все секции сразу, вне панели `Ctrl+O` (действие `app.details.toggleAll`) раскрывает/сворачивает все секции транскрипта — то же значение, что у `Ctrl+O` в терминале Empryo («expand or collapse all code and reasoning blocks»).
- **FIX**: отменить turn из TUI было НЕЧЕМ — `EngineCommand::Cancel` не отправлял ни один путь, `app.interrupt` (Ctrl+C) всегда выходил из приложения. Теперь Ctrl+C при активном turn возвращает `Dispatch::Cancel` (main шлёт `Cancel`), а в покое — `Exit`, как и обещает описание действия «Interrupt / exit». DoD в README исправлен: он обещал Ctrl+X, который открывает session switcher.
- **FIX**: неизвестный флаг молча игнорировался — `titi --hedless` запускал полноэкранный TUI и выглядел как зависание. Добавлены `--help`/`-h` и отказ с usage на `-*`; exit code 2.
- **FIX**: Ctrl+N («новая сессия») только писал в stderr — новая сессия не создавалась, движок и лог оставались на прежней. Теперь `app::new_session` создаёт пустую сессию в store, движок получает пустую историю, лог и view переключаются (тот же ход из трёх шагов, что и у switch).
- **FIX**: `set_alert` рисовался только когда скрыты ВСЕ секции, а по умолчанию thinking/tools развёрнуты — значит `error: …` от `Failed`, `cancelled`, `session: …`, `checkpoint: …`, `rewound …` не были видны вообще. Отказ провайдера выглядел как «ничего не произошло». Alert теперь рисуется всегда: последней строкой над композером, а при полностью скрытом транскрипте — как и раньше, единственным содержимым.
- **FIX**: переключение сессии было фикцией — `SessionSwitched` только делал `eprintln!` (в alternate screen его не видно), движок и лог оставались на прежней сессии. Теперь переключение — три действия: `RestoreHistory` в движок, переоткрытие `SessionLog` на новый файл, `App::switch_to_session` очищает отрисованный разговор и меняет id.
- **FIX**: `/rewind` обрезал файл сессии, но движок продолжал слать провайдеру прежнюю историю — откат был видимостью. Добавлены `EngineCommand::RestoreHistory { messages }` (движок заменяет `restored_messages`) и `SubmitEffect::Rewind`: TUI после успешного отката пересобирает историю через `app::session_history` и отправляет её в движок. Старт и откат теперь строят историю одной функцией, так что расходиться нечему.
- **FIX**: `titi --headless` без явного `--approval` вис навсегда: write-тул ждал `ApproveTool`, которого скрипт не шлёт. Теперь `--approval <always-ask|write|yolo>` (и `parse_approval` с явной ошибкой на опечатку); по умолчанию `write`, поверхность без approve-панели должна сказать это сама. Подтверждено: `--approval yolo` записал файл без approve-события.
- **FIX**: `TouchedSet` теперь bounded (`TOUCHED_CAPACITY = 64`, порядок + дедуп, самое старое вытесняется) — раньше множество росло безгранично, и буст ×3 переставал что-либо значить после ~сотни файлов.
- **FIX (critical)**: транскрипт вообще не попадал в session store — файл сессии оставался 0 байт, `restore_latest` всегда возвращал пустой разговор, `/checkpoint` фиксировал 0 записей, `/rewind` нечего было обрезать. Разговор уходил только в trajectory (другой файл). Теперь `App` ставит в очередь `(Role, text)` (prompt при отправке, ответ на `TurnFinished`), поверхность дренирует очередь в `SessionLog`: TUI — после каждого батча событий, headless — в цикле `run`. Подтверждено вживую: run 1 → `msgs=1 roles=user`, файл 276 байт; run 2 → `msgs=3 roles=user,assistant,user`, 598 байт.
- `MAX_RESTORED_MESSAGES = 40`: восстанавливается только хвост разговора, чтобы старая история не вытесняла карту workspace.
- `SessionStore::restore_latest` → id + разговор по пути к текущему leaf (брошенные fork-ветки не попадают).
- `entries_to_messages` конвертирует `Entry` → `titi_providers::ChatMessage`; `EngineConfig.restored_messages` подставляется перед user-prompt.
- CLI при старте восстанавливает последнюю сессию, иначе создаёт новую.
- Checkpoints: `checkpoint`/`checkpoints`/`rewind` поверх sidecar `<id>.checkpoints.jsonl`; rewind обрезает session JSONL до отмеченной точки, откатывает leaf, удаляет этот checkpoint и последующие и перестраивает FTS (`SessionIndex::reindex_session`).
- TUI: `/checkpoint`, `/checkpoints`, `/rewind [n]`; `App::set_session_id` привязывает живой session id, `start_engine` его возвращает.
- Headless RPC версионирован: `headless::RPC_PROTOCOL`, `HeadlessFrame { v, command }`, `decode` отклоняет чужую версию, `run` пишет `{"ready":true,"protocol":1}` первым делом.
- E4: `Claims` (per-file write claims: нормализация пути, release только владельцем, `release_all`), `Findings` (ordered bounded bus с курсором `drain_since`), `Steering` (bounded queue, drain на границе шага).
- Tool loop берёт claim на `write`/`edit` и отпускает после вызова; чужой claim → error-результат без вызова handler-а и без касания диска.
- `EngineCommand::Steer` + `App::turn_active`: submit во время активного turn шлёт `Steer`, а не новый turn.
- Subagent делит с runtime таблицу claims и findings bus; `AgentContext::finding/claim/release_claims`; `stop` и завершение агента освобождают его файлы.
- `titi-tools::ReadCache` — LRU-bounded кэш содержимого файлов, ключ `(size, mtime)`; `workspace_tools` даёт один кэш на все тулы, `write`/`edit` его инвалидируют. Удалённый файл из кэша не отдаётся.
- **FIX (critical)**: `ToolcallDelta.json` — это ФРАГМЕНТ, а не накопленный буфер. OpenAI-декодер слал весь буфер на каждый chunk + финальный полный JSON на close, а engine-коллектор конкатенирует → аргументы тулов получались мусорными (`{"path":{"path":{"`), т.е. ЛЮБОЙ вызов тула с аргументами через реальный OpenAI-совместимый провайдер ломался. Anthropic слал фрагменты правильно. Теперь openai шлёт только новый фрагмент и на close — только `ToolcallEnd`.
- Прогон реального сквозного пути (локальный OpenAI-совместимый SSE-сервер + `TITI_AGENT_DIR`): `settings → registry → HTTP → SSE → decoder → engine → tool loop → handler → replay → ответ` — работает; `read Cargo.toml` вернул 1056 байт.
- `titi --set-key <provider> <key>` пишет в `<agent_dir>/auth.db` — ту самую ступень credential ladder, которая была недостижима без env/.env; `--list-keys` показывает провайдеров без токенов.
- Сквозная проверка ключа: без ключа turn падает с `requires a credential`; после `--set-key` запрос уходит с `Authorization: Bearer <key>` и turn стримится до конца.
- Записан `~/.titi/agent/config.yml` с реальным провайдером пользователя: `opencode-go` → `https://opencode.ai/zen/go/v1`, модели `glm-5.3-flash`, `deepseek-v4-flash`, `grok-4.6`, `grok-4.5`.
- E3 углублён: symbol-level граф. Ребро = импорт **или** упоминание символа, определённого в другом файле; один и тот же rank и `(→N)` для обоих типов. Проекция печатает `+Name (users)` — сколько файлов опирается на символ.\n- Языков стало 11: Rust, TS/JS, Python, Go, Java, Kotlin, C/C++, C#, Ruby, Swift, PHP. Список расширений один (`parse::EXTENSIONS`), `scan::is_source` спрашивает его — расхождение двух списков больше невозможно (именно оно однажды молча выключило PHP).\n- Точность name-only резолва: refs собираются только с мест использования (вызов `name(`, `::name`, `TypeName`), а имя, экспортируемое более чем одним файлом, рёбер не даёт. На titi это 3023 → 681 ребро при 122 файлах: `path (33)` и `is_empty (64)` перестали быть «зависимостями».\n- Дыры закрыты: `ToolAgentRunner` даёт сабагенту свой bounded tool loop; он делит с runtime claims, touched-множество и read cache (`EngineConfig.workspace_root`, `agent_model`, `agent_writes`, `agent_rounds`, `read_cache`).
- Безопасность сабагента: его реестр сам и есть политика. По умолчанию `tools.retain_tiers(&[Read])` — ничег​о write/exec не зарегистрировано, поэтому ни один вызов не может ждать approve, которого нет на этой поверхности. `agent_writes = true` включает write, но claim-конфликт всё равно отказывает.
- Паника или ошибка в построении карты деградирует в «нет карты в этом turn» через `run_off_thread` (JoinError и Err обрабатываются одинаково) — покрыто тестом с реальной паникой.
- `titi-engine::review` — fresh-context reviewer:"}] `Verdict` (PASS/FAIL/PARTIAL, exit 0/3/1), `ReviewRequest::prompt` собирает brief + goal + evidence, `AgentReviewer` гоняет один turn через `AgentRunner` с `AgentContext::detached()`.
- Verdict читается только с первой непустой строки: эхо brief-а, отговорка или токен на второй строке дают `PARTIAL`.

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
- `titi-core` session: `rewind_keeps_entries_up_to_the_checkpoint`, `rewind_drops_that_checkpoint_and_later_ones`, `rewind_removes_entries_from_search`, `rewind_to_an_unknown_point_is_not_found`, `restore_latest_replays_the_active_conversation`, `restore_latest_follows_the_fork_not_the_abandoned_branch` — PASS.
- `titi-engine`: `restored_history_is_replayed_before_the_prompt` — PASS.
- `titi-cli` checkpoints: `checkpoint_builtins_are_reserved`, `checkpoint_rewind_roundtrip_through_the_helpers`, `rewind_reports_missing_and_out_of_range_checkpoints`, `an_app_without_a_session_reports_it_instead_of_panicking` — PASS.
- `titi-cli` headless: `frame_version_is_optional_and_enforced` — PASS.
- E4 unit: `Claims` (5 тестов), `Findings` (3), `Steering` (3) — PASS.
- E4 integration (`titi-engine/tests/claims.rs`): `a_claimed_file_is_refused_without_touching_disk`, `a_released_file_can_be_written`, `queued_steering_is_injected_before_the_prompt_answer`, `steering_sent_mid_turn_reaches_the_provider` — PASS.
- `titi-engine/tests/agents.rs`: `a_subagent_finding_reaches_the_parent_bus`, `stopping_an_agent_releases_its_write_claims` — PASS.
- `titi-cli`: `typing_during_a_turn_steers_instead_of_queueing_a_new_turn` — PASS.
- `titi-tools` cache/fs: `a_second_read_of_an_unchanged_file_hits_the_cache`, `a_deleted_file_is_not_served_from_cache`, `a_changed_file_is_read_again`, `invalidate_forces_a_fresh_read`, `the_cache_is_bounded_and_evicts_the_least_recently_used`, `a_missing_file_reports_an_error`, `a_write_invalidates_the_cached_body`, `two_reads_share_one_cache` — PASS.
- `titi-engine` review: `a_reply_is_read_by_its_first_verdict_token`, `a_reply_that_names_no_verdict_is_partial`, `echoing_the_brief_is_not_approval`, `the_first_token_wins_even_when_another_is_quoted_later`, `exit_codes_match_the_goal_loop_contract`, `the_prompt_carries_the_brief_goal_and_evidence`, `the_reviewer_sees_only_the_goal_and_the_evidence`, `the_verdict_and_exit_code_follow_the_reply`, `a_reviewer_that_fails_reports_the_error` — PASS.
- `titi-providers`: `completions_tool_call_with_partial_args` (фрагменты, не буфер), `completions_tool_arguments_concatenate_to_valid_json` — PASS.
- `titi-engine/tests/tools.rs`: `fragmented_tool_arguments_reach_the_handler_joined` — регресс на исправленный баг — PASS.
- Сквозной прогон: `read` через фрагментированные аргументы вернул реальный `Cargo.toml`; genome как system-message дошёл до провайдера (`system=yes`).
- `titi-engine/tests/tool_agent.rs`: сабагент читает файл своим tool loop-ом, греет общий кэш, не может писать по умолчанию, уважает чужой claim при `agent_writes`, останавливается на round cap — PASS.
- `runtime::tests`: `an_off_thread_panic_degrades_to_none`, `an_off_thread_failure_degrades_to_none`, `a_successful_run_passes_the_map_through` — PASS.
- Сквозная проверка сабагента через реальный бинарь и реальный HTTP: `AgentProgress { tools: read }` → `read` вернул 1056 байт `Cargo.toml` → `AgentFinished { success: true }`.\n- `titi-cli` session_log: очередь `(User, prompt)` → `(Assistant, reply)`, пустой ответ не пишется, steered-сообщение попадает в транскрипт, agent-события — нет, стамп в несуществующую сессию возвращает ошибку, а не панику — PASS.
- Сквозная проверка резюма через реальный бинарь (два запуска подряд против локального сервера) — PASS.
- `titi-engine` `TouchedSet`: ресенси без дублей, вытеснение самого старого на пределе, пустое множество — PASS.
- `titi-cli`: `approval_modes_parse_and_reject_typos` — PASS.
- `titi-engine`: `restore_history_replaces_what_the_model_sees` — откат реально меняет то, что уходит провайдеру — PASS.
- `titi-cli`: `session_history_matches_what_the_log_wrote`, `session_history_stops_at_the_tail_cap` — PASS.
- `titi-cli` interface: `a_failed_turn_says_so_instead_of_nothing`, `switching_sessions_drops_the_rendered_conversation` — PASS.
- `titi-cli`: `a_new_session_starts_empty_and_becomes_the_latest` — новая сессия пустая, становится latest, старая не тронута — PASS.
- `titi-cli`: `ctrl_c_stops_a_running_turn_and_exits_when_idle` — Cancel при активном turn, Exit в покое — PASS.
- `titi-engine`: `cancelling_a_turn_unblocks_a_pending_approval` — cancel при ожидании approve разблокирует turn и не запускает тул — PASS.
- `titi --help` печатает usage, `titi --hedless` печатает usage и выходит с кодом 2 — проверено вручную.
- `titi-tui` recap: секции стартуют свёрнутыми, Enter раскрывает выбранную, Esc закрывает, `Ctrl+O` раскрывает/сворачивает все, курсор зажат на краях, длинная секция переносится и не выходит за рамку, окно следит за выбранной секцией, пустой рекап не рисует ничего — PASS.
- `titi-cli` recap: сборка секций из реального store + trajectory (2 промпта, read ×2 = 20ms total/10ms avg, bash failed 2.4s, `src/parse.rs ×2`), пустая сессия говорит «none called», `/recap` зарезервирован, без живой сессии — alert, панель принимает `Ctrl+O` и Esc, `Ctrl+O` вне панели раскрывает/сворачивает транскрипт — PASS.
- `titi-cli`: `pause_stops_the_agent_and_not_only_the_keyboard` — `/pause` возвращает `Pause`, Esc снимает модалку — PASS.
- `titi-engine` compaction: ниже порога ничего не складывается, самый старый префикс становится одним дайджестом (цепочка падает с `Remote` на `snapcompact`), хвост не начинается с tool-результата, пустая история не трогается, оценка покрывает все сообщения — PASS.
- `titi-engine` tools: `a_long_turn_folds_its_oldest_messages` — turn с малым окном реально складывает префикс, следующий запрос несёт дайджест — PASS.
- `titi-cli`: `ctrl_c_stops_a_running_turn_and_asks_twice_when_idle` — первый Ctrl+C только вооружает, второй в окне выходит, просроченный просит заново, другая клавиша снимает — PASS.
- `titi-engine` loop: `the_system_prompt_carries_identity_and_memory`, `context_usage_reports_the_request_size` — PASS.
- `titi-cli` git_checkpoint: снимок ловит изменение, restore отказывается на грязном дереве, не-репозиторий сообщает — PASS.
- `titi-engine` agents: `focusing_an_agent_emits_the_move` — фокус шлёт `AgentFocused`, неизвестный агент даёт `Failed` — PASS.
- `titi-cli`: `herdr_state_follows_the_turn` — idle → working на `TurnStarted`, blocked с «waiting for approval» на `ToolApprovalNeeded` — PASS.
- `cargo test --workspace` — 891 passed, 0 failed.
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
- Claim берётся по `path` из аргументов тула, поэтому `bash` (произвольная команда) файлы не резервирует.
- `StreamingAgentRunner` остаётся для surfaces без тулов; рабочий путь сабагента — `ToolAgentRunner`, который runtime строит сам, когда заданы `agent_model` и `workspace_root`.
- Сабагент read-only по умолчанию: включение write — сознательное решение вызывающего, а не следствие конфигурации.
- Бounded `mpsc` с живым, но непрочитанным receiver — гарантированный deadlock: первый `send` занимает слот, второй блокируется навсегда. Такой канал надо `drop(receiver)`, а не бросать в `_receiver`.
- Кэш не отдаёт содержимое удалённого файла (сначала stat) — поэтому удаление видно сразу, а изменение без смены size/mtime теоретически нет.

## NEXT

0. **Остался один шаг до рабочего TUI с реальной моделью**: `titi --set-key opencode-go <ключ>` (конфиг провайдера уже записан, endpoint `https://opencode.ai/zen/go/v1`). Ключ пользователя я не извлекаю — он должен быть введён им. Проверено, что после этого шага путь до HTTP работает.
1. E5: GPUI desktop workbench поверх того же `EngineCommand`/`EngineEvent`.
1a. **Системный промпт подключён** — `EngineRuntime::system_prompt` собирает `titi_soul::SystemPromptBuilder` (SOUL.md + personality) и оба memory-стора (`MEMORY.md`, `USER.md`), затем genome-карту. `EngineConfig.agent_dir` задаётся из `titi_config::agent_dir()`. Без каталога агента деградирует до одной карты.
1b. **Gauge контекста** — `EngineEvent::ContextUsage { tokens, window }` перед каждым запросом (оценка `estimate_request`, провайдеры usage не отдают). `App.context_pct` кормит `StatusSnapshot.context_pct`, бар рисует `N%`.
1c. **Git-чекпоинты** — `Checkpoint.git_commit`; `/checkpoint` делает `git add -A && commit` (`titi-cli/src/git_checkpoint.rs`, локально, `--no-verify`, автор `titi`), `/rewind` делает `git reset --hard` и **отказывается на грязном дереве**. Не-репозиторий остаётся session-only.
1d. **Фокус агента** — `AgentSupervisor::focus` шлёт `EngineEvent::AgentFocused`; `App.focused_agent` показывает его в статус-баре. Несуществующий агент даёт `Failed`.
1e. **`--prompt`** — `titi --headless "текст"` и `titi --prompt "текст"` запускают один turn (`headless::run_prompt`): события на stdout, ответ на stderr, код 1 при `Failed`.
1f. **Herdr** — `titi-cli/src/herdr.rs` репортит `pane.report_agent` на `HERDR_SOCKET_PATH` (`source: "herdr:titi"`, состояния `working`/`blocked`/`idle`), тот же контракт, что `herdr-agent-state.ts` у OMP. Вне pane (`HERDR_ENV` не задан) — no-op. Спека: `docs/research/empryo-port/herdr.md`. Orca — провайдер, не протокол состояния: подключается записью в registry.
1g. **Корень агента** — всё состояние в `~/.titi/agent` (профиль: `~/.titi/profiles/<name>/agent`), как у OMP (`~/.omp/agent`) и pi (`~/.pi`). Репозиторий не хранит сессии, память и секреты; `<project>/.titi/` остаётся только для `config.yml`. Слой `<project>/.titi/.env` убран. Спека: `docs/research/empryo-port/agent-home.md`.
1h. **Хранилище остаётся как есть** — JSONL на сессию, один `state.db` (SQLite, WAL, FTS5) как производный индекс, память в `MEMORY.md`/`USER.md`. Turso не берём: репликации нет, индекс локальный и пересобирается из JSONL. БД на сессию ломает поиск по всем сессиям и всё равно требует общий каталог. Спека: `docs/research/empryo-port/storage.md`.
1i. Чего ещё нет: скиллы/hooks/MCP (E6), вкладки (в Empryo до 5), стоимость в USD (нет цен провайдеров), STT.
1b. TUI: рекап читает только store+trajectory; агентские findings и стоимости в него пока не попадают (нужна персистентность findings).
2. Genome: tree-sitter для остальных языков и symbol-level граф.
3. Goal loop поверх reviewer-а (coder ⟷ reviewer rounds с oscillation-детекцией).

## Verification baseline

```bash
cargo fmt --check
cargo test -p titi-engine
cargo clippy -p titi-engine --all-targets
cargo test --workspace
```
