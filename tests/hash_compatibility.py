"""Cross-check Rust against the independently implemented triage hash."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import os

repo = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("triage", repo / "tools/triage/scan.py")
triage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(triage)
binary = os.environ.get("WPE_GALLERY_BINARY", str(repo / "target/release/wpe-gallery"))
with tempfile.TemporaryDirectory() as temp:
    root = Path(temp)
    (root / "empty").mkdir()
    (root / "nested").mkdir()
    (root / "nested/é.txt").write_bytes(b"UTF-8 path")
    (root / ".hidden").write_bytes(bytes(range(256)))
    (root / "scene.pkg").write_bytes(b"original archive bytes")
    (root / "project.json").write_text(json.dumps({"type":"scene","file":"scene.pkg"}))
    expected = triage.scan(root)["content_hash"]
    actual = subprocess.check_output([binary, "--content-hash", str(root)], text=True).strip()
    assert actual == expected, (actual, expected)
    print("PASS: Rust and triage agree on files, hidden/Unicode names and empty dirs:", actual)
