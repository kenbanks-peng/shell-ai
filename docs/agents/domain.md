# Domain Docs

How engineering skills consume domain documentation.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root, or
- **`CONTEXT-MAP.md`** at the repo root if it exists — it points at one `CONTEXT.md` per context. Read each one relevant to the topic.
- **`docs/adr/`** — read ADRs that touch the area you are about to work in. In multi-context repos, also check `src/<context>/docs/adr/` for context-scoped decisions.

If these files do not exist, proceed silently. The `/domain-modeling` skill creates them when terms or decisions get resolved.

## File structure

Single-context repo:
```
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Use the glossary's vocabulary

Use terms defined in `CONTEXT.md`; do not substitute terms the glossary avoids. If a required concept is absent, reconsider the terminology or note the gap for `/domain-modeling`.

## Flag ADR conflicts

Surface any contradiction with an existing ADR explicitly.
