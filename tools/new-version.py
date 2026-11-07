#!/usr/bin/env python3
"""Moves Lattice to a new Minecraft version, and says what changed.

    tools/new-version.py 26.4            do it: new game facts, version numbers, a report
    tools/new-version.py 26.4 --dry-run  only the report, with nothing in the source touched
    tools/new-version.py --latest        what Mojang's newest release is, and whether we're on it

Lattice speaks one version of the protocol, so a new Minecraft version means new facts from
Mojang's own server jar, then reading what changed. The tool does the first part and lists the
second: which packets, blocks, items and registries differ, so nothing gets missed. See
docs/RELEASING.md for the rest. Nothing of Mojang's goes in the repository beyond those facts.
"""

import gzip
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "crates/lattice-data/data"
MANIFEST = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"

# Each place that names the version, and the pattern around it.
VERSION_PLACES = [
    ("crates/lattice-proto/src/packets/handshake.rs", r'(pub const GAME_VERSION: &str = ")([^"]+)(")'),
    ("crates/lattice-assets/src/lib.rs", r'(pub const GAME_VERSION: &str = ")([^"]+)(")'),
    ("tools/gen-data.sh", r'(VERSION=")([^"]+)(")'),
    ("tools/test-server.sh", r'(VERSION=")([^"]+)(")'),
    ("tools/reference/fetch.py", r'(VERSION = ")([^"]+)(")'),
    ("tools/e2e/rig.py", r'(cache/versions/)([0-9][^"]*)(")'),
]
PROTOCOL_PLACE = ("crates/lattice-proto/src/packets/handshake.rs", r"(pub const PROTOCOL_VERSION: i32 = )(\d+)(;)")


def fetch_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": "lattice-new-version"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def current_version():
    text = (ROOT / VERSION_PLACES[0][0]).read_text()
    return re.search(VERSION_PLACES[0][1], text).group(2)


def load_facts(directory):
    with gzip.open(directory / "game.json.gz", "rt") as f:
        game = json.load(f)
    return game, json.loads((directory / "packets.json").read_text())


def compare_names(title, old, new, out):
    old, new = set(old), set(new)
    added, removed = sorted(new - old), sorted(old - new)
    out.append(f"### {title}: {len(old)} before, {len(new)} now")
    if added:
        out.append(f"- New ({len(added)}): " + ", ".join(f"`{n}`" for n in added[:80]) + (" ..." if len(added) > 80 else ""))
    if removed:
        out.append(f"- Gone ({len(removed)}): " + ", ".join(f"`{n}`" for n in removed[:80]) + (" ..." if len(removed) > 80 else ""))
    if not added and not removed:
        out.append("- No change")
    out.append("")
    return added, removed


def names_in(blocks):
    return [b["name"] if isinstance(b, dict) else b for b in blocks]


def report(old_version, new_version, old, new):
    (old_game, old_packets), (new_game, new_packets) = old, new
    out = [f"# From Minecraft {old_version} to {new_version}", ""]
    out.append(f"Protocol {old_game['protocol']} to {new_game['protocol']}.")
    out.append("")
    need_work = []

    out.append("## Packets")
    out.append("")
    for state in sorted(set(old_packets) | set(new_packets)):
        for side in ("clientbound", "serverbound"):
            a = old_packets.get(state, {}).get(side, {})
            b = new_packets.get(state, {}).get(side, {})
            added, removed = compare_names(f"{state} {side}", a, b, out)
            moved = [n for n in a if n in b and a[n]["protocol_id"] != b[n]["protocol_id"]]
            if moved:
                out.append(f"- Renumbered ({len(moved)}), handled by the regenerated ids.")
                out.append("")
            if added:
                effect = "An unhandled play packet is skipped safely, but one that matters won't work." if state == "play" else "Joining won't work until these are handled."
                need_work.append(f"Decide what to do with new {state} {side} packets: {', '.join(added)}. {effect}")
            if removed:
                need_work.append(f"Remove code for {state} {side} packets that are gone: {', '.join(removed)}. The build will point at them.")

    out.append("## Game contents")
    out.append("")
    compare_names("Blocks", names_in(old_game["blocks"]), names_in(new_game["blocks"]), out)
    compare_names("Items", names_in(old_game["items"]), names_in(new_game["items"]), out)
    added, _ = compare_names("Entities", names_in(old_game["entities"]), names_in(new_game["entities"]), out)
    if added:
        need_work.append(f"New entities need a look in `tools/e2e/zoo.py`, and a body if the game's models don't cover them: {', '.join(added)}.")
    added, _ = compare_names("Mob effects", names_in(old_game["mob_effects"]), names_in(new_game["mob_effects"]), out)
    old_regs, new_regs = old_game.get("registries", {}), new_game.get("registries", {})
    other = sorted(set(new_regs) - set(old_regs))
    if other:
        out.append(f"Other registries Mojang lists this time: {', '.join(other[:40])}")
        out.append("")
    for reg in ("menu", "data_component_type", "recipe_serializer", "command_argument_type", "particle_type"):
        added, removed = compare_names(f"Registry {reg}", old_regs.get(reg, []), new_regs.get(reg, []), out)
        if added or removed:
            what = {
                "menu": "`ui/inventory.rs` needs a layout for each new menu; an unknown one shows its slots plainly",
                "data_component_type": "`lattice-proto/src/item.rs` must read each new component, or items holding them won't decode",
                "recipe_serializer": "the recipe code in `lattice-proto/src/recipes.rs` may need them",
                "command_argument_type": "the command tree reader must know each new argument type, or commands won't load",
                "particle_type": "`particles.rs` may need an entry",
            }[reg]
            need_work.append(f"{reg} changed ({', '.join(added + removed)}): {what}.")
    old_fields = {k: [f[2] for f in v["fields"]] for k, v in old_game.get("entity_classes", {}).items()}
    new_fields = {k: [f[2] for f in v["fields"]] for k, v in new_game.get("entity_classes", {}).items()}
    changed = sorted(k for k in set(old_fields) & set(new_fields) if old_fields[k] != new_fields[k])
    if new_fields:
        out.append(f"### Entity data layouts changed: {len(changed)}")
    if changed:
        out.append("- " + ", ".join(f"`{k}`" for k in changed))
        need_work.append(f"Entity fields moved in {', '.join(changed)}. Those read by name follow on their own; check any read by number in `game/mod.rs`.")
    out.append("")
    if "entity_serializers" in new_game and old_game.get("entity_serializers") != new_game.get("entity_serializers"):
        need_work.append("Entity data serializers changed: `ENTITY_SERIALIZERS` in `play.rs` must match the new list.")

    out.append("## Still to do by hand")
    out.append("")
    if need_work:
        out.extend(f"- [ ] {n}" for n in need_work)
    else:
        out.append("- Nothing stands out from the data.")
    out.extend([
        "- [ ] `cargo build` and `cargo test --workspace`",
        "- [ ] `python3 -I tools/e2e/run.py`: every scenario on the new server",
        "- [ ] `python3 -I tools/e2e/zoo.py`: look at the pictures, since looks can change with no change in the data",
        "- [ ] `python3 -I tools/e2e/soak.py 10 1 3`",
        "- [ ] Read the version's changelog at minecraft.wiki for anything new to play with",
        "- [ ] `tools/release.py <version>` when it all passes",
    ])
    return "\n".join(out) + "\n"


def java_for(version_json, work):
    """A Java that this version asks for, from Mojang's own runtimes."""
    component = version_json.get("javaVersion", {}).get("component", "java-runtime-epsilon")
    java_dir = ROOT / "target" / f"java-{component}"
    # The one the rest of the tools use, if it's the right one.
    if component == "java-runtime-epsilon" and (ROOT / "target/java/bin/java").exists():
        return ROOT / "target/java"
    if not (java_dir / "bin/java").exists():
        print(f"Fetching {component}")
        subprocess.run([sys.executable, "-I", str(ROOT / "tools/fetch-java.py"), component, str(java_dir)], check=True)
    return java_dir


def generate(version, out_dir, work):
    """Runs tools/gen-data.sh for `version`, with its output going to `out_dir`."""
    manifest = fetch_json(MANIFEST)
    entry = next((v for v in manifest["versions"] if v["id"] == version), None)
    if entry is None:
        sys.exit(f"Mojang has no version called {version}. The newest release is {manifest['latest']['release']}, the newest snapshot {manifest['latest']['snapshot']}.")
    version_json = fetch_json(entry["url"])
    java = java_for(version_json, work)
    script = (ROOT / "tools/gen-data.sh").read_text()
    script = re.sub(r'VERSION="[^"]+"', f'VERSION="{version}"', script)
    script = script.replace('OUT="$ROOT/crates/lattice-data/data"', f'OUT="{out_dir}"')
    script = script.replace('ROOT="$(cd "$(dirname "$0")/.." && pwd)"', f'ROOT="{ROOT}"')
    scratch = work / "gen-data.sh"
    scratch.write_text(script)
    env = dict(os.environ, JAVA_HOME=str(java))
    done = subprocess.run(["bash", str(scratch), str(work / "gen")], env=env, capture_output=True, text=True)
    print(done.stdout, end="")
    if done.returncode != 0:
        errors = [line for line in done.stderr.splitlines() if "error:" in line or "symbol:" in line]
        return errors or done.stderr.splitlines()[-10:]
    (out_dir / "creative.json").exists() or shutil.copy(DATA / "creative.json", out_dir / "creative.json")
    return []


def facts_from_reports(gen):
    """What Mojang's own reports say, for when the extractor no longer compiles against a new version."""
    reports = gen / "generated/reports"
    registries = {k.split(":")[-1]: [n.split(":")[-1] for n in v["entries"]] for k, v in json.loads((reports / "registries.json").read_text()).items()}
    jar = next((gen / "versions").glob("*/server-*.jar"))
    with zipfile.ZipFile(jar) as z:
        protocol = json.loads(z.read("version.json"))["protocol_version"]
    game = {
        "protocol": protocol,
        "blocks": registries.get("block", []),
        "items": registries.get("item", []),
        "entities": registries.get("entity_type", []),
        "mob_effects": registries.get("mob_effect", []),
        "registries": registries,
    }
    return game, json.loads((reports / "packets.json").read_text())


def rewrite(place, new):
    path, pattern = place
    file = ROOT / path
    text = file.read_text()
    changed, n = re.subn(pattern, lambda m: m.group(1) + str(new) + m.group(3), text)
    if n == 0:
        sys.exit(f"Couldn't find the version in {path}; the pattern in VERSION_PLACES needs updating.")
    file.write_text(changed)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    if "--latest" in flags:
        latest = fetch_json(MANIFEST)["latest"]["release"]
        now = current_version()
        print(f"Lattice is on {now}; Mojang's newest release is {latest}.")
        # For scheduled checks: a non-zero exit means there's work to do.
        sys.exit(0 if now == latest else 3)
    if len(args) != 1:
        sys.exit(__doc__)
    version = args[0]
    old_version = current_version()
    work = ROOT / "target/new-version" / version
    out_dir = work / "data"
    out_dir.mkdir(parents=True, exist_ok=True)
    broken = generate(version, out_dir, work)
    old = load_facts(DATA)
    new = facts_from_reports(work / "gen") if broken else load_facts(out_dir)
    text = report(old_version, version, old, new)
    if broken:
        text = (
            "**tools/extractor/Extract.java doesn't compile against this version:** Mojang moved or renamed "
            "something it uses. Until it's fixed the game facts can't be regenerated, and this report only "
            "covers what Mojang's own reports say. Its errors:\n\n```\n" + "\n".join(broken) + "\n```\n\n" + text
        )
    target = work / "report.md"
    target.write_text(text)
    print(text)
    print(f"The report is also at {target}")
    if broken:
        sys.exit("Fix tools/extractor/Extract.java against the new jar, then run this again. The source wasn't changed.")
    if "--dry-run" in flags:
        print("Dry run: the source wasn't changed.")
        return
    for name in ("packets.json", "game.json.gz"):
        shutil.copy(out_dir / name, DATA / name)
    for place in VERSION_PLACES:
        rewrite(place, version)
    rewrite(PROTOCOL_PLACE, new[0]["protocol"])
    print(f"Moved to {version}, protocol {new[0]['protocol']}. Next: the list at the end of the report.")


if __name__ == "__main__":
    main()
