---
name: codebase-memory-mcp-exploring
description: "Use when the user asks how code works, wants to understand architecture, trace execution flows, or explore unfamiliar parts of the codebase. Examples: \"How does X work?\", \"What calls this function?\", \"Show me the auth flow\""
---

# Exploring Codebases with codebase-memory-mcp

## When to Use

- "How does authentication work?"
- "What's the project structure?"
- "Show me the main components"
- "Where is the database logic?"
- Understanding code you haven't seen before

## Prerequisites

Every tool is scoped by a `project` name. Get it from `list_projects` — names are the
sluggified repo path (e.g. `/home/lszio/Projects/zio` → `home-lszio-Projects-zio`).
If the project is absent, index it first: `index_repository({repo_path: "<abs path>"})`.

## Workflow

```
1. list_projects()                        → confirm the project name + freshness
2. index_status({project})                → readiness; re-index if stale
3. search_graph({query: "<concept>"})     → find symbols (use INSTEAD of grep)
4. get_architecture({project})            → packages, services, dependencies at a glance
5. get_code_snippet({project, qualified_name}) → source for the symbol you care about
6. trace_path({project, function_name, mode: "calls"}) → callers/callees
```

## Tools

**search_graph** — find symbols for a concept. BM25 full-text; identifiers are
indexed per word (`updateCloudClient` → update, cloud, client). Results include
`qualified_name` — pass it to `get_code_snippet`. Structural boosting prefers
Functions/Methods (+10), Routes (+8), Classes/Interfaces (+5).

```
search_graph({project, query: "payment processing"})
search_graph({project, name_pattern: "auth.*engin"})     # exact-name match
search_graph({project, semantic_query: ["send", "pubsub"]})  # array of keywords, requires full/moderate index
```

**get_code_snippet** — read source for one symbol:

```
get_code_snippet({project, qualified_name: "home-lszio-Projects-zio.core.parser.parse"})
```

**get_architecture** — structure/dependency overview without reading every file.

**query_graph** — raw Cypher for multi-hop patterns:
`get_graph_schema({project})` first, then
`query_graph({project, query: "MATCH (a)-[:CALLS]->(b) RETURN ..."})`.

## Example: "How does payment processing work?"

```
1. list_projects / index_status
2. search_graph({project, query: "payment processing"})
   → processPayment, validateCard, chargeStripe (qualified_names + files)
3. trace_path({project, function_name: "processPayment", direction: "both"})
   → who calls it, what it calls
4. get_code_snippet({project, qualified_name: "<…>.processPayment"})
5. Read the source files for implementation details
```

> Graph first; `search_code` (text search) only when the graph comes up empty or for
> patterns the index can't see (macros, dynamic dispatch, generated code).
