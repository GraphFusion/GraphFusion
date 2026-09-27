"""Validate documentation snippets against the actual CLI and GQL parser."""
from pathlib import Path
import os
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "website/src/content/docs"
FIXTURE = """CREATE GRAPH social ANY GRAPH;
SESSION SET GRAPH social;
INSERT (a:Person {name:'Alice', age:30})-[:Knows]->
       (b:Person {name:'Bob', age:40})-[:Knows]->
       (c:Person {name:'Cara', age:25});
"""
subprocess.run(["cargo", "build", "-p", "graphfusion", "--bin", "graphfusion", "--locked"], cwd=ROOT, check=True)
subprocess.run(["cargo", "build", "-p", "gql-parser", "--example", "parse_file", "--locked"], cwd=ROOT, check=True)
# Respect custom Cargo target directories used by CI/contributors.
import json
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=ROOT))
target = Path(metadata["target_directory"]) / "debug"
suffix = ".exe" if os.name == "nt" else ""
cli = target / ("graphfusion" + suffix)
parser = target / "examples" / ("parse_file" + suffix)
failures = []
executed = parsed = 0
with tempfile.TemporaryDirectory(prefix="graphfusion-docs-") as temp:
    for doc in sorted(DOCS.rglob("*")):
        if doc.suffix not in (".md", ".mdx"):
            continue
        for index, match in enumerate(re.finditer(r"^```gql([^\n]*)\n(.*?)^```", doc.read_text(), re.M | re.S), 1):
            mode, code = match.groups()
            label = f"{doc.relative_to(DOCS)} example {index}"
            source = Path(temp) / "example.gql"
            if mode.strip() == "parse":
                source.write_text(code)
                command = [str(parser), str(source)]
                parsed += 1
            elif mode.strip() in ("test", "test=standalone"):
                source.write_text(("" if "standalone" in mode else FIXTURE) + code)
                command = [str(cli), "run", "--file", str(source)]
                executed += 1
            else:
                failures.append(f"{label}: missing test/parse classification")
                continue
            result = subprocess.run(command, capture_output=True, text=True, timeout=90)
            if result.returncode:
                failures.append(f"{label}: {result.stderr.strip()}")
    # Also execute the full user-facing programs and verify selected observable results.
    expected = {"social": "Cara", "analytics": "31", "paths": "destination", "path-patterns": "optional_stop", "element-references": "AB", "transactions": "150"}
    for name, text in expected.items():
        result = subprocess.run([str(cli), "run", "--file", str(ROOT / f"examples/{name}.gql")], capture_output=True, text=True, timeout=90)
        executed += 1
        if result.returncode or text not in result.stdout:
            failures.append(f"examples/{name}.gql: expected {text!r}; {result.stderr or result.stdout}")
    # The quickstart must survive reopening a persistent database in another process.
    quickstart = (DOCS / "start/quickstart.md").read_text()
    program = re.search(r"```gql test=standalone\n(.*?)```", quickstart, re.S).group(1)
    database = str(Path(temp) / "database")
    result = subprocess.run([str(cli), "run", "--database", database, "--create", "--query", program], capture_output=True, text=True, timeout=90)
    reopened = subprocess.run([str(cli), "run", "--database", database, "--query", "USE GRAPH social MATCH (p:Person) RETURN p.name AS name ORDER BY name"], capture_output=True, text=True, timeout=90)
    if result.returncode or reopened.returncode or not all(name in reopened.stdout for name in ("Alice", "Bob", "Cara")):
        failures.append(f"persistent quickstart: {result.stderr}; {reopened.stderr}; {reopened.stdout}")
if failures:
    raise SystemExit("\n\n".join(failures))
print(f"Verified {executed} executable examples, {parsed} syntax examples, and persistent quickstart reopen.")
