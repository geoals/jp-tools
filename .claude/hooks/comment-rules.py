#!/usr/bin/env python3
import json
import os
import subprocess
import sys
from pathlib import Path

PREFIXES = {
    ".rs": ("//", "/*", "*"),
    ".js": ("//", "/*", "*"),
    ".jsx": ("//", "/*", "*"),
    ".mjs": ("//", "/*", "*"),
    ".ts": ("//", "/*", "*"),
    ".qml": ("//", "/*", "*"),
    ".css": ("/*", "*"),
    ".html": ("<!--",),
    ".sql": ("--", "/*", "*"),
    ".py": ("#",),
    ".sh": ("#",),
    ".toml": ("#",),
    ".yml": ("#",),
    ".yaml": ("#",),
}

CLOSERS = {"/*": "*/", "<!--": "-->"}


def comment_lines(text, prefixes):
    out = set()
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#!"):
            continue
        if s.startswith(prefixes):
            out.add(s)
    return out


def strippable(s, prefixes):
    for p in prefixes:
        if s.startswith(p):
            closer = CLOSERS.get(p)
            return s.endswith(closer) if closer else p != "*"
    return False


def main():
    data = json.load(sys.stdin)
    tool_input = data.get("tool_input") or {}
    path = tool_input.get("file_path") or ""
    if not path or "/.claude/" in path:
        return
    prefixes = PREFIXES.get(Path(path).suffix)
    if not prefixes:
        return

    root = os.environ.get("CLAUDE_PROJECT_DIR") or data.get("cwd") or "."
    allow = Path(root) / ".claude" / "allow-comments"
    if allow.exists():
        allow.unlink()
        return

    new = tool_input.get("new_string") or tool_input.get("content") or ""
    old = tool_input.get("old_string") or ""
    if not new:
        return
    if not old:
        rel = str(Path(path).resolve()).removeprefix(str(Path(root).resolve()) + "/")
        old = subprocess.run(
            ["git", "-C", root, "show", f"HEAD:{rel}"],
            capture_output=True,
            text=True,
        ).stdout

    added = comment_lines(new, prefixes) - comment_lines(old, prefixes)
    if not added:
        return

    text = Path(path).read_text()
    kept, removed = [], []
    for line in text.splitlines(keepends=True):
        s = line.strip()
        if s in added and strippable(s, prefixes):
            removed.append(s)
            continue
        kept.append(line)
    if removed:
        Path(path).write_text("".join(kept))

    left = sorted(added - set(removed))
    report = []
    if removed:
        report.append(
            f"The comment hook deleted {len(removed)} comment line(s) your edit "
            f"added to {path}, so the file on disk no longer has them:"
        )
        report += [f"    {s}" for s in removed]
    if left:
        report.append(
            f"Still in {path} — a multi-line comment block the hook won't cut "
            "apart. Judge it against the bar below and remove it yourself if it "
            "doesn't clear it."
        )
        report += [f"    {s}" for s in left]
    report.append(
        "A comment earns its place only by saying why something non-obvious is "
        "the way it is: a workaround, an ordering that matters, a constraint "
        "coming from somewhere else, an alternative that looks right and isn't. "
        "Naming and smaller functions carry the rest, and they don't rot — "
        "prefer splitting something up or renaming it over describing it. "
        "Nothing that restates the code, the value on the next line, or the "
        "change you just made. Nothing about what the code does not do, used to "
        "do, or might do later, and no reference to something that isn't in the "
        "tree. If a deleted line really does clear that bar, run "
        "`touch .claude/allow-comments` and write it again — that exempts one "
        "edit. Otherwise leave it out; it belongs in the commit message or in "
        "your answer."
    )
    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "\n".join(report),
                }
            }
        )
    )


main()
