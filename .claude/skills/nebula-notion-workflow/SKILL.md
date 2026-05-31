---
name: nebula-notion-workflow
description: Use when picking up, implementing, or updating a Nebula task from Notion. Covers fetching tasks by ID, status transitions, branch/PR workflow, and post-merge property updates. Triggered by prompts like "pick up task TT-N", "implement task N", "mark task done", or "update task status".
---

# Nebula — Notion Task Workflow

## Prerequisites — pick your interface

Check which interface is available and use it consistently throughout the session:

```bash
which ntn && ntn --version && echo "USE_NTN" || echo "USE_MCP"
```

- **`ntn` available** → use the CLI patterns in this skill (preferred)
- **`ntn` missing** → fall back to the MCP tool patterns marked `[MCP]` below

If on a machine where you can install:
```bash
curl -fsSL https://ntn.dev | bash   # or: npm install --global ntn
ntn login
```

## Key IDs (hardcoded — skip discovery)

| Resource | ID |
|---|---|
| Nebula page | `36fe954d-b476-802d-8601-c822900451ef` |
| Tasks database | `36fe954d-b476-8082-9b40-d2699511884a` |
| Tasks data source | `36fe954d-b476-8008-9aca-000b5ef89feb` |
| User Stories database | `370e954d-b476-80cd-b7d6-e25a7c492349` |
| User Stories data source | `370e954d-b476-80e1-a5a3-000be9bb23ac` |
| Context Bits database | `370e954d-b476-80cc-8a33-d5d49a2b3a9b` |
| Context Bits data source | `370e954d-b476-8079-939d-000bc4380470` |

## Database Schemas

### 📜 User Stories

Tracks user stories and their test/coverage status.

| Property | Type | Values |
|---|---|---|
| `Name` | title | Free text |
| `ID` | auto_increment_id | Auto-assigned integer |
| `Status` | status | `Not covered` · `Partially covered` · `Covered` |

Query all user stories (CLI):
```bash
ntn datasources query 370e954d-b476-80e1-a5a3-000be9bb23ac --json
```

Query (MCP): `notion-fetch(id="collection://370e954d-b476-80e1-a5a3-000be9bb23ac")`

Update status (MCP):
```
notion-update-page(id="<page-id>", properties={"Status": {"status": {"name": "Partially covered"}}})
```

### 🍪 Context Bits

Freeform notes and context snippets (e.g. meeting notes, decisions).

| Property | Type | Values |
|---|---|---|
| `Name` | title | Free text |
| `Date` | date | ISO-8601 date or range |
| `Tags` | multi_select | `Meeting` (and others as added) |

Query all context bits (CLI):
```bash
ntn datasources query 370e954d-b476-8079-939d-000bc4380470 --json
```

Query (MCP): `notion-fetch(id="collection://370e954d-b476-8079-939d-000bc4380470")`

Create a new context bit (MCP):
```
notion-create-pages(
  parent={"database_id": "370e954d-b476-80cc-8a33-d5d49a2b3a9b"},
  properties={
    "Name": {"title": [{"text": {"content": "Note title"}}]},
    "Date": {"date": {"start": "2026-05-30"}},
    "Tags": {"multi_select": [{"name": "Meeting"}]}
  }
)
```

## Fetch a Task by ID

When the user says "pick up task TT-3" or "implement task 3" — the number is always an integer.

**[CLI]**
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

# Fetch a single task by number (replace 3 with the actual number — integer, not string)
ntn datasources query 36fe954d-b476-8008-9aca-000b5ef89feb \
  --filter '{"property":"ID","unique_id":{"equals":3}}' --json

# Get full page content (description, instructions, comments)
ntn pages get <page-id>
```

**[MCP]** Use `notion-search` with the data source URL, then `notion-fetch` for the full page:
```
# Step 1 — find the task (pass data_source_url to scope to Tasks only)
notion-search(
  query="TT-3",
  data_source_url="collection://36fe954d-b476-8008-9aca-000b5ef89feb"
)

# Step 2 — fetch full content by the page ID returned above
notion-fetch(id="<page-id-from-results>")
```

The `page-id` is the `id` field in the result (UUID). MCP search may not support `unique_id` filtering directly, so search by the task name or number and confirm the ID matches.

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

**[CLI]** Get the page UUID from the datasource query first, then PATCH:

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

**[MCP]** Use `notion-update-page` with the page ID and property patches:
```
notion-update-page(
  id="<page-id>",
  properties={
    "Status": {"status": {"name": "In development"}},
    "PR number": {"number": 42},
    "LOC added": {"number": 312},
    "LOC removed": {"number": 87}
  }
)
```
Pass only the properties you need to change; omit the rest.

## Implementation Workflow (step by step)

1. **Fetch the task** — read name, description, instructions (CLI: `ntn pages get <id>` / MCP: `notion-fetch(id=<id>)`)
2. **Set status → In development** using the update patterns above
3. **Use `superpowers:using-git-worktrees`** to create an isolated branch
4. **Use `superpowers:brainstorming`** before starting any non-trivial implementation
5. **Use `superpowers:writing-plans`** if the task is multi-step
6. **Use `superpowers:subagent-driven-development`** to execute the plan
7. **Open a PR** on GitHub, never push directly to main
8. **Set status → Ready for review** and record PR number (use whichever update pattern applies)

## Post-Merge Checklist (run after human merges)

Get LOC stats from the merged PR, then update Notion:

```bash
# Works in both environments — gh CLI is standard
gh pr view <PR_NUMBER> --json additions,deletions
```

Then write back (CLI):
```bash
ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[LOC added][number]:=<additions>' \
  'properties[LOC removed][number]:=<deletions>'
```

Or (MCP):
```
notion-update-page(id="<page-id>", properties={"LOC added": {"number": <additions>}, "LOC removed": {"number": <deletions>}})
```

Verify the status is **Merged** — CLI:
```bash
ntn datasources query 36fe954d-b476-8008-9aca-000b5ef89feb \
  --filter '{"property":"ID","unique_id":{"equals":<N>}}' --json | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d['results'][0]['properties']['Status']['status']['name'])"
```
MCP: `notion-fetch(id="<page-id>")` and read the Status property.

## Creating Pages via `ntn api` (properties + content)

`ntn pages create` only handles Markdown content; it cannot set properties like `Status` or `ID`. To create a task with both properties and a body, use `ntn api v1/pages` with a full JSON payload.

**CRITICAL: `ntn api` takes JSON via stdin redirect, NOT `-d @file`.**

```bash
# CORRECT: pipe JSON via stdin
cat > /tmp/new_task.json <<'JSON'
{
  "parent": {"database_id": "36fe954d-b476-8082-9b40-d2699511884a"},
  "properties": {
    "Name": {"title": [{"text": {"content": "Task title"}}]},
    "Status": {"status": {"name": "Detailed"}}
  }
}
JSON
ntn api v1/pages < /tmp/new_task.json

# WRONG: -d @file is NOT supported by ntn api
# ntn api v1/pages -d @/tmp/new_task.json   # <-- FAILS
```

**Body must come from exactly one source.** Do not mix `--data`, stdin, and inline inputs in the same call.

## Updating Page Content

`ntn pages update` takes Markdown via `--content '<markdown>'` or stdin, **not** `--content-file`.

```bash
# CORRECT: inline content
ntn pages update <page-id> --content '# Title\n\nBody text'

# CORRECT: stdin redirect
ntn pages update <page-id> < body.md

# WRONG: --content-file does not exist
# ntn pages update <page-id> --content-file body.md   # <-- FAILS
```

## Verifying Chained Operations

When creating resources that downstream steps depend on, **always verify the create-step output before proceeding.**

**Pattern — create then verify:**
```bash
# Step 1: create and capture raw output
ntn api v1/pages < /tmp/t1.json > /tmp/r1.json 2>&1
echo "create exit=$?"

# Step 2: parse and confirm the ID is real
python3 -c "
import json, sys
d = json.load(open('/tmp/r1.json'))
if 'id' in d:
    print('OK', d['id'])
else:
    print('FAIL', d.get('message', 'unknown error'))
    sys.exit(1)
"

# Step 3: only now use the ID in a PATCH
t1_id=$(python3 -c "import json; print(json.load(open('/tmp/r1.json'))['id'])")
ntn api v1/pages/$t1_id -X PATCH 'properties[Blocked by][relation][0][id]=<other-id>'
```

## Common Mistakes

| Mistake | Fix |
|---|---|
| Using `collection://` prefix with `ntn datasources query` | Use bare UUID: `36fe954d-b476-8008-9aca-000b5ef89feb` |
| Passing task ID as a string in filter | Use `:=N` (integer), not `=N` (string) |
| Mixing CLI and MCP in the same session | Pick one at the start and stick with it |
| Using `notion-search` with the database URL instead of data source URL | Pass `collection://36fe954d-b476-8008-9aca-000b5ef89feb` as `data_source_url` |
| **Using `-d @file` with `ntn api`** | Use stdin redirect: `ntn api v1/pages < file.json` |
| **Using `--content-file` with `ntn pages update`** | Use `--content '<md>'` or stdin: `ntn pages update <id> < file.md` |
| **Assuming a create succeeded and using a fabricated ID** | Always parse the create response and verify `'id'` exists |
| **Running dependent calls in parallel** | Run sequentially; a single failure cascades and cancels the rest |
| Merging PR yourself | Never. Set status to **Ready for review** and stop. |
| Pushing commits directly to `main` | Always use a branch + PR. |
| Forgetting LOC update after merge | Always run post-merge checklist above. |
