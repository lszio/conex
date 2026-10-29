---
name: codebase-memory-mcp-impact-analysis
description: "Use when the user asks about the blast radius of a change — \"What breaks if I change X?\", \"What depends on this?\", impact review before editing, or pre-commit change analysis"
---

# Impact Analysis with codebase-memory-mcp

## When to Use

- "What breaks if I change X?"
- "Who calls this function?"
- Pre-edit blast-radius check (mandatory per AGENTS.md)
- Pre-commit change analysis

## Workflow (pre-edit)

```
1. search_graph({project, query: "<symbol>"})   → exact qualified_name
2. trace_path({project, function_name, direction: "inbound", mode: "calls", risk_labels: true})
   → callers with per-hop CRITICAL/HIGH/MEDIUM/LOW risk
3. trace_path({project, function_name, direction: "outbound"})  → what it calls
4. Report callers, depth, and CRITICAL/HIGH risk to the user before editing
```

## The gate

- **MUST run before editing any function/class/method.** Never substitute grep for
  graph analysis — `search_graph` + `trace_path` resolve type-aware call edges.
- **MUST warn the user** on CRITICAL/HIGH risk before proceeding.
- **Treat unresolvable symbols as unresolved, not safe.** A symbol missing from the
  graph can mean the index missed it (macros, dynamic dispatch, `rsx!`-style macro
  expansion, generated code). Confirm with `search_code({pattern: "<symbol>", project})`
  — an empty caller set is not evidence the symbol is unused.

## Verify: no accidental impact

Call `trace_path` again in `direction: "outbound"` on your edited symbol to confirm
the call surface didn't grow unintentionally.

## Detect changes before commit

```
detect_changes({project})                       # default: compare working tree against main
detect_changes({project, since: "HEAD~5"})      # arbitrary ref/date to compare from
detect_changes({project, since: "dev"})         # integration branch
```

Response: `changed_files`, `changed_count`, `impacted_symbols`.

- Zero `changed_files` on a dirty tree means the index didn't see the edits — re-run
  `index_repository` first, then `detect_changes` again. A zero on a stale index is
  unseen, not unaffected.
- Cross-service blast radius: `trace_path({mode: "cross_service"})` follows
  HTTP/async call edges through Routes.

## Risk labels

`risk_labels: true` adds CRITICAL/HIGH/MEDIUM/LOW based on hop distance —
closer callers are riskier. Never waive a CRITICAL/HIGH finding without evidence.
