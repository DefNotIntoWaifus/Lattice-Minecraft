#!/usr/bin/env python3
"""A world on this computer, from nothing: downloads, starts, plays, saves, starts again.

    cargo build -p lattice && python3 -I tools/e2e/local_world.py

Runs in its own folder under target/e2e, so nothing of anyone's real worlds is touched.
"""

import json
import os
import pathlib
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import rig  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
HOME = ROOT / "target/e2e/home-World"


def start(kind="flat"):
    env = dict(os.environ, LATTICE_HOME=str(HOME), RUST_LOG="info")
    p = subprocess.Popen(
        [str(ROOT / "target/debug/lattice"), "--host-world", "Test place", "--accept-minecraft-license", "--world-type", kind, "--name", "Bot"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env,
    )
    address = {}
    def read():
        for line in p.stdout:
            line = line.strip()
            if line.startswith("step"):
                if "%" not in line:
                    print(line, flush=True)
            if line.startswith("ready"):
                address["at"] = line.split()[1]
    threading.Thread(target=read, daemon=True).start()
    started = time.time()
    while "at" not in address:
        if p.poll() is not None:
            raise SystemExit(f"the host stopped by itself with {p.returncode}")
        if time.time() - started > 600:
            p.kill()
            raise SystemExit("never got ready")
        time.sleep(0.1)
    print(f"ready at {address['at']} after {time.time() - started:.1f} s", flush=True)
    return p, int(address["at"].split(":")[1])


def stop(p):
    started = time.time()
    p.stdin.close()
    p.wait(timeout=60)
    print(f"closed in {time.time() - started:.1f} s, exit code {p.returncode}", flush=True)
    assert p.returncode == 0


def main():
    if HOME.exists():
        for child in (HOME / "data/worlds").glob("*"):
            shutil.rmtree(child)
    host, port = start()
    rig.PORT = port
    bot = rig.Client("Bot")
    try:
        d = bot.wait_playing()
        print("joined at", d["pos"], flush=True)
        bot.ticks(20)
        bot.press("Digit7", "/")
        bot.ticks(2)
        bot.cmd(t="type", text="give Bot minecraft:diamond 5")
        bot.ticks(3)
        bot.cmd(t="ui_key", key="Enter")
        bot.ticks(10)
        d = bot.dump()
        assert any(s["item"] == "diamond" and s["count"] == 5 for s in d["slots"]), f"the owner can run commands: {d['slots']} {d['chat'][-3:]}"
        print("owner can use commands", flush=True)
        bot.cmd(t="chat", line="/setblock ~ ~3 ~ gold_block")
        bot.ticks(10)
        bot.capture("local-world")
    finally:
        bot.quit()
    stop(host)
    folder = next((HOME / "data/worlds").glob("*"))
    info = json.loads((folder / "lattice-world.json").read_text())
    assert info["last_played"], info
    assert (folder / "world").exists(), "the world's files are there"
    props = (folder / "server.properties").read_text()
    assert "server-ip=127.0.0.1" in props
    assert "minecraft\\:flat" in props
    # It's still there when it's opened again, on the same port.
    host, port2 = start()
    assert port2 == port, f"kept its port: {port} and {port2}"
    rig.PORT = port2
    bot = rig.Client("Bot")
    try:
        bot.wait_playing()
        bot.ticks(5)
        d = bot.dump()
        assert any(s["item"] == "diamond" for s in d["slots"]), "what the player had is still there"
        print("the inventory survived closing the world", flush=True)
    finally:
        bot.quit()
    stop(host)
    print("all passed", flush=True)


if __name__ == "__main__":
    main()
