# Tool-конверсия

Тема: как единый внутренний tool-call формат конвертируется в форматы конкретных моделей (anthropic, openai, gemini, harmony, glm-4.5, qwen3, kimi-k2, minimax, deepseek, xml, hermes) и обратно — при стриминге, с восстановлением после мусора.

## omp

omp решает задачу через **каноническое внутреннее представление** + два класса конвертаций: нативные адаптеры (hosted API) и «owned in-band диалекты» (open-weight модели, у которых tool-calls живут прямо в тексте).

- **Канонический формат** — pi-ai `ToolCall` content blocks внутри `Context` / `AssistantMessageEvent`; это единственное представление, с которым работают агентский цикл, хранение и TUI. Транспорт `pi-native` (`POST /v1/pi/stream`) гоняет эти блоки на gateway без преобразований, сохраняя tool-call IDs, images, thinking budgets и tool-choice. Источник: omp://toolconv/pi-native.md.
- **Выбор формата** — enum `tools.format` из 13 значений (`auto|native|glm|hermes|kimi|xml|anthropic|deepseek|harmony|qwen3|gemini|gemma|minimax`) или `PI_DIALECT=...`; `auto` держит нативные вызовы, а при `supportsTools: false` падает на диалект по family-affinity (`preferredDialect`, fallback — `glm`). Источник: omp://toolconv/hermes.md.
- **Owned-конвейер** (каждый диалект, файлы `packages/ai/src/dialect/*.ts`): (1) убрать нативное поле `tools` из запроса; (2) дописать в system prompt компактный каталог `<tools>` (по одному OpenAI-стиль JSON-объекту на строку) + format guide диалекта; (3) перерендерить историю (прошлые вызовы/результаты) в текстовый синтаксис диалекта; (4) сканировать стрим нарастающим парсером и проецировать в канонические события `toolStart` → `toolArgDelta` → `toolEnd`, минтируя id (`ptc_…`), т.к. большинство форматов id не несут. Источники: omp://toolconv/hermes.md, omp://toolconv/glm-4.5.md, omp://toolconv/minimax.md.
- **Коэрция аргументов по схеме**: для форматов без кавычек/с типизацией по схеме (glm-4.5 `<arg_value>`, xml/minimax `<parameter>`) string-параметры остаются сырым текстом, остальные проходят repairing-JSON; исключение — GLM-4.5, где всё кроме string-типа строго `JSON.parse` с откатом к сырому тексту. Кап параметра — 1 000 000 code units. Источники: omp://toolconv/glm-4.5.md, omp://toolconv/xml.md.
- **Защита от мусора**: guard против модельной подделки результатов (`<tool_response>`/`<function_results>` в assistant-тексте → stop + `tools.abortOnFabricatedResult`); частично завершённый вызов при нормальном stop сохраняется с уже накопленными аргументами (или `{}`) и может быть диспетчеризован, при stop=`length` — не запускается; DSML-«хилинг» протечек шаблона DeepSeek через scanner-only опцию `xmlTagset: "dsml"`. Источники: omp://toolconv/xml.md, omp://toolconv/deepseek.md.
- **Нативная нормализация схем**: для Anthropic перед отправкой JSON-Schema режется до белого списка ключей (без `oneOf`, root-комбинаторов, `pattern` и т.п.; выброшенные констрейнты переезжают в `description`), strict-режим — только для 4 встроенных инструментов с бюджетами 20 strict-тулов / 24 optional-свойства / 16 union. Источник: omp://toolconv/anthropic.md.

## Hermes

Hermes Agent не имеет отдельного «dialect-конвейера» как omp — конверсия формат-в-формат живёт в резолвере провайдера и адаптерах API.

- **Три API-режима** — `chat_completions`, `codex_responses`, `anthropic_messages`; `agent/runtime_provider.py: resolve_runtime_provider()` превращает пару провайдер+модель в `api_mode` + креденшелы, и весь агентский цикл (`AIAgent.run_conversation` → `model_tools.handle_function_call`) работает поверх этого выбора. Источник: https://hermes-agent.nousresearch.com/docs/developer-guide/architecture
- **Форматная конверсия инкапсулирована в `agent/anthropic_adapter.py`** — «Anthropic Messages API format conversion»: конвертация между OpenAI-style `tool_calls[]` (строка `arguments`) и Anthropic content-blocks (`tool_use`/`tool_result`, объект `input`) происходит в одном модуле, а не размазана по коду. Источник: https://hermes-agent.nousresearch.com/docs/developer-guide/architecture
- **Сокращение числа вызовов вместо конверсии**: Programmatic Tool Calling через `execute_code` позволяет свернуть многошаговый pipeline в один inference call — иной способ решения проблемы «много форматов → много round-trip'ов». Источник: https://hermes-agent.nousresearch.com/docs/
- Инструментная поверхность унифицирована: registry (`tools/registry.py`, 70+ tools / 28 toolsets) выдаёт схемы, а dispatch (`model_tools.handle_function_call`) не зависит от wire-формата — конверсия отделена от исполнения. Источник: https://hermes-agent.nousresearch.com/docs/developer-guide/architecture

## Vellum

Тема wire-форматов tool-calls в собранном материале **не покрывается** — там нет ни одного факта о форматах вызовов или их конверсии. Ближайшие аналоги, релевантные слою конверсии:

- **Каждый вызов инструмента — в песочнице, по умолчанию deny**: граница «решение модели → исполнение» принудительно проходит через sandbox, что диктует форму канонического результата (structured verdict, а не свободный текст). Источник: local://vellum-summary.md.
- **Учётные данные живут в отдельном процессе и никогда не попадают в модель** — конвертер не имеет права протаскивать креды через tool-результаты/промпты; actor identity (guardian/trusted/unknown) резолвится один раз и гейтит триггер инструментов. Для мультиботной сети titi это значит: диалект-слой должен быть изолирован per-bot, без общего состояния сканеров между ботами. Источник: local://vellum-summary.md.

## Решение (одно/комбо)

Комбо, ядром которого является каноническое представление omp. Единый тип `ToolCall`/`ToolResult` (объектные аргументы, обязательный id) — единственный источник правды; каждый формат реализуется парой `render + scan` (рендерер истории + нарастающий сканер), а не ad-hoc конвертацией сообщений. Нативные hosted-API (Anthropic Messages, Gemini, OpenAI-совместимые) получают прямые адаптеры wire-формата с нормализацией схем на белых списках; open-weight модели — owned in-band диалекты (harmony, hermes/qwen3, glm-4.5, kimi, deepseek, gemini-pythonic, xml, minimax), которые убирают нативные `tools` и переносят протокол в текст по проверенным форматам, сканируя стрим в канонические события. Это оптимально по эффективности: агентский цикл, хранение, компакция и TUI пишутся один раз против канона; добавление формата = один файл пары render/scan + golden-тесты; общий repairing-JSON, коэрция по схеме, капы и fabricated-result guard переиспользуются всеми диалектами. Для openai-формата отдельная дока в omp отсутствует — представителем семейства служит harmony (формат gpt-oss), а классический Chat Completions `tool_calls[]` покрывается нативным адаптером по правилам из omp://toolconv/anthropic.md (таблица маппинга OpenAI↔Anthropic).

### Таблица форматов

| Формат | Семейство | Вызов (assistant) | `arguments` на проводе | id на проводе | Результат (feedback) | Связка вызов↔результат |
| --- | --- | --- | --- | --- | --- | --- |
| **anthropic** (нативный) | Claude (hosted) | `tool_use` content block + `stop_reason:"tool_use"` | **объект** (`input`), не строка | `toolu_…` | `tool_result` block в `user`-сообщении (первым), `is_error` | `tool_use_id`; стрим: `input_json_delta` |
| **anthropic-XML** (owned, базовый) | Claude (token-level) | `<invoke name="…"><parameter name="…">v</parameter></invoke>` в `<function_calls>` | string по схеме — сырой текст, остальное JSON | нет (минтится) | `<function_results><result><tool_name><stdout>` / `<error>` | по порядку |
| **openai** (нативный; доки нет — см. harmony) | OpenAI-совместимые | `message.tool_calls[]` + `finish_reason:"tool_calls"` | **JSON-строка** (`function.arguments`) | `call_…` | `{"role":"tool","tool_call_id","content"}` | `tool_call_id` |
| **harmony** (owned) | gpt-oss | `<\|start\|>assistant<\|channel\|>commentary to=functions.NAME<\|message\|>{json}<\|call\|>` | JSON-тело после `<\|message\|>` | нет (минтится) | `<\|start\|>functions.NAME to=assistant<\|channel\|>commentary<\|message\|>…<\|end\|>` | по порядку; role сообщения = имя тула |
| **gemini** (owned, Pythonic) | Gemini / Gemma 3 | ` ```tool_code ` → `print(default_api.NAME(k=v))` | **Python-литералы** (`True/None`, одинарные кавычки) | нет (минтится) | ` ```tool_outputs ` блок в следующем user-ходе | по порядку |
| **hermes** (owned) | Hermes 2/3, community | ChatML: `<tool_call>\n{"name":…,"arguments":{…}}\n</tool_call>` | **вложенный JSON-объект** (не строка) | нет (`ptc_…`) | `<\|im_start\|>tool` + `<tool_response>{"name","content"}` | по порядку (имя в результате) |
| **qwen3** (owned) | Qwen3/2.5/QwQ | как hermes, но `FunctionCall`-схема выброшена | вложенный объект | нет (`ptc_…`) | `<tool_response>` с **голым** контентом внутри **`user`**-хода | только по порядку |
| **glm-4.5** (owned) | GLM-4.5/4.6 | `<tool_call>NAME\n<arg_key>k</arg_key><arg_value>v</arg_value>…</tool_call>` | string — **без кавычек**, остальное JSON (тип из схемы) | нет (`ptc_…`) | `<\|observation\|>` + `<tool_response>` (один на результат) | по порядку; `tool_call_id` игнорируется шаблоном |
| **kimi-k2** (owned) | Kimi K2 | `<\|tool_calls_section_begin\|><\|tool_call_begin\|>functions.NAME:IDX<\|tool_call_argument_begin\|>{json}<\|tool_call_end\|>…` | JSON-строка между маркерами | **есть**: `functions.{name}:{idx}` | `<\|im_system\|>{name}<\|im_middle\|>## Return of {id}\n{content}` | `tool_call_id` (idx per-turn) |
| **minimax** (owned) | MiniMax | `<minimax:tool_call><invoke name="…"><parameter name="…">v</parameter></invoke></minimax:tool_call>` | string по схеме — сырой текст, остальное JSON; `string="true/false"` override | нет (минтится) | `<function_results><result><tool_name><stdout>` / `<error><stderr>` | по порядку |
| **deepseek V3.1** (owned) | DeepSeek V3.1 | `<｜tool▁calls▁begin｜><｜tool▁call▁begin｜>NAME<｜tool▁sep｜>{json}<｜tool▁call▁end｜>…<｜tool▁calls▁end｜>` (fullwidth `｜` U+FF5C, `▁` U+2581) | сырой JSON без fence; вызовы **встык**, без разделителя | нет (`ptc_…`) | `<｜tool▁output▁begin｜>…<｜tool▁output▁end｜>`; ответ модели идёт сразу после, без `<｜Assistant｜>` | по порядку |
| **xml** (owned, generic) | любой | `<invoke name="…"><parameter name="…">v</parameter></invoke>`, опц. `<tool_calls>`-обёртка | как minimax (string-схема/JSON) | нет (минтится) | `<tool_response>` голый текст, **без флага ошибки** | только по порядку |
| **pi-native** | — (транспорт, не диалект) | канонические `ToolCall`-блоки в `Context`, SSE `/v1/pi/stream` | объект | сохраняется | канонические блоки без reshaping | id сохраняется (lossless) |

## Rust-маппинг

Крейты workspace: **titi-core** (канонические типы), **titi-providers** (нативные адаптеры + owned-диалекты), **titi-tools** (реестр и JSON-Schema тулов), **titi-tui**/**titi-cli** (потребители событий). Рекомендуется новый **titi-dialect** — изолированный, без зависимости от providers, чтобы диалекты тестировались golden-файлами автономно.

```rust
// titi-core: канон — единственный источник правды
#[derive(Clone, Serialize, Deserialize)]
pub enum ContentBlock {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

pub struct ToolCall { pub id: ToolCallId, pub name: String, pub arguments: serde_json::Value }
pub struct ToolResult { pub call_id: ToolCallId, pub content: ResultContent, pub is_error: bool }

pub enum StopReason { EndTurn, ToolUse, MaxTokens, StopSequence, Paused, Aborted }
```

```rust
// titi-dialect: один трейт на формат; render — история, scan — стрим
pub trait Dialect: Send + Sync {
    fn id(&self) -> DialectId;                       // Anthropic | Harmony | Hermes | Qwen3 | Glm45 | Kimi | DeepSeek | GeminiPy | Xml | MiniMax
    fn render_tools(&self, tools: &[ToolDef]) -> String;          // каталог + format guide
    fn render_history(&self, ctx: &Context) -> Vec<RenderedMsg>;  // вызовы/результаты -> текст
    fn scanner(&self, tools: &[ToolDef]) -> Box<dyn InbandScanner>;
}

pub trait InbandScanner {
    fn feed(&mut self, chunk: &str) -> Vec<StreamEvent>;          // chunk-boundary safe
    fn flush(&mut self) -> Vec<StreamEvent>;                       // EOF-семантика (незавершённый вызов)
}

pub enum StreamEvent {
    ToolStart { id: ToolCallId, name: String },
    ToolArgDelta { id: ToolCallId, key: String, delta: String },
    ToolEnd { id: ToolCallId, arguments: serde_json::Value, raw: String },
    ThinkingDelta(String), TextDelta(String),
}

// титi-providers: нативные адаптеры — трейт над wire-конверсией
pub trait NativeAdapter {
    fn encode_request(&self, ctx: &Context, tools: &[ToolDef], opts: &StreamOpts) -> Result<reqwest::Request>;
    fn decode_event(&self, sse: &SseFrame) -> Option<StreamEvent>; // input_json_delta, tool_calls deltas, functionCall…
    fn normalize_schema(&self, schema: &mut serde_json::Value);    // белый список ключей (anthropic-стиль)
}
```

- **Общие утилиты** (titi-dialect): repairing-JSON (`serde_json` + собственный repair, как omp coercion), коэрция по схеме (string vs non-string, `string="true/false"` override), кап параметра 1 МБ, hold-back частичных маркеров между чанками, guard подделанных результатов.
- **Внешние крейты**: `tokio` + `reqwest`/`reqwest-eventsource` (SSE), `serde`/`serde_json` (канон и каталоги), `winnow` или рукописные state-машины для сканеров (GLM-4.5 XML→JSON, Gemini Python-литералы, DeepSeek fullwidth-маркеры), `regex` для статических extraction-паттернов (нежадные `<tool_call>.*?</tool_call>`), `schemars`/`serde_json` для JSON-Schema обхода при коэрции, `unicode-ident`-независимая работа с U+FF5C/U+2581 (просто `&str`-литералы), `insta` (golden-тесты), `criterion` (бенчмарк сканеров).
- **Хранение диалектов**: реестр `phf::Map<DialectId, fn() -> Box<dyn Dialect>>`; family-affinity по имени модели — как omp `preferredDialect` (auto → натив, fallback glm).

## Definition of Done

- [ ] Round-trip тест render→scan для каждого из 10 диалектов (harmony, gemini, hermes, qwen3, glm-4.5, kimi, minimax, deepseek, xml, anthropic-XML) на общем fixture `get_weather(location, unit)` — итоговый `ToolCall` совпадает байт-в-байт с ожидаемым `arguments`.
- [ ] Golden-тесты (insta) рендера истории и каталога `<tools>` совпадают с примерами из omp-док (ChatML-ходы hermes/qwen3, `[gMASK]<sop>`-стрим glm-4.5, fullwidth-маркеры deepseek V3.1, `functions.{name}:{idx}` kimi).
- [ ] Chunked-stream тест: подача потока чанками по 1–7 байт даёт идентичную последовательность `StreamEvent` (chunk-boundary safety, включая частичные маркеры `<|tool_call_begin|>`, `<arg_value>`).
- [ ] Семантика EOF: вызов без закрывающего маркера при stop=Stop сохраняется с аргументами `{}`/накопленными; при stop=MaxTokens — `StopReason::MaxTokens`, не диспетчеризуется.
- [ ] Коэрция по схеме: glm-4.5 `Beijing` → `"Beijing"`, `3` → `3`, `true` → `true`; xml/minimax `string="false"` включает JSON-парсинг; malformed JSON откатывается к сырому тексту.
- [ ] DeepSeek-специфика: ASCII-подмены маркеров (`<|tool_calls_begin|>`) не матчатся; DSML-протечка `<｜DSML｜invoke …>` хилится в вызов и убирается из видимого текста.
- [ ] Нативные адаптеры: canonical→Anthropic (`input` объект, `tool_result` первым в `user`-ходе) и canonical→OpenAI (`JSON.stringify(arguments)`, `role:"tool"`) проходят интеграционный тест на маппинге из omp://toolconv/anthropic.md.
- [ ] Fabricated-result guard: `<tool_response>`/`<function_results>` в assistant-стриме останавливает проекцию; при `abort_on_fabricated_result=true` генерация абортится.

## Deep-dive

Подсистемы-доки (план 2-го уровня, `docs/research/toolconv/`):

- `canonical-model.md` — канонические типы ToolCall/ToolResult/StopReason, stop-семантика, pi-native lossless-транспорт (по omp://toolconv/pi-native.md).
- `native-adapters.md` — Anthropic Messages (content blocks, input_json_delta, ordering rules, нормализация схем), OpenAI Chat Completions (стрингификация arguments), Gemini GenerateContent (functionCall/functionResponse, thoughtSignature).
- `owned-dialects-json.md` — hermes, qwen3, kimi-k2, deepseek (V3.1/legacy/DSML), harmony: ChatML/спец-токены, minting id, repairing-JSON.
- `owned-dialects-xml.md` — anthropic-XML, glm-4.5 (arg_key/arg_value, типизация по схеме), xml, minimax: delimiter-matching вместо XML-парсера, капы, healing.
- `gemini-pythonic.md` — tool_code/tool_outputs, Python-литералы, fence- ambiguities, leak-поведение MALFORMED_FUNCTION_CALL.
- `streaming-scanners.md` — дизайн InbandScanner: chunk-boundary hold-back, toolStart/toolArgDelta/toolEnd, EOF-семантика, fabricated-result guard.
- `schema-normalization.md` — белые списки ключей JSON-Schema per-провайдер, strict-режим и бюджеты, переезд констрейнтов в description.
