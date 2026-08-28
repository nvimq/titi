# Траектория и self-improvement

Тема: запись траектории агента на диск, периодический GEPA-ревью (примерно каждые 15 тул-коллов), автосоздание `SKILL.md`, компаундинг-переиспользование накопленного опыта и риски (залипание в циклах, неверные скиллы).

## omp

omp темы «непрерывного self-improvement» как единого контура не покрывает, но даёт три готовых строительных блока, из которых контур собирается.

1. **Траектория уже персистится как сессионные артефакты.** Транскрипты сессий и сабагентов пишутся в append-only JSONL внутри артефакт-директории сессии (`<session>/<SubId>/…jsonl`);advisor-транскрипты — `__advisor[.<slug>].jsonl`. Автоматический handoff с `compaction.handoffSaveToDisk: true` пишет таймстампed-артефакт `handoff-*.md` — готовый «снимок что происходило» на диске (omp://handoff-generation-pipeline).
2. **Advisor — инкрементальный ревьюер транскрипта.** `AdvisorRuntime` получает только дельту транскрипта с прошлого обновления (включая reasoning и тул-интенты), ревьюит её отдельной моделью и доставляет заметки через тул `advise` с severity `nit`/`concern`/`blocker`. Ростер настраивается в `WATCHDOG.yml` (модель, грант тулов, специализация), а `advisor.immuneTurns` (по умолчанию 3) и emission-guard (нормализация, дедуп, content-free фильтр, максимум одна заметка за обновление) защищают от спама ревьюера (omp://advisor-watchdog).
3. **Тул `learn` — готовый механизм «урок → память → SKILL.md».** При `autolearn.enabled: true` и memory-бэкенде (`hindsight`/`mnemopi`/`local`) агент сохраняет урок (`memory`) и опционально создаёт/обновляет managed-скилл `<agent-dir>/managed-skills/<name>/SKILL.md` (cap 64 000 байт, имя `[a-z0-9][a-z0-9-]{0,63}`); authored-скиллы всегда побеждают managed, чтобы автосгенерированное не затирало рукописное (omp://tools/learn).
4. **Коммит результата как compaction-entry.** Документ handoff коммитится как `CompactionEntry` с `firstKeptEntryId` — старый префикс заменяется сводкой, свежая история сохраняется дословно; это паттерн «переиспользования»: следующий заход стартует со сжатого опыта (omp://handoff-generation-pipeline).

## Hermes

Hermes — эталон темы: self-improvement loop заявлен как headline-фича v0.8.0.

1. **Trajectory Capture на диск.** Hermes записывает каждый API-вызов, каждое решение о тул-колле и каждый вывод в порядке следования; траектория сохраняется в `~/.hermes/sessions/sessions.json` + `state.db` (FTS5-индекс всех прошлых сессий, полнотекстовый поиск «haven't we solved something like this before») и остаётся после конца сессии — в отличие от «stateless» фреймворков (источник: https://agentwikis.com/wiki/hermes/wiki/concepts/self-improvement-loop.md).
2. **GEPA-ревью каждые ~15 тул-коллов.** Каденс конфигурируется как `skills.creation_nudge_interval: 15` в `config.yaml`; агент «паузит, читает назад недавнюю траекторию, разбирает что сработало/что упало» и выдаёт правки собственных промптов и памяти — «back propagation for prompts instead of model weights». Механизм соответствует GEPA (Genetic-Pareto) из DSPy — reflective prompt evolution с Pareto-отбором кандидатов (корроборация: https://dspy.ai/api/optimizers/GEPA/overview/, https://arxiv.org/abs/2507.19457).
3. **Автосоздание SKILL.md + компаундинг.** Когда анализ траектории признаёт работу переиспользуемой, она пакуется в `~/.hermes/skills/custom/<slug>/SKILL.md` (frontmatter, тело, иногда скрипты); при похожем следующем запросе Hermes запускает существующий скилл, дообучает его на новом фидбеке и «ratchets capability upward» — скиллы «learn from each execution» (тот же источник, разделы 3 и 5).
4. **Рискованный контур — задокументированные режимы отказов.** (а) Stuck loops: «multiple fixed stuck agent loop entries» в changelog'ах, итерационные циклы, игнорирующие ввод пользователя; v0.8.0 вводит inactivity-based timeout вместо wall-clock. (б) Неверные скиллы: «LLM-generated… it is not actually going to be guaranteed to work» — рекомендован `hermes skills inspect` перед cron-использованием и периодический `hermes skills audit`; коллизии слагов дают полумёртвые дубли (`morning-briefing` + `morning-briefing-1`). (в) Context bloat от накопленной памяти на маленьких моделях; лечится `session_reset.mode: both` + `idle_minutes`. (г) Скиллы создаются только на «очень сложных» задачах — простые обходят контур (тот же источник, раздел Risks & Pitfalls).
5. **Self-диагностика харнесса (v0.8.0).** Hermes прогнал траектории против GPT/Codex-бэкендов, GEPA-ревью выявило 5 failure modes tool-use, и агент сам сгенерировал патчи провайдер-специфичного гайда «without a human in the loop» — демонстрация, что контур работает на самом харнессе (release notes v0.8.0 через ту же вики).

## Vellum

Vellum тему покрывает частично: самообучение есть, межботовый обмен — заявлен приоритетом пользователя без публичного описания протокола.

1. **Процедурная память и reflections как носители опыта.** Из восьми типов памяти Vellum `procedural`-тип (с собственным staleness window) — прямой аналог скиллов; per-user журнал размышлений (reflections) и `NOW.md`-скретчпад фиксируют, чему ассистент научился в ходе работы (local://vellum-summary.md, разделы «Память» и «Идентичность»).
2. **Мультиботность с пер-бот изоляцией и обменом опытом.** Приоритет пользователя: «пер-бот изоляция SOUL+memory+skills; обмен опытом между ботами» — то есть скиллы/уроки изолированы по ботам, а обмен — отдельный механизм поверх изоляции (local://vellum-summary.md, «Приоритеты пользователя для titi»). Конкретный транспорт обмена (общий реестр? экспорт-импорт? критерии доверия?) в материале не описан — ближайший аналог omp: managed-skills с «authored всегда побеждает generated» как правило разрешения конфликтов.
3. **Безопасность как рамка самоизменения.** Actor identity (guardian/trusted/unknown), «каждый вызов инструмента — в песочнице, по умолчанию — deny» — шаблон для ограничений автозаписи скиллов автократом (local://vellum-summary.md, «Безопасность»).

## Решение (одно/комбо)

Комбо из трёх omp-блоков, склеенных Hermes-каденсом. Траектория пишется на диск уже существующим механизмом сессионных JSONL + опциональные `handoff-*.md` артефакты (бесплатно, ничего нового не изобретаем); каждые ~15 тул-коллов (конфиг `trajectory.review_interval: 15`) запускается не постоянный advisor, а oneshot GEPA-ревьюер — дешёвый side-request по образцу `generateHandoffFromContext` с `toolChoice: "none"`, который читает недавнюю дельту траектории и решает: сохранить урок (память) и/или создать/обновить `SKILL.md` — ровно семантика тула `learn`. Компаундинг обеспечивается тем, что сгенерированные скиллы попадают в обычный skill-дискавери следующей сессии, а рукописные (authored) всегда побеждают автосгенерированные. Против рисков Hermes — анти-стак-гарды (детектор повторяющихся одинаковых тул-коллов + принудительный break/escalation) и карантин/аудит свежесозданных скиллов перед первым автозапуском; между ботами titi обмен опытом делаем экспортом/импортом скилл-пакетов с обязательным ревью получателем, сохраняя пер-бот изоляцию SOUL+memory, как требует приоритет Vellum. Это оптимально по эффективности: переиспользуем три готовых механизма omp вместо нового контура, а тяжёлый ревьюер работает дозированно (1 oneshot на ~15 тул-коллов) на маленькой/быстрой модели.

## Rust-маппинг

Крейты workspace:

- **titi-core** — ядро контура:
  ```rust
  struct TrajectoryEntry { ts: DateTime<Utc>, kind: EntryKind, /* ToolCall|ToolResult|Text|Error */ payload: Value }
  struct Trajectory { session_id: SessionId, entries: Vec<TrajectoryEntry> }
  trait TrajectorySink { fn append(&mut self, e: &TrajectoryEntry) -> io::Result<()>; fn since(&self, cursor: u64) -> Vec<TrajectoryEntry>; }
  struct GePaReviewer { interval: u32, model: ModelRef }           // счётчик тул-коллов -> oneshot
  enum ReviewVerdict { None, Lesson { text: String, context: Option<String> }, SkillOp { action: SkillAction, name: SkillName, body: String } }
  trait SkillStore { fn create(&self, s: &ManagedSkill) -> Result<(), SkillError>; fn update(&self, s: &ManagedSkill) -> Result<(), SkillError>; }
  struct StuckLoopDetector { window: u32, digest: ring::Digest }   // дедуп повторяющихся тул-коллов
  ```
- **titi-providers** — oneshot side-request ревьюера (аналог `generateHandoffFromContext`: переиспользование transform-пайплайна, `tool_choice: None` с ретраем на `Auto`, clamp reasoning).
- **titi-tools** — тул `learn` (`memory` + опциональный `skill { action, name, description, body }`), запись managed-скиллов `~/.titi/agent/managed-skills/<name>/SKILL.md`, лимиты 64 KiB / имя-регекс из omp.
- **titi-tui** — индикатор «GEPA review in progress», карточки `<advisory>`-стиля для вердиктов, `/skills audit` экран.
- **titi-cli** — флаг `--gepa` для headless-режима (аналог `--advisor`).
- **Новый крейт titi-trajectory** (или внутри titi-core): sink'и траектории + FTS-индекс + экспорт/импорт скилл-пакетов для межботового обмена.

Внешние крейты: `tokio` (background-таски ревьюера, spawn после N-го тул-колла), `serde`/`serde_json` (JSONL-сериализация), `rusqlite` (+ `fts5` feature — FTS5-индекс сессий как у Hermes), `sha2`/`xxhash-rust` (дайджесты для stuck-loop детектора и дедупа), `schemars`/`serde_yaml` (frontmatter SKILL.md и конфиг `trajectory.*`), `tracing` (инструментация ревью-цикла).

## Definition of Done

- [ ] Траектория пишется в append-only JSONL на диск при каждом тул-колле/результате; файл переживает перезапуск процесса (тест: kill -9 в середине сессии → файл валиден, `since()` возвращает все записи).
- [ ] После `trajectory.review_interval` (по умолчанию 15) тул-коллов запускается ровно один oneshot-запрос ревьюера с `tool_choice: none` (тест: mock-провайдер, счётчик вызовов == 1 на окно из 15).
- [ ] Вердикт `SkillOp { action: create }` записывает `managed-skills/<name>/SKILL.md` с валидным frontmatter; при конфликте с authored-скиллом — `isError` + урок сохранён, файл не создан (тест на оба исхода).
- [ ] Authored-скилл с тем же именем всегда побеждает managed в дискавери следующей сессии (интеграционный тест дискавери).
- [ ] Детектор stuck-loop: 5+ идентичных по дайджесту тул-коллов подряд прерывают цикл (принудительный break + карточка пользователю), а не уходят в бесконечность (тест на последовательности повторов).
- [ ] Свежесозданный скилл помечен `provenance: auto` и не автозапускается до первого ревью/аудита пользователем; команда `titi skills audit` выводит все auto-скиллы с датой и источником-травмой (traject entry id).
- [ ] Импорт скилл-пакета от другого бота не трогает SOUL и память получателя; импортированный скилл проходит тот же карантин (тест изоляции).
- [ ] `titi skills inspect <name>` показывает тело, frontmatter и статус (auto/authored, quarantined/active) — по образцу `hermes skills inspect`.

## Deep-dive

План подсистем-доков второго уровня (пишутся по мере проработки):

- `docs/research/trajectory-gepa/trajectory-capture.md` — формат JSONL-записей, ротация, FTS5-индекс, курсоры инкрементального чтения.
- `docs/research/trajectory-gepa/gepa-reviewer.md` — промпт oneshot-ревьюера, окно дельты, схема вердикта, ретраи tool_choice, бюджет токенов.
- `docs/research/trajectory-gepa/skill-synthesis.md` — генерация SKILL.md, frontmatter, лимиты 64 KiB, конфликт authored/managed, slug-нормализация.
- `docs/research/trajectory-gepa/compounding-reuse.md` — дискавери скиллов, дообучение на фидбеке, compaction-entry как носитель сжатого опыта.
- `docs/research/trajectory-gepa/safety-and-risks.md` — stuck-loop детектор, карантин auto-скиллов, context bloat, межботовый обмен с изоляцией SOUL+memory.
