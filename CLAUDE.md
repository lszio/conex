<!-- codebase-memory-mcp:start -->
# Codebase Memory — Code Intelligence

This project is indexed by **codebase-memory-mcp**. Project name (the `project` arg for every tool, confirm with `list_projects`): `home-lszio-Projects-conex`. Not indexed yet? Run `index_repository({repo_path: "/home/lszio/Projects/conex"})` once, then `index_status` to confirm readiness.

> Index stale? Run `index_status({project: "home-lszio-Projects-conex"})`; re-index with `index_repository` when it reports stale — always re-index before trusting `detect_changes`.

## Always Do

- **MUST run impact before editing.** `trace_path({function_name: "<symbol>", project: "home-lszio-Projects-conex", direction: "inbound", mode: "calls", risk_labels: true})` lists callers with per-hop risk; symbol not found → `search_graph({query: "<symbol>", project})` first to get its exact `qualified_name`. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** `detect_changes({project: "home-lszio-Projects-conex"})`; for regression review `detect_changes({project, since: "dev"})` (any ref/date works: `HEAD~5`, `v0.5.0`, `2026-01-01`). Zero `changed_files` on a dirty tree means the index didn't see the edits — re-index and re-run. A zero on a stale index is unseen, not unaffected.
- MUST warn on CRITICAL/HIGH risk from `trace_path` pre-edit; never waive a CRITICAL/HIGH finding without evidence.
- **MUST treat an unresolvable symbol as unresolved, not as safe.** A symbol missing from `search_graph` can mean the index missed it (macro expansion, dynamic dispatch, generated code). Confirm with `search_code({pattern: "<symbol>", project})` before treating it as unused or safe to change/delete.
- **MUST use the graph first for read-only questions** — callers, callees, dependencies, imports, flows: `search_graph` for symbols/concepts, `get_code_snippet` for symbol source, `trace_path` for paths, `get_architecture` for structure. Text search only when the graph comes up empty.
- Security review: `trace_path({mode: "data_flow"})` for value propagation, `mode: "cross_service"` for route/async reachability.

## Never Do

- NEVER edit a function, class, or method before graph impact analysis.
- NEVER ignore CRITICAL or HIGH risk labels from `trace_path`.
- NEVER rename symbols with find-and-replace — enumerate every reference with `search_graph` + `trace_path` first.
- NEVER commit before `detect_changes` on a fresh index.

## Skills

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/codebase-memory-mcp-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/codebase-memory-mcp-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/codebase-memory-mcp-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/codebase-memory-mcp-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/codebase-memory-mcp-guide/SKILL.md` |
| Index, status, projects, cleanup | `.claude/skills/codebase-memory-mcp-cli/SKILL.md` |

<!-- codebase-memory-mcp:end -->
