---
name: catwright-ui
description: Load this skill when you answer inside the Catwright GUI (the user's prompt mentions Catwright UI blocks) and an interactive answer (table, chart, form, calculator, checklist, comparison) or a small tool with logic would help more than plain text.
---

# Catwright UI blocks

Catwright renders a fenced code block with the language tag `ui` as native
widgets instead of code. The block body is JSON: exactly one object or one
array of objects, no comments, no trailing commas. Use a `ui` block only when
an interactive or visual answer genuinely helps; otherwise answer with plain
Markdown. Never put HTML, JavaScript or CSS into a block.

Interactions come back to you as a normal user message starting with `[UI]`,
for example `[UI] Button "ok" clicked.` or
`[UI] Form "save" submitted: {"name":"Ada"}`. Sliders, number fields,
selects and checklists react locally without a message; their values are sent
with the next form submit of the same block.

## Nodes

Every node is an object with a `type` field (snake_case). `card` and `columns`
nest other nodes in `children`.

### text
```json
{"type":"text","text":"**Markdown** is allowed here."}
```

### heading
```json
{"type":"heading","level":2,"text":"Result"}
```

### button
Clicking sends `[UI] Button "<action>" clicked.`
```json
{"type":"button","label":"Show details","action":"details"}
```

### table
```json
{"type":"table","columns":["Model","Context"],"rows":[["Large","128k"],["Small","32k"]]}
```

### form
Fields have `kind` `text`, `number` or `checkbox`. Submitting sends
`[UI] Form "<submit>" submitted: {values}`.
```json
{"type":"form","submit":"save","fields":[
  {"name":"email","label":"E-Mail","kind":"text"},
  {"name":"seats","label":"Seats","kind":"number","value":"3"},
  {"name":"news","label":"Newsletter","kind":"checkbox","value":"true"}
]}
```

### bar_chart
```json
{"type":"bar_chart","title":"Revenue","bars":[{"label":"Q1","value":12.5},{"label":"Q2","value":18}]}
```

### line_chart
Up to 6 series, each with `[x, y]` points.
```json
{"type":"line_chart","title":"Growth","x_label":"Month","y_label":"Users",
 "series":[{"name":"Free","points":[[0,10],[1,14],[2,21]]},
           {"name":"Paid","points":[[0,2],[1,5],[2,9]]}]}
```

### pie_chart
```json
{"type":"pie_chart","title":"Share","slices":[{"label":"A","value":3},{"label":"B","value":1}]}
```

### progress
`value` is 0..1.
```json
{"type":"progress","label":"Migration","value":0.42}
```

### checklist
Locally checkable; the state stays in the message.
```json
{"type":"checklist","items":[{"text":"Write tests","done":true},{"text":"Update docs"}]}
```

### kv
Two-column key/value list.
```json
{"type":"kv","rows":[["Region","eu-central"],["Latency","12 ms"]]}
```

### divider
```json
{"type":"divider"}
```

### card
```json
{"type":"card","title":"Summary","children":[{"type":"text","text":"All good."}]}
```

### columns
```json
{"type":"columns","children":[{"type":"text","text":"left"},{"type":"text","text":"right"}]}
```

## Calculator: slider, number, select, computed

`slider`, `number` and `select` are inputs of the block; `computed` shows the
result of a formula over their values and updates live while the user drags —
no model round trip. All of them live in the same `ui` block.

```json
{"type":"card","title":"Loan","children":[
  {"type":"slider","name":"amount","label":"Amount","min":1000,"max":100000,"step":500,"value":25000},
  {"type":"slider","name":"rate","label":"Rate %","min":0.1,"max":10,"step":0.1,"value":3.5},
  {"type":"number","name":"years","label":"Years","value":10},
  {"type":"select","name":"kind","label":"Kind","options":["1","2"],"value":"1"},
  {"type":"computed","label":"Monthly payment","expr":"amount*(rate/100/12)/(1-(1+rate/100/12)^(-years*12))","decimals":2,"unit":"€"}
]}
```

Formula rules: numbers (`1.5`, `1e3`), the `name` of a slider/number/select of
the same block, `+ - * / ^ %`, unary minus, parentheses and the functions
`min`, `max` (2+ arguments), `round`, `floor`, `ceil`, `abs`, `sqrt`. A select
contributes its numeric option value, otherwise 0. Division by zero, unknown
names and syntax errors are shown as an error instead of a result.

## Inputs: toggle, text_input

```json
{"type":"toggle","name":"monthly","label":"Monatlich","value":true}
```
```json
{"type":"text_input","name":"title","label":"Titel","value":""}
```

## Tools with logic: ```ui-js

When a fixed `ui` block is not enough (lists the user can extend, values
that depend on each other, charts that react to inputs, small simulations),
write a fenced block with the language tag `ui-js` containing JavaScript.
Catwright runs it locally in a sandbox and paints what it returns with the
same nodes as above.

Contract:
- `function render(state)` (required) returns one node or an array of nodes.
- `function init()` (optional) returns the initial `state` object; without it
  `state` starts as `{}`.
- Inputs with a `name` (`slider`, `number`, `select`, `toggle`, `text_input`)
  are bound to `state[name]`: when the user changes one, the app sets the
  value and calls `render(state)` again. Pass `value: state.x` so inputs show
  the current state.
- `function onAction(action, state)` (optional) handles a `button` click
  locally when it returns the new state object. Returning nothing sends the
  click to you as `[UI] Button "<action>" clicked.`
- A `form` submit is sent to you as before, with the tool state under `"state"`.

Limits: plain JavaScript only — no DOM, no `console`, no network, no files,
no timers. About one million loop iterations and recursion depth 256 per call,
script at most 20 000 characters, result at most 2 000 nodes. The state is
capped at 64 KiB of JSON, and one answer (nodes plus the echoed state) stays
inside a 592 KiB (606 208 byte) budget. Errors are shown to the user instead
of the tool, so keep the code simple and robust.

Example: budget planner with an extendable list
```ui-js
function init() {
  return { items: [{ name: "Miete", amount: 900 }, { name: "Strom", amount: 80 }], months: 12 };
}
function render(state) {
  const total = state.items.reduce((s, i) => s + i.amount, 0);
  return [
    { type: "slider", name: "months", label: "Monate", min: 1, max: 24, step: 1, value: state.months },
    { type: "table", columns: ["Posten", "€ / Monat"], rows: state.items.map(i => [i.name, String(i.amount)]) },
    { type: "button", label: "+ Posten", action: "add" },
    { type: "kv", rows: [["Summe / Monat", total + " €"], ["Gesamt", total * state.months + " €"]] }
  ];
}
function onAction(action, state) {
  if (action === "add") { state.items.push({ name: "Neu", amount: 50 }); return state; }
}
```

Example: loan with a chart that follows the sliders
```ui-js
function init() { return { amount: 12000, rate: 4.9, years: 5 }; }
function render(s) {
  const r = s.rate / 1200, n = s.years * 12;
  const pay = r === 0 ? s.amount / n : s.amount * r / (1 - Math.pow(1 + r, -n));
  const pts = []; let rest = s.amount;
  for (let m = 0; m <= n; m++) {
    if (m % 12 === 0) pts.push([m / 12, Math.round(rest)]);
    rest = rest * (1 + r) - pay;
  }
  return { type: "card", title: "Kredit", children: [
    { type: "slider", name: "amount", label: "Betrag", min: 1000, max: 50000, step: 500, value: s.amount },
    { type: "slider", name: "rate", label: "Zins %", min: 0, max: 12, step: 0.1, value: s.rate },
    { type: "number", name: "years", label: "Jahre", value: s.years },
    { type: "kv", rows: [["Rate / Monat", pay.toFixed(2) + " €"]] },
    { type: "line_chart", title: "Restschuld", x_label: "Jahr", y_label: "€", series: [{ name: "Rest", points: pts }] }
  ] };
}
```

## Rules

- One `ui` or `ui-js` block per answer is usually enough; keep it small and readable.
- Prefer a plain `ui` block; use `ui-js` only when the tool needs logic.
- JSON only: no comments, no trailing commas, no single quotes.
- Labels are short; they follow the user's UI language (English by default
  — German labels fit a German UI).
- Do not repeat in prose what the block already shows.
