# Journey-граф обучения

Таймлайн «всего выученного агентом»: сохранённые навыки и записи памяти, нанесённые на временную ось (старые сверху, новые снизу), с воспроизведением-реплеем («constellation» scrubber), интерактивным просмотром и операциями prune/edit над узлами. Это визуализация и точка управления поверх [stores.md](./stores.md) — не отдельный стор.

## omp

omp не покрывает тему journey-графа/визуализации обучения: ни в omp://memory.md, ни в omp://mnemosyne-memory-backend.md, ни в доках инструментов нет ни графовой витрины, ни таймлайна. Ближайшие аналоги по составным частям:

1. Авто-консолидация как источник «узлов»: пайплайн `memory.backend: local` генерирует три артефакта — `MEMORY.md` (долгосрочная память), `memory_summary.md`, `skills/` (процедурные playbook'и, каждый в своей поддиректории, stale-директории прунятся автоматически между ранами) — источник omp://memory.md. Т.е. материал графа (skills + memory-куски) у omp есть, витрины-графа нет.
2. Управление памятью как CLI-поверхность: `/memory view|stats|diagnose|clear|enqueue` — единая команда для всех бэкендов (omp://memory.md); `/memory enqueue` форсирует retention текущей сессии, flush pending extraction и полный sleep/consolidation по банкам с age-гейтом (`workingMemoryTtlHours/2` = 12ч) — источник omp://mnemosyne-memory-backend.md. Это рудимент «admin-команд над накопленным», но без граф/таймлайн-представления.
3. Графовая механика есть только внутри Mnemopi: `mnemopi.proactiveLinking` — инжест новых воспоминаний в episodic graph с линковкой на связанные сущности/воспоминания при записи; polyphonic recall использует граф как один из 4 голосов (vector, graph, fact, temporal) с reciprocal rank fusion — источник omp://mnemosyne-memory-backend.md. Это внутренний retrieval-граф, не экспонируемый пользователю как таймлайн.

## Hermes

Эталон — «Learning Journey» (`/journey`):

1. Состав графа: узлы — сохранённые skills (SKILL.md) и куски памяти; memory-узлы адресуются как `memory:<source>:<index>`, скиллы — по имени; данные графа одни и те же для трёх поверхностей: CLI `hermes journey` (алиасы `hermes learning`, `hermes memory-graph`), TUI-оверлей `/journey` (алиасы `/learning`, `/memory-graph`), Desktop «Star Map»/memory-graph панель (источник: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory).
2. CLI-флаги и рендер: `--play` анимирует build-up (темп через `--fps`), `--width`/`--height` — размер рендера, `--no-color`, `--json` — сырой graph payload для машинной обработки; таймлайн: oldest at top, newest at bottom; «constellation» scrubber проигрывает накопление (там же).
3. Операции над узлами (prune/correct): `hermes journey list` — перечисляет node id; `hermes journey delete <node> [-y]` — удаляет узел, при этом skills архивируются (restorable), memory-куски удаляются; `hermes journey edit <node>` — открывает содержимое узла (SKILL.md или memory-кусок) в `$EDITOR`. Те же `list/delete/edit` доступны из in-chat `/journey`; Desktop-панель даёт edit/delete прямо на узлах (там же).
4. Источник узлов — те же механизмы записи, что в stores: память пишет инструмент `memory` + фоновый self-improvement review; скиллы — `skills.write_approval` со staged-диффами (`/skills pending|diff|approve|reject`) — т.е. граф всегда консистентен со сторами, потому что это их проекция, а не отдельное хранилище (источники: https://hermes-agent.nousresearch.com/docs/user-guide/features/memory). Управление write-гейтом памяти через `memory.write_approval` и `/memory pending` — там же.

## Vellum

Vellum в собранном материале (local://vellum-summary.md) journey-граф не покрывает. Ближайшие аналоги:

1. Per-user журнал размышлений (reflections): непрерывная лента самоанализа агента — по сути таймлайн, но нарративный, а не графовый; в titi это может быть ещё одним типом узла (reflections) в графе.
2. `NOW.md` — скретчпад текущего фокуса и активных нитей: ортогонален графику «что выучено», но связан с ним: узлы, помеченные как активные нити, могли бы подсвечиваться в витрине.
3. Ежечасная проактивная перечитка заметок с поиском незавершённого — фоновый процесс, который может генерировать новые узлы (заметки о надвигающихся событиях prospective-памяти), т.е. пополнять таймлайн автономно. Секьюрити-рамка Vellum (unknown-акторы не читают память) очевидно переносится и на витрину: граф не должен показывать память unknown-актору.

## Решение (одно/комбо)

Одно: проекция Hermes-style без собственного хранилища. Граф вычисляется on-the-fly из уже существующих сторов (SQLite факты + каталог скиллов): узел = skill (имя) или memory-кусок (`memory:<source>:<index>`), позиция по `added_at`, тип как цвет/форма. Это оптимально по эффективности: ноль синхронизации, ноль двойной записи, граф не может разойтись со сторами; `--json`-payload собирается одним SQL-запросом с UNION. Реплей-«constellation» реализуем в TUI на crossterm без эмбеддингов и без внешних зависимостей. Операции `list/delete/edit`: delete у skill-узла — архивация (переименование каталога, restorable), у memory-узла — удаление строки; edit — `$EDITOR` над SKILL.md/записью с последующим security-сканом как у любой записи. Компонент Vellum добавляем минимально: узлы-рефлексии и подсветка `NOW.md`-нитей — только если сторы их уже хранят, отдельной инфраструктуры не заводим.

## Rust-маппинг

Крейты: `titi-memory` — извлечение графа из сторов (SQL-проекция); `titi-tui` — рендеринг таймлайна и scrubber; `titi-cli` — `titi journey list|delete|edit|play`; `titi-tools` — не участвует (граф — пользовательская витрина, не модельный инструмент).

```rust
// titi-memory::journey
pub enum NodeKind { Skill { name: String }, MemoryChunk { source: String, index: u32 } }
pub struct JourneyNode { pub id: NodeId, pub kind: NodeKind, pub at: DateTime<Utc>, pub summary: String }
pub struct GraphPayload { pub nodes: Vec<JourneyNode> }            // то, что отдаёт --json

impl JourneyIndex {
    pub fn build(db: &rusqlite::Connection, skills_dir: &Path) -> Result<GraphPayload, Error>; // UNION проекция
    pub fn delete(&mut self, id: NodeId, confirm: bool) -> Result<Deleted, Error>;  // skill → Archived, memory → removed
    pub fn edit(&mut self, id: NodeId, editor: impl FnOnce(&Path) -> io::Result<()>) -> Result<(), Error>;
}

// titi-tui
pub struct JourneyView { cursor: usize, playing: bool, fps: u32 }
impl JourneyView {
    pub fn render(&mut self, frame: &mut Frame, g: &GraphPayload, w: u16, h: u16, color: bool);
    pub fn scrub(&mut self, t: f32);                                // constellation-реплей build-up
    pub fn play(&mut self, fps: u32);
}
```

Внешние крейты: `crossterm` (события/рендер TUI-оверлея, `--no-color`), `ratatui` (виджеты таймлайна, если TUI уже на нём), `rusqlite` (проекция), `serde_json` (`--json` payload), `chrono` (сортировка узлов), `edit` или ручной `$EDITOR`-спавн (JourneyNode::edit).

## Definition of Done

- [ ] `JourneyIndex::build` собирает узлы из SQLite фактов и каталога skills одним запросом; узлы memory адресуются `memory:<source>:<index>`, скиллы — именем (тест на id-формат).
- [ ] Сортировка oldest→newest сохраняется в `GraphPayload` и в TUI-рендере (golden-тест payload).
- [ ] `titi journey --json` выводит сырой payload; `titi journey` рендерит таймлайн в терминале с `--width/--height/--no-color` (smoke: выходной код 0, непустой вывод).
- [ ] `--play` анимирует build-up с управляемым `--fps`; scrub-логика детерминирована при одинаковом payload (unit-тест на курсор).
- [ ] `delete` skill-узла архивирует каталог (restorable) и не трогает стор памяти; `delete` memory-узла удаляет строку из SQLite и не трогает скиллы (два теста).
- [ ] `edit` открывает содержимое в `$EDITOR` и после правки прогоняет security-скан: вредоносная правка отвергается (тест с фейк-редактором).
- [ ] Витрина недоступна при identity=unknown (ошибка доступа) — привязка к Vellum-изоляции (тест).
- [ ] Граф строится без собственного персистентного хранилища (code-review-критерий: нет вторых таблиц/файлов состояния графа).

## Deep-dive

Дальнейшие подсистемные доки (2-й уровень; писать при захвате фазы реализации):

- [../journey-graph/rendering.md] — ASCII/цветной рендер таймлайна на crossterm/ratatui, layout при разных `--width/--height`, обработка узлов за экраном.
- [../journey-graph/skill-archive.md] — архивация/восстановление skill-узлов, конфликт имён, влияние на discovery скиллов.

Кандидаты: экспорт графа (HTML/Star Map-подобная витрина), привязка узлов к NOW.md-нитям Vellum-стиля.
