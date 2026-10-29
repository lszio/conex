---
name: codebase-memory-mcp-refactoring
description: "Use before and during refactors: renames, extractions, splitting functions, changing signatures — anything where a symbol's references must be enumerated first"
---

# Refactoring with codebase-memory-mcp

## When to Use

- Rename a function/class/method
- Extract or split a function
- Change a signature or behavior

## Never

- NEVER rename symbols with find-and-replace across the tree — that misses dynamic
  references and string-indirect usages. Enumerate every reference from the graph first.

## Before the refactor

```
1. search_graph({project, query: "<symbol>", include_connected: true})
   → exact qualified_name + connected nodes
2. trace_path({project, function_name, direction: "both", mode: "calls", include_tests: true})
   → every caller and callee, tests included
3. search_code({project, pattern: "<symbol>"})
   → text references the index can't see (comments, strings, dynamic dispatch)
4. Cross-service band: trace_path({mode: "cross_service"}) if the symbol is reachable
   from a Route/Channel
```

## After the refactor

```
detect_changes({project})   → confirm changed_files/impacted_symbols match the plan
```

Re-run `trace_path` on the renamed/extracted symbol and diff the caller set against
step 2 — any new caller is an unintended behavioral change.

## Extra: architecture before a big split

```
get_architecture({project, aspects: ["all"]})   → dependency picture
query_graph({project, query: "MATCH ..."})      → custom multi-hop queries
```

If the refactor embodies a decision worth keeping (and the project isn't already
covered), persist it with `manage_adr({project, mode: "update", content: "..."})`.
