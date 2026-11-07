#!/usr/bin/env python3
"""Frame rate with nothing, then with crowds, on a generated world.

    cargo build --release -p lattice && LATTICE_BIN=target/release/lattice python3 -I tools/e2e/perf.py

Headless, as fast as it'll go, so it measures the work and not the display.
"""

import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import rig  # noqa: E402


def measure(bot, seconds=6.0):
    bot.ticks(10)
    samples = []
    end = time.time() + seconds
    while time.time() < end:
        d = bot.dump(timeout=20)
        samples.append((d["fps"], d.get("frame_ms", 0)))
        time.sleep(0.25)
    fps = sorted(s[0] for s in samples)
    return fps[len(fps) // 2], fps[0], d


def main():
    server = rig.Server(fresh=True, flat=False)
    bot = None
    try:
        bot = rig.Client("Bot")
        bot.wait_playing()
        server.send("op Bot")
        server.send("gamerule spawn_mobs false")
        server.send("gamemode creative Bot")
        bot.cmd(t="fps", fps=0)
        d = bot.dump()
        x, y, z = [int(v) for v in d["pos"]]
        server.send(f"tp Bot {x} {y + 20} {z} 0 25")
        bot.look(0, 25)
        time.sleep(8)
        median, low, d = measure(bot)
        print(f"terrain only: {median:.0f} fps median, {low:.0f} lowest; sections drawn: {d.get('sections_drawn', '?')}", flush=True)
        kinds = ["zombie", "cow", "sheep", "villager", "skeleton", "horse"]
        placed = 0
        for target in [50, 200, 500]:
            while placed < target:
                kind = kinds[placed % len(kinds)]
                dx, dz = (placed % 25) * 2 - 24, 8 + (placed // 25) * 2
                server.send(f"summon {kind} {x + dx} {y + 6} {z + dz} " + "{NoAI:1b,NoGravity:1b,PersistenceRequired:1b}")
                placed += 1
            time.sleep(1.5)
            median, low, d = measure(bot)
            print(f"{target} mobs in view: {median:.0f} fps median, {low:.0f} lowest", flush=True)
    finally:
        # Whatever went wrong, don't leave a server holding the test port.
        if bot:
            bot.quit()
        server.stop()


if __name__ == "__main__":
    main()
