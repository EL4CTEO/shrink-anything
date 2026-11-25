# Changelog

## 0.1.1

- Fixed: names with non-ASCII letters (`résumé_title`, `dаta_format`) were reported dead and cut while still called. Mentions are now split exactly where definitions are (Unicode `\w`) ([#1](https://github.com/EL4CTEO/shrink-anything/issues/1)).
- Dynamic name prefixes and folder paths with non-ASCII letters now keep what they reach.

## 0.1.0

- Finds dead code in 19 languages, dead files, unused dependencies (npm, pnpm, yarn, bun, Cargo, pip, Go modules) and duplicated code.
- `cut` deletes what's dead and runs your check after every batch: what passes stays, what breaks is put back.
- `go --agent` hands duplicates and risky candidates to a coding agent and keeps its work only if the check agrees.
- `undo`, `report` (markdown + SVG card), `--size` to weigh a build output, MCP server, Claude Code plugin and skill.
