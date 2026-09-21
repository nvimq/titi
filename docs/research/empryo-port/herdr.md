# Herdr integration

Herdr tracks every pane and rolls agent state up to the tab and workspace.
Agents with a lifecycle integration report that state themselves; everyone
else is guessed from the screen. OMP reports through
`herdr-agent-state.ts` (`source: "herdr:omp"`). titi reports the same way, so
a pane running it is tracked instead of guessed and `herdr agent wait` can
wait on it.

## When it reports

Only inside a pane Herdr launched. The process then has `HERDR_ENV=1`,
`HERDR_SOCKET_PATH` and `HERDR_PANE_ID`. Outside a pane `Reporter::from_env`
returns nothing and no report is attempted, so the TUI never depends on
Herdr running.

## The report

One newline-delimited JSON request on the Unix socket, the same shape OMP
sends:

```json
{"id":"titi-1","method":"pane.report_agent","params":{
  "pane_id":"w1:p1","source":"herdr:titi","agent":"titi",
  "state":"working","seq":1,"agent_session_id":"<session>"}}
```

`state` is one of Herdr's words:

| titi | reported | why |
|---|---|---|
| a turn is in flight | `working` | the agent is busy |
| a tool approval is open, or the exit confirmation is armed | `blocked` | another agent should stop and look |
| nothing running | `idle` | free to take work |

A change is reported once. A dead socket is ignored: Herdr going away must
not take the TUI with it.

## What this is not

Herdr's socket can also drive a pane (`pane.send_text`, `agent.prompt`).
titi does not listen on it. External control of a titi process is the
headless JSONL surface (`titi --headless`), which already speaks
`EngineCommand`. The two are complementary: Herdr observes, the JSONL
surface controls.

## Orca

OrcaRouter is a provider, not a state protocol. A titi process reaches it
the same way it reaches any other provider: an entry in the registry with a
base URL and a stored key. There is nothing to implement on the state side.
