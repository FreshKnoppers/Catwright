# ACP (Agent Client Protocol): what Catwright needs from `vibe-acp`

Verified on 2026-11-07 against Mistral Vibe 3.1.0 (`vibe-acp`, Python
`agent-client-protocol` 0.11.0, schema v1.16.0, `protocolVersion` 1).

Real wire transcripts (the source of truth for tests):

- `app/tests/fixtures/acp/handshake.jsonl`: initialize, session/new,
  session/list, session/close (no model call).
- `app/tests/fixtures/acp/prompt_session.jsonl`: initialize, session/new,
  session/set_mode `ask`, one session/prompt with thoughts, text, tool calls
  (bash, write_file with diff, read), plan updates, one
  session/request_permission, then session/list, session/close and a
  session/load replay of the same session.

Fixture format: one JSON object per line,
`{"dir":"c2a"|"a2c","msg":<JSON-RPC message>}` (`c2a` = client to agent,
`a2c` = agent to client), plus `{"dir":"marker","note":"..."}` lines that
separate phases. Paths are sanitized to `/tmp/acp-fixture/ws`.

## Transport

- Spawn `vibe-acp` (no arguments) as a child process. stdin/stdout carry
  JSON-RPC 2.0 messages, one JSON object per line (UTF-8, newline-delimited,
  no Content-Length headers).
- stderr is free-form log output. Drain it continuously on its own thread
  (otherwise the pipe fills up and the agent blocks). Never parse it as
  protocol. Keep only a bounded tail (e.g. last 64 KiB) for error messages.
- Both sides send requests. The client sends requests and notifications. The
  agent sends notifications (`session/update`) and requests
  (`session/request_permission`, possibly others).
- Request ids: the client uses its own increasing integers starting at 1.
  Agent request ids are independent (Vibe starts at **0**). Echo the agent's
  `id` value verbatim (number or string) in the response.
- Unknown agent-to-client request: reply
  `{"jsonrpc":"2.0","id":<id>,"error":{"code":-32601,"message":"Method not found"}}`.
  Unknown notification: ignore.
- Every object may carry `_meta`: ignore it unless noted. Ignore unknown
  fields. Missing optional fields must not fail parsing. An unknown
  `sessionUpdate` kind or content `type` must not fail parsing either (map it
  to an `Unknown` variant).

## Client to agent

| method | params | result |
|---|---|---|
| `initialize` | `{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,"elicitation":{"form":{}}},"clientInfo":{"name":"catwright","version":"0.2.0"}}` | `{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true,"audio":false,"embeddedContext":true},"sessionCapabilities":{"list":{},"fork":{},"close":{}}},"authMethods":[...],"agentInfo":{"name":"@mistralai/mistral-vibe","title":"Mistral Vibe","version":"3.1.0"}}` |
| `session/new` | `{"cwd":"/abs/path","mcpServers":[]}` | `{"sessionId":"...","modes"?:ModeState,"configOptions"?:[ConfigOption]}` (+ `_meta.workspace_trust.status`) |
| `session/load` | `{"cwd":"/abs/path","sessionId":"...","mcpServers":[]}` | `{"modes"?:ModeState,"configOptions"?:[ConfigOption]}`. **Before** this response arrives, the agent replays the whole history as `session/update` notifications (see Replay). |
| `session/resume` | same params as `session/load` | `{}`. Implemented by Vibe 3.1.x but **not advertised** in `sessionCapabilities`. For a session already open in the process it does nothing but re-send `usage_update` (verified with Vibe 3.1.2). The client sends it right after a model switch to get the new model's `size`; if it fails, the size stays unknown ("–") until the turn's next usage_update. |
| `session/fork` | `{"sessionId":"...","cwd":"/abs/path","mcpServers":[],"_meta"?:{"messageId":"..."}}` | Like `session/new`'s result. The history is not replayed; the client keeps its own copy (with the source's message ids removed: the fork gets its own). Without `messageId` the whole conversation forks; with one (a user message's id, in `_meta`) the fork ends after that message's turn. `forkSessionId` is rejected with `-32602`. Verified against Vibe 3.1.0. |
| `session/list` | `{"cwd"?:"/abs/path","cursor"?:"..."}` | `{"sessions":[{"sessionId","cwd","title"?,"updatedAt"?,"additionalDirectories"?}],"nextCursor"?}` |
| `session/prompt` | `{"sessionId":"...","prompt":[ContentBlock]}` | Resolves only when the whole turn ends: `{"stopReason":"end_turn"\|"max_tokens"\|"max_turn_requests"\|"refusal"\|"cancelled","usage"?:{"totalTokens","inputTokens","outputTokens","thoughtTokens"?,"cachedReadTokens"?,"cachedWriteTokens"?}}` |
| `session/cancel` | **notification** (no `id`): `{"sessionId":"..."}` | none. The pending `session/prompt` then resolves with `stopReason:"cancelled"`. |
| `session/set_mode` | `{"sessionId":"...","modeId":"ask"}` | `{}` |
| `session/set_config_option` | `{"sessionId":"...","configId":"model","value":"mistral-medium-3.5"}` | `{"configOptions":[ConfigOption]}` (the updated full list) |
| `session/close` | `{"sessionId":"..."}` | `{}` |

`ModeState` = `{"currentModeId":"accept-edits","availableModes":[{"id","name","description"?}]}`.
Vibe modes are its agent profiles: `ask`, `accept-edits` (default),
`smart-approve`, `auto-approve`, plus custom profiles from
`~/.vibe/agents/*.toml`.

`ConfigOption` (select) =
`{"id":"model","name":"Model","category"?:"model","description"?,"type":"select","currentValue":"mistral-large-4-0-3","options":[{"value","name","description"?}]}`.
Observed ids/categories: `mode`, `model` (values `glm-5-3`,
`mistral-large-4-0-3`, `zai-glm-5-3-flash`, `mistral-medium-3.5`),
`thinking` (`low`/`medium`/`high`). A boolean variant exists:
`"type":"boolean"` with a bool `currentValue` and no `options`.

### Errors

JSON-RPC error objects `{"code","message","data"?}`. Vibe codes:
`-32000` authentication required, `-32601` method not found, `-32602`
invalid params (also: session not found), `-32603` internal error,
`-31001` rate limited, `-31002` configuration error, `-31003` conversation
limit, `-31004` context too long, `-31005` refusal, `-31006` compaction
failed, `-31007` invalid image attachment, `-31008` images not supported.

Auth: Vibe uses the user's existing login (key in the system keyring /
browser sign-in), so the client never handles API keys. On `-32000`, tell the
user (German UI): "Vibe ist nicht angemeldet. Bitte einmal `vibe` im
Terminal starten und anmelden."

## Agent to client: `session/update` notifications

`{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"...","update":{"sessionUpdate":"<kind>", ...}}}`

| kind | fields | notes |
|---|---|---|
| `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk` | `content`: ContentBlock, `messageId`? | Streamed pieces. Consecutive chunks with the same `messageId` belong to one message (append text). Vibe ids: `assistant-<uuid>` for answer text, `reasoning-<uuid>-<n>` for thoughts. |
| `tool_call` | `toolCallId`, `title`, `kind`?, `status`?, `content`?: [ToolCallContent], `locations`?: [`{"path","line"?}`], `rawInput`?, `rawOutput`? | Creates a tool call. In a replay it already has its final status and content. |
| `tool_call_update` | `toolCallId` + any of the `tool_call` fields | Only present fields change. `content` / `locations` replace the old list when present. |
| `plan` | `entries`: [`{"content","priority":"high"\|"medium"\|"low","status":"pending"\|"in_progress"\|"completed"}`] | The full current plan. It replaces the previous one. |
| `available_commands_update` | `availableCommands`: [`{"name","description","input"?:{"hint"}}`] | Slash commands. To run one, send prompt text `/name args`. |
| `current_mode_update` | `currentModeId` | (not observed yet) |
| `config_option_update` | `configOptions`: [ConfigOption] | (not observed yet) |
| `session_info_update` | `title`?, `updatedAt`? | Session title for the sidebar. |
| `usage_update` | `used`, `size`, `cost`?: `{"amount","currency"}` | Context usage. `used` = tokens of the newest model call. `size` is Vibe's **auto-compaction threshold** (`runtime.context_window`, set from the compaction threshold in `vibe/app_server/_runtime.py`), not the model's window; the CLI footer shows the same number. Verified with Vibe 3.1.2: 400000 for Mistral Large 4 (window 1048576), 256000 for DeepSeek V4.1 Flash. The values come from Vibe's model catalogue and can change without a Vibe update: never hardcode them. A model switch (`session/set_config_option`) sends **no** usage_update; see `session/resume`. `_meta` has promptTokens / completionTokens / cachedTokens / totalTokens (optional, cumulative for the session). |
| `plan_update`, `plan_removed` | (schema only) | Ignore for now (Unknown). |

`kind` of a tool call: `read|edit|delete|move|search|execute|think|fetch|switch_mode|other`.
`status`: `pending|in_progress|completed|failed`.
Vibe puts `{"tool_name":"bash"|"write_file"|...,"effect_kind":"shell"|"file_write"|...}` into the tool call's `_meta`
(nice for icons, but optional).

ToolCallContent:
- `{"type":"content","content":ContentBlock}`
- `{"type":"diff","path","oldText"?:string|null,"newText"}` (no/null `oldText` = new file)
- `{"type":"terminal","terminalId"}`

ContentBlock:
- `{"type":"text","text"}`
- `{"type":"image","data":"<base64>","mimeType","uri"?}`
- `{"type":"resource_link","uri","name","mimeType"?,"title"?,"description"?,"size"?}`
- `{"type":"resource","resource":{"uri","text"?|"blob"?,"mimeType"?}}`
- `{"type":"audio",...}` (not used)

## Agent to client: `session/request_permission` (a REQUEST)

params:
`{"sessionId","toolCall":{"toolCallId", ...optional tool_call fields},"options":[{"optionId","name","kind":"allow_once"|"allow_always"|"reject_once"|"reject_always"}]}`

- Vibe sends only `toolCallId` inside `toolCall`. Title, kind and diff come
  from the earlier `tool_call` notification with the same id. The client
  must join them.
- Observed options: `allow_once` "Allow once", `allow_always` "Allow for
  remainder of this session", `allow_always_permanent` (kind
  `allow_always`) "Always allow", `reject_once` "Deny". Show `name`, send
  `optionId`. Never assume optionId == kind.
- Response: `{"outcome":{"outcome":"selected","optionId":"<id>"}}`, or
  `{"outcome":{"outcome":"cancelled"}}` when the user cancels the turn while
  the request is open.
- The agent blocks until it gets the response. The turn does not continue.

## Replay on `session/load`

Observed order: `session_info_update`, then per turn `user_message_chunk`
(whole text), `agent_thought_chunk` (whole), `agent_message_chunk` (whole),
`tool_call` (final status and content, no separate update), `plan`, ...,
`usage_update`. Then the `session/load` response. Then
`available_commands_update`.

## What Catwright does NOT advertise (v1)

`fs/*` and `terminal/*` client capabilities stay off. Vibe then executes file
and shell tools itself. Requests for these methods get the `-32601` reply.

## Vibe's extension methods (verified 2026-11-08 against vibe-acp 3.1.0)

ACP extensions start with `_` on the wire. All of these exist in
`vibe/acp/agent.py` (`ext_method`); the ones marked "live" were also exercised
end to end against the real binary with an isolated `VIBE_HOME` and a local
scripted model. Parsing is tolerant (see `harness/extras.rs`).

| method | params | result |
|---|---|---|
| `_session/set_title` (live) | `{sessionId,title}` (also for a saved, unopened session) | `{}`, plus a `session_info_update` |
| `_session/delete` (live) | `{sessionId}` | `{}` |
| `_rewind/preview` (live) | `{sessionId,messageId}` | `{"paths":[...]}` |
| `_rewind/to` (live) | `{sessionId,messageId,restoreFiles}` | `{"messageContent","restoreErrors":[],"restoredPaths":[]}`. **No notifications follow**: the client cuts its transcript itself. Message ids of a fork's copied history do not work here. |
| `_account/read` (live) | `{sessionId}` | `{"status","plan":{"kind","name","title"},"budget":{"usagePercentage","resetAt","paygEnabled"},"apiKey":{...}}`. Calls Mistral; never read `apiKey`. |
| `_identity/read` (live) | `{sessionId}` | `{"email","firstName","lastName","workspace":{"name"},"organization":{"name"}}` |
| `_trust/decision` (live) | `{sessionId,cwd,decision}`: `trust_cwd`, `trust_repo`, `decline` | the resulting state, `{"trust_status","details"}` (trust itself arrives via `_meta.workspace_trust`). Fails with `-32602` when no decision is available (`details: null`, nothing to trust). |
| `_loops/list` / `_loops/create` / `_loops/delete` / `_loops/clear` (live) | `{sessionId}` / `+{interval,prompt}` (`"30s"`, `"5m"`, `"2h"`, `"1d"`; no cron: `interval` goes through `parse_interval`, and the ACP request has no `cron` field, verified in 3.1.3) / `+{loopId}` / `+{sessionId}` | `{"loops":[{id,prompt,intervalSeconds,cron?,nextFireAt}]}` / `{"loop":...}`. Loops run only while the session is live and idle. |
| `_review/state` (live) | `{sessionId}` | `{"files":[{path,status,regions:[{decision,...}]}],"scopes":[...]}`; `decision` is `pending`/`keep`/`revert` |
| `_review/approve`, `_review/revert` (live) | `{sessionId,target}` with `target` = `{"kind":"all"}`, `{"kind":"file","path"}`, `{"kind":"lastTurns","count"}`, … | `{}`. `revert` changes the working tree. |
| `_connectors/list`, `_connectors/toggle` (list live) | `{sessionId}` / `+{name,disabled,toolName?}` | `{"connectors":[{name,enabled,reachability,tools:[...],error}],"error"}` |
| `_session/retrying` (notification) | | `{sessionId,category,detail}`, `category` = `rate_limited`, `server_error`, `timed_out`, `connection`, `unknown` |

`session/new` and `session/load` results carry `_meta.workspace_trust`:
`{"status":"untrusted"|"trusted","details":null|{"detectedFiles":[...],"repoDetectedFiles":[...],"availableDecisions":[...],...}}`.
Vibe loads AGENTS.md, `.vibe/`, skills and hooks of a project only when it is
trusted.

Also used: `_review/turnDiff` with `{sessionId,path,owner:{turnId}}`, answering
`{status,baseline,current}` (one file's content before and after a turn; the
turn id is the user message id). It feeds the per-round file changes in the
transcript.

Not used yet: `_voice/*`, `_projectLinks/*`, `_auth/*`, `_review/hunks`,
`_logLevel/*`.

## Agent to client: `elicitation/create` (a REQUEST, verified live)

Sent for Vibe's `ask_user_question` tool, which exists only when the client
advertises `elicitation.form`.

```json
{"sessionId":"...","toolCallId":"...","message":"User input required","mode":"form",
 "requestedSchema":{"required":["q0"],"description"?:"footer note","properties":{
   "q0":{"type":"string","title"?:"header","description":"question","oneOf":[{"const":"CSV","title":"CSV"}]},
   "q1":{"type":"array","description":"question","items":{"anyOf":[{"const":"x","title":"x"}]}}}}}
```

Response: `{"action":"accept","content":{"q0":"CSV","q1":["x"]}}` (values outside
the options are accepted: free text), `{"action":"decline"}` or
`{"action":"cancel"}`.

## Subagents (verified 2026-11-07, `app/tests/fixtures/acp/subagent_session.jsonl`)

Vibe runs subagents through tools inside its `run_typescript` flow. Over ACP
they show up as ordinary tool calls:

1. `tool_call`, `kind: "think"`, `status: "in_progress"`, `title` = the name
   the agent gave the child (e.g. "Starting reader"),
   `rawInput: {"task": "<goal>", "agent": "generic"}`,
   `_meta: {"tool_name": "agents.spawn", "effect_kind": "subagent", "agent", "task"}`.
2. `tool_call_update` for the same id, `status: "completed"` as soon as the
   child has **started** (content text "Started reader"); `_meta` now also has
   `child_session_id` (e.g. `child-f5de…`). If the spawn fails, the status
   is `failed`. (Vibe's code can also put `turn_count` and `response` into
   `_meta` when the effect carries an output. Treat them as optional.)
3. The parent then waits with a separate tool call
   (`_meta.tool_name: "self.wait"`, title "Waiting for input"). It completes
   when the child has delivered its result. The child's own steps are NOT
   streamed to the client.

Child sessions cannot be opened: `session/load` with a child id fails with
`-31002` ("A subagent Session cannot be resumed independently"), and they do
not appear in `session/list`. A client can only show goal, agent, status and
the parent's waiting state.

Worktree sessions (Vibe `--worktree`, or delegated runs) have their own
`cwd` under `~/.vibe/worktrees/...`. `session/list` with a project `cwd`
returns only sessions whose `cwd` is exactly that directory.
