"""Test rig: a private official server plus headless Lattice clients.

Scenarios (scenarios.py) drive both and check results from the server's side wherever
they can, because the server's word is what counts in the game.
"""

import json
import os
import pathlib
import queue
import re
import shutil
import subprocess
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
WORK = ROOT / "target/e2e"
JAVA = str(ROOT / "target/java/bin/java")
PORT = 25568

PROPERTIES = f"""server-port={PORT}
bug-report-link=https://example.net/bugs
online-mode=false
enforce-secure-profile=false
level-type=minecraft:flat
generate-structures=false
spawn-monsters=false
spawn-animals=false
difficulty=easy
view-distance=6
simulation-distance=5
pause-when-empty-seconds=-1
white-list=false
spawn-protection=0
allow-flight=true
sync-chunk-writes=false
max-tick-time=-1
level-seed=lattice
op-permission-level=4
"""


class Failure(Exception):
    pass


class Lines:
    """A process's output, readable line by line with a timeout."""

    def __init__(self, stream, log_path: pathlib.Path):
        self.q: queue.Queue[str] = queue.Queue()
        self.all: list[str] = []
        self.log = open(log_path, "w")
        threading.Thread(target=self._read, args=(stream,), daemon=True).start()

    def _read(self, stream) -> None:
        for line in stream:
            self.log.write(line)
            self.log.flush()
            self.all.append(line.rstrip("\n"))
            self.q.put(line.rstrip("\n"))
        self.q.put("__EOF__")


class Server:
    def __init__(self, fresh: bool = True, flat: bool = True, extra: str = ""):
        d = WORK / "server"
        if fresh and (d / "world").exists():
            shutil.rmtree(d / "world")
        d.mkdir(parents=True, exist_ok=True)
        (d / "eula.txt").write_text("eula=true\n")
        props = PROPERTIES if flat else re.sub(r"level-type=.*\n", "", PROPERTIES)
        (d / "server.properties").write_text(props + extra)
        self.p = subprocess.Popen(
            [JAVA, "-Xmx1500M", "-jar", str(ROOT / "target/test-server/server.jar"), "nogui"],
            cwd=d, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        )
        self.lines = Lines(self.p.stdout, WORK / "server.log")
        self.wait_line("Done (", 240)
        self.send("gamerule spawn_mobs false")
        self.send("gamerule advance_time false")
        self.send("time set day")
        self.send("weather clear")

    def send(self, cmd: str) -> None:
        self.p.stdin.write(cmd + "\n")
        self.p.stdin.flush()

    def wait_line(self, text: str, timeout: float) -> str:
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = self.lines.q.get(timeout=max(0.05, end - time.time()))
            except queue.Empty:
                break
            if line == "__EOF__":
                raise Failure("the server exited")
            if text in line:
                return line
        raise Failure(f"server: no line with '{text}' within {timeout} s")

    def drain(self) -> None:
        while not self.lines.q.empty():
            self.lines.q.get_nowait()

    def ask(self, cmd: str, expect: str = "", timeout: float = 3.0) -> str:
        """Sends a command and returns the line of its answer."""
        self.drain()
        self.send(cmd)
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = self.lines.q.get(timeout=max(0.05, end - time.time()))
            except queue.Empty:
                break
            if "]: " in line and (not expect or expect in line):
                return line.split("]: ", 1)[1].replace("System chat: ", "")
        raise Failure(f"server didn't answer '{cmd}'" + (f" with '{expect}'" if expect else ""))

    def stop(self) -> None:
        try:
            self.send("stop")
            self.p.wait(timeout=30)
        except Exception:
            self.p.kill()

    def problems(self, since: int = 0) -> list[str]:
        """Things the server said that mean a client misbehaved."""
        pat = re.compile(r"moved wrongly|moved too quickly|Invalid move|lost connection: (?!Disconnected)|was kicked|kicked for|Exception|invalid_player|out-of-order|Chat message validation")
        lines = self.lines.all[since:]

        def seconds(line):
            m = re.match(r"\[(\d+):(\d+):(\d+)\]", line)
            return int(m[1]) * 3600 + int(m[2]) * 60 + int(m[3]) if m else None

        stalls = [seconds(l) for l in lines if "Can't keep up" in l]
        # The server only reports stalls over two seconds, and six queued moves are enough. Clients
        # are separate processes, so several tripping in one second points at the server's hitch.
        tripped = {}
        for l in lines:
            m = re.search(r"\]: (\S+) moved too quickly", l)
            if m:
                tripped.setdefault(seconds(l), set()).add(m[1])
        found = []
        for l in lines:
            if not pat.search(l):
                continue
            # When the server stalls, a client's queued moves arrive at once and are all
            # measured from where the tick began, and more than five in a tick drops the
            # allowance to one move. Any client that's moving trips this, so it's no fault.
            # A stall is only reported once it's over, so it can be logged just after.
            at = seconds(l)
            if "moved too quickly" in l and at is not None and any(s is not None and abs(s - at) <= 3 for s in stalls):
                continue
            if "moved too quickly" in l and len(tripped.get(at, ())) >= 2:
                continue
            found.append(l)
        return found


class Client:
    def __init__(self, name: str, width: int = 1280, height: int = 760):
        self.name = name
        home = WORK / f"home-{name}"
        (home / "cache/versions").mkdir(parents=True, exist_ok=True)
        link = home / "cache/versions/26.3"
        if not link.exists():
            target = ROOT / "target/e2e/client-files"
            target.mkdir(parents=True, exist_ok=True)
            jar = target / "client.jar"
            if not jar.exists():
                shutil.copy(ROOT / "target/reference/client.jar", jar)
                import hashlib
                (target / "client.jar.sha1").write_text(hashlib.sha1(jar.read_bytes()).hexdigest())
            link.symlink_to(target)
        env = dict(os.environ, LATTICE_HOME=str(home), LATTICE_HEADLESS=f"{width}x{height}", RUST_LOG="info,wgpu_core=warn,wgpu_hal=warn,naga=warn,lattice_net=warn")
        env.pop("WAYLAND_DISPLAY", None)
        self.p = subprocess.Popen(
            [str(ROOT / os.environ.get("LATTICE_BIN", "target/debug/lattice")), "--session"], cwd=WORK, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        self.out = Lines(self.p.stdout, WORK / f"{name}.out")
        self.err = Lines(self.p.stderr, WORK / f"{name}.log")
        self.next_id = 1
        spec = {"server_name": "Rig", "address": f"localhost:{PORT}", "profile": {"kind": "offline", "name": name}, "reports": False}
        self.raw(json.dumps(spec))

    def raw(self, line: str) -> None:
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def cmd(self, **kw) -> None:
        self.raw(json.dumps(kw))

    def dump(self, timeout: float = 10.0) -> dict:
        n = self.next_id
        self.next_id += 1
        self.cmd(t="dump", id=n)
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = self.out.q.get(timeout=max(0.05, end - time.time()))
            except queue.Empty:
                break
            if line == "__EOF__":
                raise Failure(f"{self.name} exited: {self.tail()}")
            if line.startswith("{"):
                d = json.loads(line)
                if d.get("dump") == n:
                    return d
        raise Failure(f"{self.name} didn't answer a dump; {self.tail()}")

    def tail(self, n: int = 12) -> str:
        return "\n".join(self.err.all[-n:])

    def wait_playing(self, timeout: float = 60.0) -> dict:
        end = time.time() + timeout
        d = {}
        while time.time() < end:
            d = self.dump()
            if d["phase"] == "Playing" and d["pos"] is not None and d["screen"] == "in_game":
                # Answering the server takes a few ticks after arriving.
                time.sleep(0.5)
                return self.dump()
            if d["screen"].startswith("ended"):
                raise Failure(f"{self.name}: {d['screen']}")
            time.sleep(0.2)
        raise Failure(f"{self.name} didn't get into the game: {d.get('screen')} {d.get('phase')}\n{self.tail()}")

    def ticks(self, n: int) -> None:
        time.sleep(n * 0.05)

    def key(self, code: str, down: bool, text: str | None = None) -> None:
        self.cmd(t="key", code=code, down=down, text=text)

    def press(self, code: str, text: str | None = None) -> None:
        self.key(code, True, text)
        self.ticks(2)
        self.key(code, False)

    def button(self, b: int, down: bool) -> None:
        self.cmd(t="button", button=b, down=down)

    def click_mouse(self, b: int) -> None:
        self.button(b, True)
        self.ticks(2)
        self.button(b, False)

    def look(self, yaw: float, pitch: float) -> None:
        self.cmd(t="look", yaw=yaw, pitch=pitch)

    def capture(self, name: str, world_only: bool = False) -> pathlib.Path:
        path = WORK / "shots" / f"{name}.png"
        self.cmd(t="capture", path=str(path), world_only=world_only)
        self.ticks(4)
        return path

    def click(self, label: str, button: int = 0, shift: bool = False) -> None:
        """Clicks a labelled control, as a person would."""
        self.ticks(2)
        w = self.widget(label)
        self.cmd(t="click", x=(w[0] + w[2]) / 2, y=(w[1] + w[3]) / 2, button=button, shift=shift)
        self.ticks(2)

    def widget(self, label: str) -> list[float]:
        d = self.dump()
        # The list holds what was drawn since the last dump, so ask for a frame's worth more.
        time.sleep(0.05)
        d = self.dump()
        for w in d["widgets"]:
            if w["label"] == label:
                return w["rect"]
        raise Failure(f"no control labelled '{label}' on screen; there are {[w['label'] for w in d['widgets']][:60]}")

    def has_widget(self, label: str) -> bool:
        self.dump()
        time.sleep(0.05)
        return any(w["label"] == label for w in self.dump()["widgets"])

    def quit(self) -> None:
        try:
            self.cmd(t="quit")
            self.p.wait(timeout=8)
        except Exception:
            self.p.kill()

    def problems(self) -> list[str]:
        pat = re.compile(r"left over after decoding|undecodable|PANIC|panicked|failed to decode|ERROR| WARN .*(unknown|ignored|couldn)")
        return [l for l in self.err.all if pat.search(l)]


def near(a: float, b: float, tolerance: float) -> bool:
    return abs(a - b) <= tolerance
