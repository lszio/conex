---
name: codebase-memory-mcp-guide
description: "Reference for the codebase-memory-mcp MCP server: every tool, its parameters, and when to use it. Read this before writing custom tool calls"
---

# codebase-memory-mcp — Tool Reference

Server: `codebase-memory-mcp` (v0.10.0). 14 tools, no resources.
Every project-scoped tool requires `project` — the exact name from `list_projects`
(sluggified repo path: `/home/lszio/Projects/zio` → `home-lszio-Projects-zio`).

| Tool | Purpose | Key params |
| --- | --- | --- |
| `index_repository` | Index a repo into the graph | `repo_path` (req), `mode` (full/moderate/fast/cross-repo-intelligence), `persistence` (writes `.codebase-memory/graph.db.zst` into the repo for team sharing), `target_projects` (cross-repo mode) |
| `list_projects` | All indexed projects | — |
| `index_status` | Index freshness | `project` |
| `delete_project` | Drop a project index | `project` |
| `search_graph` | Find symbols (BM25, use instead of grep) | `project` (req), `query` \| `name_pattern` \| `qn_pattern` \| `file_pattern`, `label`, `semantic_query` (array), `min_degree`/`max_degree`, `include_connected`, `exclude_entry_points`, `limit` |
| `get_code_snippet` | Source of one symbol | `project`, `qualified_name`, `include_neighbors` |
| `trace_path` | Callers/callees, data flow, cross-service | `project`, `function_name` (req), `direction` (inbound/outbound/both), `depth` (3), `mode` (calls/data_flow/cross_service), `parameter_name`, `risk_labels`, `include_tests`, `edge_types` |
| `get_architecture` | Package/service/dependency overview | `project`, `aspects` |
| `search_code` | Graph-augmented grep | `project`, `pattern`, `regex`, `file_pattern`, `path_filter`, `mode` (compact/full/files), `context`, `limit` (10) |
| `query_graph` | Custom Cypher | `project`, `query` (Cypher), `max_rows` |
| `get_graph_schema` | Node labels / edge types (before Cypher) | `project` |
| `detect_changes` | Impact of working-tree changes | `project`, `scope`, `depth` (2), `base_branch` (main), `since` (ref/date) |
| `manage_adr` | Architecture Decision Records | `project`, `mode` (get/update/sections), `content`, `sections` |
| `ingest_traces` | Ingest runtime traces into the graph | — |

## Behavior notes (verified)

- `search_graph({query})` BM25 tokenizes camelCase; structural boost Functions/Methods
  +10, Routes +8, Classes/Interfaces +5; noise labels filtered. Results carry
  `qualified_name`, `label`, `file_path`, `start_line`. `semantic_query` MUST be an
  array (`["send","pubsub"]`), requires full/moderate index; results land in the
  separate `semantic_results` field.
- `trace_path` with `risk_labels: true` classifies each hop CRITICAL/HIGH/MEDIUM/LOW
  by distance. Test files are filtered unless `include_tests: true`.
- `search_code` reports `total_grep_matches` vs `total_results` — differing means
  truncation; narrow with `file_pattern`/`path_filter`, raise `limit` (no offset).
- `index_repository` modes all run type-aware LSP call/usage resolution;
  `fast` skips similarity/semantic edges, `moderate` filters files.
  `cross-repo-intelligence` skips extraction — indexes must already exist and
  `target_projects: ["*"]` matches all.
- `detect_changes` compares the working tree; `since` takes a git ref or date
  (`HEAD~5`, `v0.5.0`, `2026-01-01`). A clean tree returns `changed_count: 0`.
- Data lives in `~/.local/share/codebase-memory-mcp/` (global, not in the repo)
  unless `persistence: true` writes a shared artifact to the repo.
