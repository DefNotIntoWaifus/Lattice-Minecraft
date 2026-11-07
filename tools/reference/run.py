#!/usr/bin/env python3
"""Runs the official client against a private test server and records reference data.

Scenes are defined in scenes.py. Each one builds a small world with server commands,
then plays scripted input through the client while recording the player's state every
tick. The result lands in crates/lattice-world/tests/golden/NAME.json, which
`cargo test -p lattice-world` replays against Lattice's physics.

    tools/reference/fetch.py            once, to download the client
    tools/reference/run.py              record every scene
    tools/reference/run.py water_sink   record scenes whose name contains this text
    tools/reference/run.py --creative   write the creative inventory to crates/lattice-data/data/
    tools/reference/run.py --shell      leave both running and read commands from stdin

Uses port 25567 and target/reference/, so it can't touch anything else.
"""

import json
import os
import pathlib
import queue
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import scenes as scene_defs  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
REF = ROOT / "target/reference"
JAVA = str(ROOT / "target/java/bin/java")
GOLDEN = ROOT / "crates/lattice-world/tests/golden"
PORT = 25567
PLAYER = "Ref"

PROPERTIES = f"""server-port={PORT}
online-mode=false
enforce-secure-profile=false
level-type=minecraft:flat
generator-settings={{"layers":[{{"block":"minecraft:air","height":1}}],"biome":"minecraft:plains"}}
generate-structures=false
spawn-monsters=false
difficulty=peaceful
view-distance=6
simulation-distance=5
pause-when-empty-seconds=-1
white-list=false
spawn-protection=0
allow-flight=true
sync-chunk-writes=false
max-tick-time=-1
"""

OPTIONS = """pauseOnLostFocus:false
enableVsync:false
maxFps:260
renderDistance:6
simulationDistance:5
onboardAccessibility:false
narrator:0
tutorialStep:none
joinedFirstServer:true
skipMultiplayerWarnings:true
soundCategory_master:0.0
guiScale:2
"""


class Proc:
    def __init__(self, name: str, cmd: list[str], cwd: pathlib.Path, env: dict | None = None):
        self.name = name
        self.lines: queue.Queue[str] = queue.Queue()
        self.log = open(REF / f"{name}.log", "w")
        self.p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        for line in self.p.stdout:
            self.log.write(line)
            self.log.flush()
            self.lines.put(line.rstrip("\n"))
        self.lines.put("__EOF__")

    def send(self, s: str) -> None:
        self.p.stdin.write(s + "\n")
        self.p.stdin.flush()

    def wait_for(self, text: str, timeout: float) -> str:
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = self.lines.get(timeout=max(0.05, end - time.time()))
            except queue.Empty:
                break
            if line == "__EOF__":
                raise RuntimeError(f"{self.name} exited; see {self.log.name}")
            if text in line:
                return line
        raise TimeoutError(f"{self.name}: no '{text}' within {timeout} s; see {self.log.name}")

    def drain(self) -> None:
        while not self.lines.empty():
            self.lines.get_nowait()


def start_server() -> Proc:
    d = REF / "server"
    d.mkdir(parents=True, exist_ok=True)
    (d / "eula.txt").write_text("eula=true\n")
    (d / "server.properties").write_text(PROPERTIES)
    jar = ROOT / "target/test-server/server.jar"
    s = Proc("server", [JAVA, "-Xmx1500M", "-jar", str(jar), "nogui"], d)
    s.wait_for("Done (", 240)
    return s


def start_client() -> Proc:
    game = REF / "game"
    game.mkdir(parents=True, exist_ok=True)
    (game / "options.txt").write_text(OPTIONS)
    natives = REF / "natives"
    for sub in ("java", "jna", "lwjgl", "netty"):
        (natives / sub).mkdir(parents=True, exist_ok=True)
    version = json.loads((REF / "version.json").read_text())
    cp = [str(REF / "harness"), str(REF / "client.jar")] + (REF / "classpath.txt").read_text().splitlines()
    cmd = [
        JAVA,
        "-Xmx2G",
        "-XX:StackShadowPages=32",
        "--enable-native-access=ALL-UNNAMED",
        "--add-exports",
        "java.base/jdk.internal.misc=ALL-UNNAMED",
        f"-Djava.library.path={natives}/java",
        f"-Djna.tmpdir={natives}/jna",
        f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}/lwjgl",
        f"-Dio.netty.native.workdir={natives}/netty",
        "-Dminecraft.launcher.brand=lattice-reference",
        "-Dminecraft.launcher.version=1",
        "-cp",
        ":".join(cp),
        "RefHarness",
        "--username", PLAYER,
        "--version", version["id"],
        "--gameDir", str(game),
        "--assetsDir", str(REF / "assets"),
        "--assetIndex", version["assetIndex"]["id"],
        "--uuid", "00000000000000000000000000000001",
        "--accessToken", "0",
        "--versionType", "release",
        "--quickPlayPath", str(game / "quickplay.json"),
        "--quickPlayMultiplayer", f"localhost:{PORT}",
        "--width", "854",
        "--height", "480",
    ]
    return Proc("client", cmd, game, env=dict(os.environ))


def wait_joined(server: Proc, client: Proc, timeout: float = 240) -> None:
    end = time.time() + timeout
    while time.time() < end:
        try:
            server.wait_for(f"{PLAYER} joined the game", 2)
            return
        except TimeoutError:
            if client.p.poll() is not None:
                raise RuntimeError(f"the client exited with {client.p.returncode}; see {client.log.name}")
    raise TimeoutError("the client never joined")


def record(server: Proc, client: Proc, scene: scene_defs.Scene) -> None:
    # A frame long enough to hold two ticks loses a row; those recordings are thrown away.
    for _ in range(4):
        if record_once(server, client, scene):
            return
        print(f"{scene.name}: a tick was missed, recording again", flush=True)
    raise RuntimeError(f"{scene.name}: couldn't record without missing ticks")


def record_once(server: Proc, client: Proc, scene: scene_defs.Scene) -> bool:
    x1, y1, z1, x2, y2, z2 = scene_defs.AREA
    server.send("tick freeze")
    server.send(f"fill {x1} {y1} {z1} {x2} {y2} {z2} minecraft:air")
    for c in scene.build:
        server.send(c)
    for c in scene.commands:
        server.send(c)
    sx, sy, sz = scene.start
    server.send(f"tp {PLAYER} {sx} {sy} {sz} {scene.yaw} {scene.pitch}")
    time.sleep(scene.settle)
    dump = REF / "out" / f"{scene.name}.blocks"
    client.drain()
    # Relative to the client's folder: the commands are split on spaces, and the project path has some.
    client.send(f"dump ../out/{scene.name}.blocks {x1} {y1} {z1} {x2} {y2} {z2}")
    client.wait_for("REF dumped", 30)
    trace = REF / "out" / f"{scene.name}.jsonl"
    segs = " ".join(f"{t}:{','.join(k) if k else '-'}:{'~' if yaw is None else yaw}:{'~' if pitch is None else pitch}" for t, k, yaw, pitch in scene.segments)
    client.send(f"run {scene.name} ../out/{scene.name}.jsonl {segs}")
    done = client.wait_for(f"REF done {scene.name}", sum(s[0] for s in scene.segments) / 20 + 60)
    print(done, flush=True)
    if not done.rstrip().endswith("missed=0"):
        return False
    blocks = [[int(v) for v in line.split()] for line in dump.read_text().splitlines()]
    rows = [json.loads(line) for line in trace.read_text().splitlines()]
    GOLDEN.mkdir(parents=True, exist_ok=True)
    (GOLDEN / f"{scene.name}.json").write_text(
        json.dumps(
            {
                "name": scene.name,
                "about": scene.about,
                "attrs": scene.attrs,
                "effects": scene.effects,
                "mode": scene.mode,
                "worn": scene.worn,
                "blocks": blocks,
                "segments": [{"ticks": t, "keys": list(k), "yaw": yaw, "pitch": pitch} for t, k, yaw, pitch in scene.segments],
                "trace": rows,
            },
            separators=(",", ":"),
        )
    )
    for c in scene.cleanup:
        server.send(c)
    return True


def hands(server: Proc, client: Proc, ground: int) -> None:
    """The player's own view with a bare hand and holding things."""
    server.send(f"gamemode survival {PLAYER}")
    server.send(f"tp {PLAYER} 0.5 {ground} 0.5 0 10")
    time.sleep(1.0)
    client.send("look 0 10")
    time.sleep(2.0)
    client.send("shot . zoo-hand-empty.png")
    client.wait_for("shot zoo-hand-empty.png", 20)
    for slot, (item, name) in enumerate([("diamond_sword", "sword"), ("stone", "block"), ("bow", "bow"), ("shield", "shield"), ("apple", "apple")]):
        server.send(f"item replace entity {PLAYER} hotbar.0 with {item}")
        time.sleep(1.5)
        client.send(f"shot . zoo-hand-{name}.png")
        client.wait_for(f"shot zoo-hand-{name}.png", 20)
        print("ref hand", name, flush=True)
    server.send(f"gamemode spectator {PLAYER}")


def zoo(server: Proc, client: Proc, wanted: list[str]) -> None:
    """Pictures of the same rows of things the end-to-end zoo shows, from the official client."""
    sys.path.insert(0, str(ROOT / "tools/e2e"))
    import zoo as zoo_defs

    ground = zoo_defs.GROUND
    server.send(f"gamemode spectator {PLAYER}")
    server.send(f"tp {PLAYER} 0.5 {ground + 5} 0.5 180 18")
    time.sleep(2.0)
    server.send(f"fill -30 {ground - 1} -30 30 {ground - 1} 30 grass_block")
    server.send("time set noon")
    server.send("weather clear")
    server.send(f"effect give {PLAYER} minecraft:night_vision infinite 0 true")
    if not wanted or "hand" in wanted:
        hands(server, client, ground)
    if not wanted or "sky" in wanted:
        server.send(f"effect clear {PLAYER}")
        for name, t, yaw, pitch in [("noon", 6000, 180, -40), ("sunset", 12300, 90, -8), ("night", 18000, 180, -50), ("morning", 23500, 270, -8)]:
            server.send(f"time set {t}")
            client.send(f"look {yaw} {pitch}")
            time.sleep(2.0)
            client.send(f"shot . zoo-sky-{name}.png")
            client.wait_for(f"shot zoo-sky-{name}.png", 20)
        server.send("time set 6000")
        server.send("weather rain")
        client.send("look 180 10")
        time.sleep(8.0)
        client.send("shot . zoo-sky-rain.png")
        client.wait_for("shot zoo-sky-rain.png", 20)
        server.send("weather clear")
    for name, entities in zoo_defs.GROUPS.items():
        if wanted and not any(w in name for w in wanted):
            continue
        server.send("kill @e[type=!player]")
        server.send("kill @e[type=item]")
        time.sleep(0.3)
        half = len(entities) // 2
        zoo_defs.row(server, entities[:half], -9)
        zoo_defs.row(server, entities[half:], -16, spacing=5.0)
        server.send(f"tp {PLAYER} 0.5 {ground + 5} 0.5 180 18")
        client.send("look 180 18")
        time.sleep(2.5)
        client.send(f"shot . zoo-{name}.png")
        client.wait_for(f"shot zoo-{name}.png", 20)
        print("ref", name, flush=True)


def main() -> None:
    args = sys.argv[1:]
    if not (REF / "harness/RefHarness.class").exists():
        sys.exit("compile tools/reference/RefHarness.java into target/reference/harness first")
    server = start_server()
    print("server up", flush=True)
    client = start_client()
    try:
        server.send(f"forceload add -32 -32 31 31")
        wait_joined(server, client)
        print("client joined", flush=True)
        server.send(f"op {PLAYER}")
        server.send("gamerule spawn_mobs false")
        server.send(f"effect give {PLAYER} minecraft:resistance infinite 255 true")
        server.send(f"effect give {PLAYER} minecraft:saturation infinite 255 true")
        # Damage makes the server overwrite the client's velocity, which isn't movement being tested.
        server.send(f"effect give {PLAYER} minecraft:fire_resistance infinite 0 true")
        time.sleep(3)
        if "--models" in args:
            client.send("models ../out/models-official.json")
            client.wait_for("REF models done", 60)
            print("wrote target/reference/out/models-official.json", flush=True)
            return
        if "--creative" in args:
            client.send("creative ../out/creative.json")
            client.wait_for("REF creative done", 60)
            data = ROOT / "crates/lattice-data/data/creative.json"
            shutil.copy(REF / "out/creative.json", data)
            print(f"wrote {data}", flush=True)
            return
        if "--zoo" in args:
            zoo(server, client, [a for a in args if not a.startswith("-")])
            return
        if "--shell" in args:
            for line in sys.stdin:
                who, _, rest = line.strip().partition(" ")
                (server if who == "s" else client).send(rest)
            return
        wanted = [a for a in args if not a.startswith("-")]
        for scene in scene_defs.all_scenes():
            if not wanted or any(w in scene.name for w in wanted):
                record(server, client, scene)
    finally:
        try:
            client.send("quit")
            client.p.wait(timeout=15)
        except Exception:
            client.p.kill()
        try:
            server.send("stop")
            server.p.wait(timeout=30)
        except Exception:
            server.p.kill()


if __name__ == "__main__":
    main()
