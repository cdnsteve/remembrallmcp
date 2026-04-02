#!/usr/bin/env python3
"""
RemembrallMCP Benchmark Analyzer

Parses Claude Code conversation logs and compares agent performance
with and without RemembrallMCP on identical coding tasks.

Usage:
    python benchmarks/analyze.py
    python benchmarks/analyze.py --report benchmarks/reports/run-001.md

Reads runs.json for conversation mappings, finds JSONL logs in
~/.claude/projects/, and generates comparison reports.
"""

import json
import os
import sys
from pathlib import Path
from datetime import datetime

BENCHMARKS_DIR = Path(__file__).parent
RUNS_FILE = BENCHMARKS_DIR / "runs.json"
TASKS_FILE = BENCHMARKS_DIR / "tasks.toml"
CLAUDE_DIR = Path.home() / ".claude" / "projects"


def find_conversation_jsonl(conversation_id: str) -> Path | None:
    """Search ~/.claude/projects/ for a conversation JSONL file."""
    for project_dir in CLAUDE_DIR.iterdir():
        if not project_dir.is_dir():
            continue
        conv_dir = project_dir / "conversations"
        if not conv_dir.exists():
            continue
        jsonl = conv_dir / f"{conversation_id}.jsonl"
        if jsonl.exists():
            return jsonl
    return None


def parse_conversation(jsonl_path: Path) -> dict:
    """Extract metrics from a Claude Code conversation JSONL."""
    messages = []
    with open(jsonl_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                messages.append(json.loads(line))
            except json.JSONDecodeError:
                continue

    total_input = 0
    total_output = 0
    total_cache_read = 0
    total_cache_creation = 0
    tool_calls = 0
    turns = 0
    first_ts = None
    last_ts = None

    for msg in messages:
        # Extract timestamp
        ts = msg.get("timestamp")
        if ts:
            if first_ts is None:
                first_ts = ts
            last_ts = ts

        # Only count assistant messages for usage
        if msg.get("role") != "assistant":
            continue

        turns += 1

        # Extract token usage
        usage = msg.get("usage", {})
        if not usage:
            # Sometimes nested under message
            usage = msg.get("message", {}).get("usage", {})

        total_input += usage.get("input_tokens", 0)
        total_output += usage.get("output_tokens", 0)
        total_cache_read += usage.get("cache_read_input_tokens", 0)
        total_cache_creation += usage.get("cache_creation_input_tokens", 0)

        # Count tool calls in content blocks
        content = msg.get("content", [])
        if isinstance(content, list):
            for block in content:
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    tool_calls += 1

    # Wall clock time
    wall_clock_s = 0
    if first_ts and last_ts:
        try:
            t0 = datetime.fromisoformat(first_ts.replace("Z", "+00:00"))
            t1 = datetime.fromisoformat(last_ts.replace("Z", "+00:00"))
            wall_clock_s = (t1 - t0).total_seconds()
        except (ValueError, TypeError):
            pass

    return {
        "input_tokens": total_input,
        "output_tokens": total_output,
        "cache_read_tokens": total_cache_read,
        "cache_creation_tokens": total_cache_creation,
        "total_tokens": total_input + total_output + total_cache_read + total_cache_creation,
        "tool_calls": tool_calls,
        "turns": turns,
        "wall_clock_s": round(wall_clock_s, 1),
    }


def load_tasks() -> dict:
    """Load task definitions from tasks.toml."""
    # Minimal TOML parser for our simple format
    try:
        import tomllib
    except ImportError:
        try:
            import tomli as tomllib
        except ImportError:
            print("Warning: No TOML parser available. Install tomli: pip install tomli")
            return {}

    with open(TASKS_FILE, "rb") as f:
        return tomllib.load(f)


def delta_str(without: float, with_val: float) -> str:
    """Format a percentage delta."""
    if without == 0:
        return "N/A"
    pct = ((with_val - without) / without) * 100
    sign = "+" if pct > 0 else ""
    return f"{sign}{pct:.1f}%"


def generate_report(runs: list[dict], tasks: dict) -> str:
    """Generate a markdown comparison report."""
    task_lookup = {}
    for task in tasks.get("tasks", []):
        task_lookup[task["id"]] = task

    # Group runs by task_id
    by_task = {}
    for run in runs:
        tid = run["task_id"]
        if tid not in by_task:
            by_task[tid] = {"with": None, "without": None}
        by_task[tid][run["mode"]] = run

    lines = [
        "# RemembrallMCP Benchmark Results",
        "",
        f"Generated: {datetime.now().strftime('%Y-%m-%d %H:%M')}",
        "",
    ]

    aggregate_without = {
        "input_tokens": 0, "output_tokens": 0, "total_tokens": 0,
        "tool_calls": 0, "turns": 0, "wall_clock_s": 0,
    }
    aggregate_with = {
        "input_tokens": 0, "output_tokens": 0, "total_tokens": 0,
        "tool_calls": 0, "turns": 0, "wall_clock_s": 0,
    }
    task_count = 0

    for tid, pair in by_task.items():
        task_def = task_lookup.get(tid, {})
        task_name = task_def.get("name", tid)

        lines.append(f"## {task_name}")
        lines.append("")

        if task_def.get("why"):
            lines.append(f"*{task_def['why']}*")
            lines.append("")

        lines.append(f"**Prompt:** {task_def.get('prompt', 'N/A')}")
        lines.append("")

        if not pair["with"] or not pair["without"]:
            missing = "with" if not pair["with"] else "without"
            lines.append(f"> Missing `{missing}` run. Skipping comparison.")
            lines.append("")
            continue

        m_without = pair["without"].get("metrics", {})
        m_with = pair["with"].get("metrics", {})

        if not m_without or not m_with:
            lines.append("> Metrics not yet collected. Run the analyzer after recording conversation IDs.")
            lines.append("")
            continue

        lines.append("| Metric | Without RemembrallMCP | With RemembrallMCP | Delta |")
        lines.append("|--------|----------------------|---------------------|-------|")

        rows = [
            ("Input tokens", "input_tokens"),
            ("Output tokens", "output_tokens"),
            ("Total tokens", "total_tokens"),
            ("Tool calls", "tool_calls"),
            ("Turns", "turns"),
            ("Wall clock (s)", "wall_clock_s"),
        ]

        for label, key in rows:
            w = m_without.get(key, 0)
            r = m_with.get(key, 0)
            lines.append(f"| {label} | {w:,} | {r:,} | {delta_str(w, r)} |")

        # Accuracy (manual)
        acc_without = pair["without"].get("accuracy")
        acc_with = pair["with"].get("accuracy")
        if acc_without is not None and acc_with is not None:
            lines.append(f"| Accuracy | {acc_without} | {acc_with} | |")

        lines.append("")

        # Accumulate aggregates
        for key in aggregate_without:
            aggregate_without[key] += m_without.get(key, 0)
            aggregate_with[key] += m_with.get(key, 0)
        task_count += 1

    # Aggregate summary
    if task_count > 0:
        lines.append("## Aggregate")
        lines.append("")
        lines.append("| Metric | Without RemembrallMCP | With RemembrallMCP | Delta |")
        lines.append("|--------|----------------------|---------------------|-------|")

        for label, key in [
            ("Total tokens", "total_tokens"),
            ("Total tool calls", "tool_calls"),
            ("Total turns", "turns"),
            ("Total time (s)", "wall_clock_s"),
        ]:
            w = aggregate_without[key]
            r = aggregate_with[key]
            lines.append(f"| {label} | {w:,} | {r:,} | {delta_str(w, r)} |")

        lines.append("")

    return "\n".join(lines)


def main():
    if not RUNS_FILE.exists():
        print(f"No runs file found at {RUNS_FILE}")
        print("Record runs first. See benchmarks/README.md for instructions.")
        sys.exit(1)

    with open(RUNS_FILE) as f:
        data = json.load(f)

    runs = data.get("runs", [])
    if not runs:
        print("No runs recorded yet.")
        print("See benchmarks/README.md for how to record benchmark runs.")
        sys.exit(0)

    # Enrich runs with parsed metrics
    enriched = False
    for run in runs:
        if "metrics" in run:
            continue  # Already parsed

        conv_id = run.get("conversation_id")
        if not conv_id:
            continue

        jsonl = find_conversation_jsonl(conv_id)
        if not jsonl:
            print(f"Warning: Could not find conversation {conv_id}")
            continue

        print(f"Parsing {conv_id} ({run['task_id']}, {run['mode']})...")
        run["metrics"] = parse_conversation(jsonl)
        enriched = True

    # Save enriched data back
    if enriched:
        with open(RUNS_FILE, "w") as f:
            json.dump(data, f, indent=2)
        print()

    # Load tasks for labels
    tasks = load_tasks()

    # Generate report
    report = generate_report(runs, tasks)
    print(report)

    # Write report file
    report_arg = None
    for i, arg in enumerate(sys.argv):
        if arg == "--report" and i + 1 < len(sys.argv):
            report_arg = sys.argv[i + 1]

    if report_arg:
        report_path = Path(report_arg)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        with open(report_path, "w") as f:
            f.write(report)
        print(f"\nReport written to {report_path}")
    else:
        # Auto-save to reports/
        ts = datetime.now().strftime("%Y%m%d-%H%M")
        report_path = BENCHMARKS_DIR / "reports" / f"report-{ts}.md"
        report_path.parent.mkdir(parents=True, exist_ok=True)
        with open(report_path, "w") as f:
            f.write(report)
        print(f"\nReport saved to {report_path}")


if __name__ == "__main__":
    main()
