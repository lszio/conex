---
name: codebase-memory-mcp-cli
description: "Index lifecycle for codebase-memory-mcp: indexing repos (modes, cross-repo), status, cleanup, and sharing the index artifact. Read this when a project is missing, stale, or needs removal"
---

# codebase-memory-mcp — Index Lifecycle

## Status & lookup

```
list_projects()             → all indexed projects (name, node/edge counts)
index_status({project})     → ready / indexing; node + edge counts
```

Project names are sluggified absolute paths (`/home/lszio/Projects/conex` →
`home-lszio-Projects-conex`). Always confirm the exact name with `list_projects`;
guessing the name fails with "project not found or not indexed".

## First-time index

```
index_repository({repo_path: "/home/lszio/Projects/<repo>"})
```

- Default `mode: "full"` — all files + similarity/semantic edges. All modes run
  type-aware LSP call/usage resolution (per-file + cross-file).
- Large repos: `mode: "moderate"` (filtered files) or `mode: "fast"` (no
  similarity/semantic) for a quicker first pass; re-index `full` when needed.
- Index staleness is cheap to check: run `index_status` before trusting
  `detect_changes`/`search_graph` results; re-index after big rebases or bulk moves.

## Re-index

Same call as first-time index — `index_repository` upserts.

## Cross-repo intelligence

```
index_repository({repo_path: "<this repo>", mode: "cross-repo-intelligence",
                  target_projects: ["*"]})
```

Skips extraction; matches Routes/Channels across projects and adds
CROSS_HTTP_CALLS / CROSS_ASYNC_CALLS / CROSS_CHANNEL edges. Target projects must
already be indexed (run their `index_repository` first).

## Sharing / team bootstrap

```
index_repository({repo_path, persistence: true})
```

Writes a compressed artifact `.codebase-memory/graph.db.zst` into the repo —
teammates start from it instead of a full re-index. Makes the artifact visible in
`git status`; it is regenerable, so gitignore it in-tree unless versions matter.

## Cleanup

```
delete_project({project})   → drop an index entirely (e.g. obsolete worktrees)
```

The default store is global (`~/.local/share/codebase-memory-mcp/`), so
deleting an index never touches the repo's working tree.

## Failure modes

- `"project not found or not indexed"` → wrong name; use `list_projects`.
- Stale results after edits → `index_repository` again before `detect_changes`.
- Unresolvable symbols → index blind spot (macros, generated code); confirm with
  `search_code` instead of assuming the symbol is unused.
