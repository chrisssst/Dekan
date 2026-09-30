# AGENTS.md — Dekan

> **Pointer file.** The canonical context for any AI agent working on this project is **`CLAUDE.md`** at the
> repository root. Read it first. This file only exists so tools that look for `AGENTS.md` find the way there;
> keeping two full copies made them drift apart in the past.

## Start here

| Need | Where |
| --- | --- |
| What the project is, critical rules, key decisions | `CLAUDE.md` |
| Architecture, flows, build and CI, logs, security | `docs/` |
| What a crate does and which file holds what | `crates/<crate>/README.md` |
| Build, packaging and diagnostic commands | `xtask/README.md` |
| User-facing overview | `README.md` |

## Two rules that cost the project the most so far

1. **A green `cargo test` is not proof that a feature works.** Anything that touches the outside world (a
   process, the registry, the League client, a binary format, the game) is only done with evidence collected
   outside the Dekan process, ideally a real match and its log.
2. **Filtered command output is not evidence.** If a proxy such as `rtk` filters output, run the raw command
   (`rtk proxy <cmd>`) or use the agent's own search tools before claiming a count or a result.
