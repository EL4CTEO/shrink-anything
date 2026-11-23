# Changelog

## 0.1.0

- Finds dead code in 19 languages, dead files, unused dependencies (npm, pnpm, yarn, bun, Cargo, pip, Go modules) and duplicated code.
- `cut` deletes what's dead and runs your check after every batch: what passes stays, what breaks is put back.
- `go --agent` hands duplicates and risky candidates to a coding agent and keeps its work only if the check agrees.
- `undo`, `report` (markdown + SVG card), `--size` to weigh a build output, MCP server, Claude Code plugin and skill.
