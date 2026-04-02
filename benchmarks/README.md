# RemembrallMCP Benchmarks

A/B benchmarks measuring AI agent performance with and without RemembrallMCP on identical coding tasks.

## What we measure

| Metric | Why it matters |
|--------|---------------|
| Token usage | Direct cost savings - fewer exploration tokens = cheaper |
| Tool calls | Fewer calls = less latency and context overhead |
| Wall clock time | End-to-end speed improvement |
| Accuracy | Did the agent find all affected files, or miss cross-module impacts? |

## Running a benchmark

### 1. Set up the test repo

```bash
gh repo clone pallets/click -- --branch 8.1.7 --depth 1
```

### 2. Run WITHOUT RemembrallMCP

1. Disable RemembrallMCP in your `.mcp.json` (comment it out or remove it)
2. Restart your MCP client (Claude Code, Cursor, etc.)
3. Open a new conversation in the Click repo
4. Copy a prompt from `tasks.toml` and paste it exactly
5. Let the agent finish
6. Note the conversation ID (in Claude Code: visible in the conversation URL or file path)

### 3. Run WITH RemembrallMCP

1. Re-enable RemembrallMCP in `.mcp.json`
2. Restart your MCP client
3. Index the Click repo first:
   ```
   > "Index the Click project at /path/to/click with project name 'click'"
   ```
4. Open a new conversation
5. Paste the same prompt
6. Let the agent finish
7. Note the conversation ID

### 4. Record the runs

Add entries to `runs.json`:

```json
{
  "runs": [
    {
      "task_id": "blast-radius-invoke",
      "mode": "without",
      "conversation_id": "abc123...",
      "accuracy": "3/4 expected files found",
      "notes": "Missed testing.py reference"
    },
    {
      "task_id": "blast-radius-invoke",
      "mode": "with",
      "conversation_id": "def456...",
      "accuracy": "4/4 expected files found",
      "notes": "Single remembrall_impact call got everything"
    }
  ]
}
```

### 5. Generate the report

```bash
python benchmarks/analyze.py
```

The analyzer:
- Finds conversation JSONL files in `~/.claude/projects/`
- Extracts token usage, tool calls, turns, and wall clock time
- Generates a side-by-side comparison report in `benchmarks/reports/`

## Benchmark tasks

See `tasks.toml` for the 5 tasks. Each is chosen to highlight where the dependency graph saves the most tokens:

1. **Blast radius** - "What breaks if I change this function?"
2. **Find callers** - "Who calls format_help()?"
3. **Trace data flow** - "How does Context propagate through invocation?"
4. **Rename class** - "What files need changes to rename BaseCommand?"
5. **Add parameter** - "Add a deprecated flag to @command"

## Tips

- Always start a fresh conversation for each run (no prior context)
- Use the exact prompt from tasks.toml - don't rephrase
- For the WITH run, make sure the repo is indexed before starting
- Record accuracy by checking the agent's answer against `expected_files` and `expected_symbols` in tasks.toml
