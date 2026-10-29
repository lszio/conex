---
name: codebase-memory-mcp-debugging
description: "Use when tracing bugs, investigating unexpected behavior or regressions — \"Why is X failing?\", \"What changed to break this?\", \"Where does this value come from?\""
---

# Debugging with codebase-memory-mcp

## When to Use

- "Why is X failing?"
- "What changed to break this?"
- Tracing how a value/argument flows through the code

## Workflow

```
1. search_graph({project, query: "<failing symbol or concept>"})   → locate candidates
2. trace_path({project, function_name, mode: "data_flow", direction: "both"})
   → value propagation with the argument expression at each hop
3. trace_path({project, function_name, mode: "data_flow", parameter_name: "<arg>"})
   → scope the trace to one parameter
4. Narrow the suspect window: detect_changes({project, since: "<last known good ref>"})
   → which symbols changed recently
5. search_code({project, pattern, context: 3})  → verify index gaps by text
```

## Tools

**trace_path** — the primary tool.

| mode | what it follows |
| --- | --- |
| `calls` | CALLS edges (callers/callees) |
| `data_flow` | CALLS + DATA_FLOWS with arg expressions at each hop |
| `cross_service` | HTTP_CALLS + ASYNC_CALLS + DATA_FLOWS through Routes |

- `direction: "inbound"` = who feeds the symbol; `"outbound"` = what it feeds;
  `"both"` (default) = full path.
- `depth` (default 3) caps hop distance — raise for deep chains.
- `include_tests: true` to see test callers (filtered out by default).

**search_code** — graph-augmented grep: runs a real grep, then enriches matches with
graph context (dedupes, ranks, reveals connectivity you can't see in raw grep output).

```
search_code({project, pattern: "TODO|FIXME", regex: true, file_pattern: "*.rs"})
```

Response exposes `total_grep_matches` and `total_results` — if they differ, the
result set was truncated; narrow with `file_pattern`/`path_filter` or raise `limit`.

**ingest_traces** — feed runtime traces into the graph to correlate observed behavior
with call paths when static tracing stalls.

## Example: "Why is the timeout firing?"

```
1. search_graph({project, query: "timeout"})          → timeout-related functions/routes
2. trace_path({project, function_name: "enforceDeadline", mode: "data_flow"})
3. detect_changes({project, since: "HEAD~10"})        → recently touched symbols
4. search_code({project, pattern: "deadline", path_filter: "src/.*\\.rs$"})
```

> Graph first; raw grep only for index blind spots (macros, generated/mirror code).
