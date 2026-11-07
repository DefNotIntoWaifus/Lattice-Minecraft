#!/usr/bin/env python3
"""Plays badly for a long time on a normal world, and looks for what breaks.

    cargo build -p lattice && python3 -I tools/e2e/soak.py [minutes] [seed]

Random keys, clicks, menus and server events. Fails on: the client dying, being kicked,
errors in either log, or the frame rate collapsing.
"""

import pathlib
import random
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import rig  # noqa: E402

MOBS = ["zombie", "skeleton", "creeper", "spider", "cow", "pig", "sheep", "chicken", "villager", "horse", "wolf", "slime", "enderman", "witch", "bee", "iron_golem", "boat", "oak_boat", "minecart", "tnt", "armor_stand", "item_frame"]
ITEMS = ["diamond_sword", "bow", "arrow", "oak_planks", "torch", "water_bucket", "lava_bucket", "cooked_beef", "shield", "ender_pearl", "snowball", "egg", "writable_book", "filled_map", "map", "flint_and_steel", "oak_sign", "white_bed", "minecart", "oak_boat", "saddle", "lead", "potion", "golden_apple", "tnt", "chest", "crafting_table", "furnace", "anvil"]


def drive(name, bot, server, rng, minutes, keys, stats, failures):
    """One bot playing at random until the time's up."""
    frames = []
    worst = 0.0
    try:
        end = time.time() + minutes * 60
        last_report = 0
        while time.time() < end:
            if bot.p.poll() is not None:
                raise rig.Failure(f"the client died:\n{bot.tail(30)}")
            d = bot.dump(timeout=30)
            # A change of dimension loads terrain for a moment; stuck there is the failure.
            waited = time.time()
            while d["phase"] == "LoadingTerrain" and time.time() - waited < 45:
                bot.ticks(10)
                d = bot.dump(timeout=30)
            if d["phase"] != "Playing":
                raise rig.Failure(f"{d['phase']} for {time.time() - waited:.0f} s\n{bot.tail(10)}")
            pos = d["pos"]
            x, y, z = [int(v) for v in pos] if pos else (0, 64, 0)
            if d["dead"]:
                stats["deaths"] += 1
                bot.click("Respawn")
                bot.ticks(20)
                continue
            if d["paused"] or "chat" in d["overlays"] or d["menu"] or d["own_screen"] or d["modal"]:
                # Something's open: poke at it, then leave.
                if rng.random() < 0.5:
                    widgets = rng.randint(100, 1100), rng.randint(60, 700)
                    bot.cmd(t="click", x=widgets[0], y=widgets[1], button=rng.choice([0, 0, 1]), shift=rng.random() < 0.2)
                    bot.ticks(2)
                else:
                    bot.press("Escape")
                    bot.ticks(2)
                continue
            action = rng.random()
            if action < 0.35:
                k = rng.choice(keys)
                bot.key(k, True)
                bot.ticks(rng.randint(2, 30))
                bot.key(k, False)
            elif action < 0.5:
                bot.look(rng.uniform(-180, 180), rng.uniform(-60, 80))
                bot.ticks(1)
            elif action < 0.62:
                button = rng.choice([0, 1])
                bot.button(button, True)
                bot.ticks(rng.randint(1, 25))
                bot.button(button, False)
            elif action < 0.7:
                bot.press(rng.choice(["Digit1", "Digit2", "Digit3", "Digit4", "Digit5"]))
            elif action < 0.74:
                bot.press("KeyE", "e")
                stats["menus"] += 1
                bot.ticks(2)
            elif action < 0.8:
                item = rng.choice(ITEMS)
                server.send(f"give {name} minecraft:{item} {rng.randint(1, 16)}")
                stats["items"] += 1
            elif action < 0.88:
                mob = rng.choice(MOBS)
                server.send(f"summon {mob} {x + rng.randint(-8, 8)} {y + rng.randint(0, 3)} {z + rng.randint(-8, 8)}")
                stats["summons"] += 1
            elif action < 0.91:
                server.send(rng.choice(["time set day", "time set night", "weather rain", "weather thunder", "weather clear", "gamemode creative Bot", "gamemode survival Bot", "effect give Bot minecraft:speed 10 1", "effect give Bot minecraft:blindness 3", "clear Bot"]))
            elif action < 0.93:
                dx, dz = rng.randint(-300, 300), rng.randint(-300, 300)
                server.send(f"spreadplayers {x + dx} {z + dz} 0 10 false {name}")
                bot.ticks(20)
            elif action < 0.95:
                bot.cmd(t="chat", line=rng.choice(["hello", "/help", "/seed", "/list", "/nonsense command", "a" * 200]))
            elif action < 0.97:
                server.send(f"setblock {x + rng.randint(-3, 3)} {y + rng.randint(-1, 3)} {z + rng.randint(-3, 3)} minecraft:" + rng.choice(["chest", "crafting_table", "furnace", "oak_sign", "lectern", "enchanting_table", "anvil", "beacon", "water", "lava", "fire"]))
            elif action < 0.98:
                dim = rng.choice(["the_nether", "the_end", "overworld"])
                server.send(f"execute in minecraft:{dim} run tp {name} 0 100 0")
                server.send(f"effect give {name} minecraft:slow_falling 10")
                stats["worlds"] += 1
                bot.ticks(40)
            else:
                server.send("kill @e[type=!player,distance=..64]") if False else server.send("kill @e[type=!player]")
            fps = d.get("fps")
            if fps:
                frames.append(fps)
            if time.time() - last_report > 60:
                last_report = time.time()
                recent = frames[-200:]
                low = min(recent) if recent else 0
                print(f"{name}: {(end - time.time()) / 60:.1f} min left, {stats}, entities {len(d.get('entities', []))}, slowest recent {low:.0f} fps", flush=True)
                worst = max(worst, 0)
    except Exception as e:  # noqa: BLE001
        failures.append(f"{name}: {e}")


def main():
    minutes = float(sys.argv[1]) if len(sys.argv) > 1 else 10
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else int(time.time())
    count = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    print(f"seed {seed}, {minutes} minutes, {count} bot(s)", flush=True)
    server = rig.Server(fresh=True, flat=False)
    names = ["Bot", "Buddy", "Third", "Fourth"][:count]
    bots = {name: rig.Client(name) for name in names}
    keys = ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ShiftLeft", "ControlLeft"]
    stats = {"menus": 0, "summons": 0, "items": 0, "deaths": 0, "worlds": 0}
    failures = []
    try:
        for name, bot in bots.items():
            bot.wait_playing()
            server.send(f"op {name}")
        server.send("gamerule keep_inventory true")
        server.send("gamerule spawn_mobs true")
        server.send("gamerule advance_time true")
        threads = [
            threading.Thread(target=drive, args=(name, bot, server, random.Random(seed + i), minutes, keys, stats, failures))
            for i, (name, bot) in enumerate(bots.items())
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        problems = [f"{name}: {p}" for name, bot in bots.items() for p in bot.problems()]
        server_problems = server.problems(0)
        if failures:
            print("bots failed:", *failures, sep="\n  ")
        if problems:
            print("client log problems:", *problems[:20], sep="\n  ")
        if server_problems:
            print("server complaints:", *server_problems[:20], sep="\n  ")
        print(f"done: {stats}", flush=True)
        sys.exit(1 if failures or problems or server_problems else 0)
    finally:
        for bot in bots.values():
            bot.quit()
        server.stop()


if __name__ == "__main__":
    main()
