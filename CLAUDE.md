@RTK.md

# Tooling discipline
- NEVER issue filler/no-op commands (e.g. `echo p1`, `echo probe`, repeated `sleep`) to "flush" or poll for delayed tool output. If a tool result comes back empty or the harness seems laggy, wait for the real result or re-issue the single substantive command once — do not spam. Wasting tokens on probe commands is not acceptable.

## Error-handling discipline
- AFTER any command that creates a resource (Notion page, git branch, API call),
  immediately verify success by inspecting the actual output/ID before using it
  in the next command.
- NEVER fabricate IDs or assume a previous step succeeded because the next step
  didn't error — check the create-step output explicitly.
- When steps have dependencies, run them SEQUENTIALLY, not in parallel.
  A cascade-cancelled batch wastes more tokens than waiting.
- If a foundational step fails (e.g., `ntn api` returns an error), STOP. Do not
  proceed with downstream steps until the root failure is understood and fixed.
