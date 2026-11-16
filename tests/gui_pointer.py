"""Pointer regression. Run ONLY against an existing agent-owned headless sway.
Usage: python3 tests/gui_pointer.py /path/to/private/headless-env.json [binary]
Builds a virtual pointer locally; installs nothing. Never starts a renderer/unit.
"""
import json, os, pathlib, shutil, struct, subprocess, sys, tempfile, time, zlib

repo = pathlib.Path(__file__).resolve().parents[1]
env = json.loads(pathlib.Path(sys.argv[1]).read_text())
binary = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else repo / "target/release/wpe-gallery").resolve()
assert env.get("WAYLAND_DISPLAY") and env.get("SWAYSOCK"), "Explicit isolated compositor environment required"
outputs = json.loads(subprocess.check_output(["swaymsg", "-t", "get_outputs"], env=env))
assert outputs and all(o["name"].startswith("HEADLESS-") for o in outputs), "Refusing a non-headless compositor"
out = next(o for o in outputs if o["name"] == "HEADLESS-1")
assert out["current_mode"]["width"] == 5120 and out["scale"] == 1.25
artifact = pathlib.Path(tempfile.mkdtemp(prefix="wpe-pointer-"))
home = artifact / "home"; config = artifact / "config"
root = home / ".local/share/Steam/steamapps/workshop/content/431960"
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\x00\x22\x55\x88")) + chunk(b"IEND", b"")
reviews = {}
for id, title, verdict in [("1001", "A safe fixture", "safe"),
        ("1002", "B safe fixture with a very long title that must stay in its own tile", "safe"),
        ("1003", "C blocked fixture", "blocked"), ("1004", "D unreviewed fixture", None),
        ("1005", "E changed fixture", "safe")]:
    p = root / id; p.mkdir(parents=True)
    (p / "project.json").write_text(json.dumps({"title": title, "type": "scene", "preview": "preview.png", "description": "Synthetic regression fixture", "tags": ["Test"]}))
    (p / "preview.png").write_bytes(png)
    if verdict:
        digest = subprocess.check_output([str(binary), "--content-hash", str(p)], text=True).strip()
        reviews[id] = dict(verdict=verdict, reviewed="2026-11-16", reviewer="Automated synthetic fixture", notes="Not a real wallpaper review", content_hash=digest)
(root / "1005/changed.txt").write_text("Changed after synthetic review")
(config / "wpe-gallery").mkdir(parents=True)
(config / "wpe-gallery/whitelist.json").write_text(json.dumps(reviews))
mock = artifact / "bin"; mock.mkdir()
(mock / "systemctl").write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "' + str(artifact / "systemctl.log") + '"\n')
(mock / "systemctl").chmod(0o755)
env.update(HOME=str(home), XDG_CONFIG_HOME=str(config), PATH=str(mock)+":"+env["PATH"], PULSE_SINK="wpe-tests-no-real-audio")
env.pop("NIRI_SOCKET", None); env.pop("DISPLAY", None)
protocol = repo / "tests/wayland/pointer.xml"
for mode, dest in [("client-header", "pointer.h"), ("private-code", "pointer.c")]:
    subprocess.run(["wayland-scanner", mode, str(protocol), str(artifact / dest)], check=True)
flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True).split()
subprocess.run(["clang", "-I"+str(artifact), str(repo / "tests/wayland/pointer-device.c"), str(artifact / "pointer.c"), "-o", str(artifact / "pointer-device"), *flags], check=True)
pointer = subprocess.Popen([str(artifact / "pointer-device")], env=env, stdout=subprocess.PIPE, text=True)
assert pointer.stdout.readline().strip() == "ready"
app = subprocess.Popen([str(binary)], env=env, stdout=open(artifact / "app.log", "w"), stderr=subprocess.STDOUT)
def ipc(*args):
    subprocess.run(["swaymsg", *args], env=env, check=True, stdout=subprocess.DEVNULL)
def click(x, y, hold=.15):
    ipc("seat", "-", "cursor", "set", str(x), str(y))
    ipc("seat", "-", "cursor", "press", "button1"); time.sleep(hold)
    ipc("seat", "-", "cursor", "release", "button1"); time.sleep(.3)
def capture(name):
    subprocess.run(["grim", "-o", "HEADLESS-1", str(artifact / name)], env=env, check=True)
try:
    time.sleep(2)
    ipc('[app_id="io.github.poipoi300.wpe-gallery"]', "focus")
    capture("default-safe-only.png")
    # Image click and held press across redraws.
    click(400, 230, .4)
    capture("selected-by-pointer.png")
    click(3900, 692)
    assignment = config / "linux-wallpaperengine/assignments.json"
    assert json.loads(assignment.read_text())["outputs"]["HEADLESS-1"]["wallpaper_id"] == "1002"
    assert (artifact / "systemctl.log").read_text().strip() == "--user restart linux-wallpaperengine.service"
    # Title click selects first safe item; Apply button uses its shorter details layout.
    click(100, 280)
    click(3900, 625)
    assert json.loads(assignment.read_text())["outputs"]["HEADLESS-1"]["wallpaper_id"] == "1001"
    # Turn off safe-only and select blocked/unreviewed/changed fixtures. Enter
    # isn't needed: even clicking the disabled Apply area must not restart.
    click(80, 100)
    capture("all-verdicts.png")
    count = len((artifact / "systemctl.log").read_text().splitlines())
    for x in [700, 1000, 1300]:
        click(x, 230)
        for y in [625, 650, 692, 720]:
            click(3900, y)
        assert len((artifact / "systemctl.log").read_text().splitlines()) == count
    print("PASS: fractional-scale image/title selection, Apply, Safe-only and unsafe refusal")
    print("Evidence:", artifact)
finally:
    app.terminate(); pointer.terminate()
    app.wait(timeout=5); pointer.wait(timeout=5)
