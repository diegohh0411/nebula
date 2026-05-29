---
name: nebula-notion-workflow
description: Use when picking up, implementing, or updating a Nebula task from Notion. Covers fetching tasks by ID, status transitions, branch/PR workflow, and post-merge property updates. Triggered by prompts like "pick up task TT-N", "implement task N", "mark task done", or "update task status".
---

# Nebula — Notion Task Workflow

## Prerequisites

Verify `ntn` CLI is available before doing anything else:

```bash
which ntn && ntn --version
```

If missing: install via `npm i -g notionhq-cli` and run `ntn login`.

## Key IDs (hardcoded — skip discovery)

| Resource | ID |
|---|---|
| Nebula page | `36fe954d-b476-802d-8601-c822900451ef` |
| Tasks database | `36fe954d-b476-8082-9b40-d2699511884a` |
| Tasks data source | `36fe954d-b476-8008-9aca-000b5ef89feb` |

## Fetch a Task by ID

When the user says "pick up task TT-3" or "implement task 3" — the number is always an integer:

```bash
# List all tasks (quick overview)
ntn datasources query 36fe954d-b476-8008-9aca-000b5ef89feb --json | \
  python3 -c "
import sys, json
d = json.load(sys.stdin)
for r in d['results']:
    p = r['properties']
    n = p['ID']['unique_id']['number']
    name = p['Name']['title'][0]['plain_text'] if p['Name']['title'] else ''
    status = p.get('Status', {}).get('status', {}).get('name', '')
    print(f'TT-{n}: {name} [{status}] — {r[\"id\"]}')
"

# Fetch a single task by number (replace 3 with the actual number)
ntn datasources query 36fe954d-b476-8008-9aca-000b5ef89feb \
  --filter '{"property":"ID","unique_id":{"equals":3}}' --json

# Get full page content (description, instructions, comments)
ntn pages get <page-id>
```

The `page-id` is the `id` field in the query result (UUID, no dashes needed).

## Status Lifecycle

```
Noticed → Detailed → In development → Ready for review
                                              ↓              ↓
                                     Changes requested → In development
                                              ↓
                                     Approved for merge → (human merges) → Merged
```

| Status | Meaning | Who acts next |
|---|---|---|
| **Noticed** | Logged, not yet detailed | Human |
| **Detailed** | Ready to implement | AI agent picks up |
| **In development** | Being built | AI agent |
| **Ready for review** | Implementation done, awaiting review | Human reviews |
| **Changes requested** | Human found issues | AI agent fixes |
| **Approved for merge** | Human approved | Human merges PR |
| **Merged** | PR is merged | AI agent updates final properties |

**Rules:**
- AI agents **never** merge PRs into `main`. Only the human does.
- All changes enter `main` via PRs only. Never push directly to `main`.
- When an implementer finishes, set status to **Ready for review** — nothing else.

## Update a Task Property

Get the page UUID from the datasource query first, then PATCH:

```bash
PAGE_ID=<uuid-from-query>

# Update status
ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[Status][status][name]=In development'

# Update PR number (integer)
ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[PR number][number]:=42'

# Update LOC added and removed
ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[LOC added][number]:=312' \
  'properties[LOC removed][number]:=87'
```

## Implementation Workflow (step by step)

1. **Fetch the task** — read name, description, instructions with `ntn pages get <id>`
2. **Set status → In development** via PATCH
3. **Use `superpowers:using-git-worktrees`** to create an isolated branch
4. **Use `superpowers:brainstorming`** before starting any non-trivial implementation
5. **Use `superpowers:writing-plans`** if the task is multi-step
6. **Use `superpowers:subagent-driven-development`** to execute the plan
7. **Open a PR** on GitHub, never push directly to main
8. **Set status → Ready for review**, record PR number:
   ```bash
   ntn api v1/pages/$PAGE_ID -X PATCH \
     'properties[Status][status][name]=Ready for review' \
     'properties[PR number][number]:=<PR_NUMBER>'
   ```

## Post-Merge Checklist (run after human merges)

Once the PR is merged, the AI agent must update these fields:

```bash
# Get LOC stats from the merged PR
gh pr view <PR_NUMBER> --json additions,deletions

# Then update Notion
ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[LOC added][number]:=<additions>' \
  'properties[LOC removed][number]:=<deletions>'
```

Status will already be **Merged** (set by the human or confirmed after merge). Verify it:

```bash
ntn datasources query 36fe954d-b476-8008-9aca-000b5ef89feb \
  --filter '{"property":"ID","unique_id":{"equals":<N>}}' --json | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d['results'][0]['properties']['Status']['status']['name'])"
```

## Common Mistakes

| Mistake | Fix |
|---|---|
| Using `collection://` prefix with `ntn datasources query` | Use bare UUID: `36fe954d-b476-8008-9aca-000b5ef89feb` |
| Passing task ID as a string in filter | Use `:=N` (integer), not `=N` (string) |
| Merging PR yourself | Never. Set status to **Ready for review** and stop. |
| Pushing commits directly to `main` | Always use a branch + PR. |
| Forgetting LOC update after merge | Always run post-merge checklist above. |
