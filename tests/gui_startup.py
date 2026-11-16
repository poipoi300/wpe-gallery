"""Initial tiled paint regression inside niri nested in private headless sway.

python3 tests/gui_startup.py /private/nested-env.json [release-binary]
Requires Pillow for pixel assertions. Caller starts/stops the private compositors.
No pointer events, real library, renderer or systemd calls are used.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time
from PIL import Image

repo = Path(__file__).resolve().parents[1]
env = json.loads(Path(sys.argv[1]).read_text())
binary = Path(sys.argv[2] if len(sys.argv) > 2 else repo / "target/release/wpe-gallery").resolve()
runtime = Path(env["XDG_RUNTIME_DIR"]).resolve()
assert runtime != Path(f"/run/user/{os.getuid()}"), "Refusing the user's runtime directory"
assert stat.S_IMODE(runtime.stat().st_mode) == 0o700
assert Path(env["NIRI_SOCKET"]).parent.resolve() == runtime
assert (runtime / env["WAYLAND_DISPLAY"]).is_socket()
assert all(o["make"] == "Smithay" for o in json.loads(subprocess.check_output(["niri", "msg", "--json", "outputs"], env=env)).values())
artifact = Path(tempfile.mkdtemp(prefix="wpe-startup-"))
home = artifact / "home"; config = artifact / "config"
root = home / ".local/share/Steam/steamapps/workshop/content/431960"
reviews = {}
for n in range(96):
    id = str(1000+n); item = root / id; item.mkdir(parents=True)
    Image.new("RGB", (32, 32), (34, 85, 136)).save(item / "preview.png")
    (item / "project.json").write_text(json.dumps({"title": f"Wallpaper {n:03}", "preview": "preview.png", "type": "scene"}))
    if n == 0:
        # Sparse file ensures verification outlives the initial configure/frame.
        with (item / "delayed-hash.bin").open("wb") as f:
            f.truncate(4 * 1024**3)
    digest = subprocess.check_output([str(binary), "--content-hash", str(item)], text=True).strip()
    reviews[id] = dict(verdict="safe", reviewed="2026-11-17", reviewer="Synthetic fixture", notes="", content_hash=digest)
(config / "wpe-gallery").mkdir(parents=True)
whitelist = config / "wpe-gallery/whitelist.json"
whitelist.write_text(json.dumps(reviews))
env.update(HOME=str(home), XDG_CONFIG_HOME=str(config), PULSE_SERVER="unix:"+str(runtime / "no-audio"))
env.pop("DISPLAY", None)
apps = []
def launch(name):
    log = artifact / (name+".log")
    app = subprocess.Popen([str(binary)], env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
    apps.append(app)
    data = Path(f"/proc/{app.pid}/environ").read_bytes().split(b"\0")
    for key in ["XDG_RUNTIME_DIR", "WAYLAND_DISPLAY", "NIRI_SOCKET"]:
        assert (key+"="+env[key]).encode() in data
    return app, log
def windows():
    return json.loads(subprocess.check_output(["niri", "msg", "--json", "windows"], env=env))
def wait_for(predicate, description, timeout=30):
    end = time.monotonic()+timeout
    while time.monotonic()<end:
        result = predicate()
        if result: return result
        time.sleep(.05)
    raise AssertionError("Timed out: "+description)
def capture(name):
    path=artifact/name
    subprocess.run(["grim", str(path)], env=env, check=True)
    return Image.open(path).convert("RGB")
def assert_tiles(name, width):
    image=capture(name)
    # First column starts at the left edge. Exclude the sidebar's large preview.
    region=image.crop((0, 160, max(1, int((width-360)*1.25)), image.height))
    count=sum(count for count,rgb in region.getcolors(region.width*region.height) if rgb==(34,85,136))
    assert count>10000, (name, "model completed but thumbnails not painted", count)
def action(*args):
    subprocess.run(["niri", "msg", "action", *args], env=env, check=True, stdout=subprocess.DEVNULL)
try:
    # Configure before launching; do not resize after mapping until paint is proven.
    action("set-column-width", "1331") if windows() else None
    app, log=launch("initial")
    window=wait_for(lambda: next((w for w in windows() if w.get("pid")==app.pid),None),"initial tiled configure")
    width=window["layout"]["window_size"][0]
    assert 760<=width<2000, ("Expected normal narrow tiled column",window)
    assert "Whitelist verification complete" not in log.read_text(), "Slow fixture did not cover the empty first frame"
    capture("initial-verifying.png")
    wait_for(lambda:"Whitelist verification complete" in log.read_text(),"background verification")
    time.sleep(.2)
    assert_tiles("completed-without-resize.png",width)
    # Content bounds change even when the number of columns stays the same.
    action("set-column-width","1336")
    width=wait_for(lambda: next((w["layout"]["window_size"][0] for w in windows() if w.get("pid")==app.pid and w["layout"]["window_size"][0]>width),None),"column resize")
    time.sleep(.2); assert_tiles("resized-column.png",width)
    action("fullscreen-window")
    width=wait_for(lambda: next((w["layout"]["window_size"][0] for w in windows() if w.get("pid")==app.pid and w["layout"]["window_size"][0]>3000),None),"fullscreen")
    time.sleep(.2); assert_tiles("fullscreen.png",width)
    action("fullscreen-window")
    width=wait_for(lambda: next((w["layout"]["window_size"][0] for w in windows() if w.get("pid")==app.pid and w["layout"]["window_size"][0]<2000),None),"return to column")
    time.sleep(.2); assert_tiles("returned-column.png",width)
    app.terminate(); app.wait(timeout=5)
    # Broken whitelist must report the path and parse diagnostic even with zero items.
    whitelist.write_text("{broken")
    app, log=launch("broken-whitelist")
    wait_for(lambda:"Could not load whitelist" in log.read_text(),"parse error")
    assert str(whitelist) in log.read_text()
    assert "line 1 column" in log.read_text()
    time.sleep(.2);capture("whitelist-error.png")
    print("PASS: initial async paint in a normal niri column, resize/fullscreen transitions, whitelist error")
    print("Evidence:",artifact)
finally:
    for app in apps:
        if app.poll() is None:
            app.terminate();app.wait(timeout=5)
