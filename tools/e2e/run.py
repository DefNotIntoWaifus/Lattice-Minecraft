#!/usr/bin/env python3
"""Plays every scenario on a private official server with headless Lattice clients.

    cargo build -p lattice && tools/e2e/run.py [text in the scenario name ...]

Needs no window or display. Uses port 25568 and target/e2e/. Exits non-zero if anything
fails, listing what the server or client complained about along the way.
"""

import gzip
import json
import pathlib
import sys
import time
import traceback

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rig  # noqa: E402
import scenarios  # noqa: E402


class T:
    def __init__(self, server, bot):
        self.server = server
        self.bot = bot
        self.notes = []

    def check(self, ok, message):
        if not ok:
            raise rig.Failure(message)

    def wait(self, condition, seconds):
        """Polls the client's state until the condition holds, returning the last state."""
        end = time.time() + seconds
        d = self.bot.dump()
        while time.time() < end and not condition(d):
            time.sleep(0.05)
            d = self.bot.dump()
        return d

    def note(self, message):
        self.notes.append(message)

    def item_names(self):
        data = json.load(gzip.open(rig.ROOT / "crates/lattice-data/data/game.json.gz", "rt"))
        return [i["name"] for i in data["items"] if i["name"] != "air"]


def main():
    wanted = sys.argv[1:]
    rig.WORK.mkdir(parents=True, exist_ok=True)
    print("starting the server", flush=True)
    server = rig.Server()
    bot = rig.Client("Bot")
    failures = []
    try:
        bot.wait_playing()
        server.send("op Bot")
        server.send("gamerule keep_inventory true")
        for f in scenarios.SCENARIOS:
            if wanted and not any(w in f.__name__ for w in wanted):
                continue
            t = T(server, bot)
            since = len(server.lines.all)
            started = time.time()
            try:
                f(t)
                problems = server.problems(since)
                if problems:
                    raise rig.Failure(f"the server complained: {problems[:2]}")
                print(f"ok    {f.__name__} ({time.time() - started:.1f} s)" + "".join(f"\n      {n}" for n in t.notes), flush=True)
            except rig.Failure as e:
                failures.append(f.__name__)
                print(f"FAIL  {f.__name__}: {e}", flush=True)
            except Exception:
                failures.append(f.__name__)
                print(f"ERROR {f.__name__}\n{traceback.format_exc()}", flush=True)
            if bot.p.poll() is not None:
                print(f"the client died:\n{bot.tail(30)}", flush=True)
                failures.append("client exited")
                break
        problems = bot.problems()
        if problems:
            print(f"client log has {len(problems)} problem lines, first:\n  " + "\n  ".join(problems[:8]), flush=True)
            failures.append("client log")
    finally:
        bot.quit()
        server.stop()
    print(f"{'all passed' if not failures else 'failed: ' + ', '.join(failures)}", flush=True)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
