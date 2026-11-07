#!/usr/bin/env bash
# Regenerates crates/lattice-data/data/ from the official server jar.
#
# Needs: a JDK whose version matches the game's requirement (26.3 → Java 25),
# curl, python3. End users never run this; it is only for version bumps.
#
#   JAVA_HOME=/path/to/jdk tools/gen-data.sh [workdir]
#
# The workdir (default: ./target/gen-data) holds the downloaded server jar and
# the generator output. Nothing from it is committed except the files written
# to crates/lattice-data/data/.
set -euo pipefail

VERSION="26.3"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${1:-$ROOT/target/gen-data}"
OUT="$ROOT/crates/lattice-data/data"
JAVA="${JAVA_HOME:+$JAVA_HOME/bin/}java"
JAVAC="${JAVA_HOME:+$JAVA_HOME/bin/}javac"

mkdir -p "$WORK" "$OUT"
cd "$WORK"

if [[ ! -f server.jar ]]; then
  echo "Resolving $VERSION from Mojang's version manifest"
  manifest_url="https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
  version_url=$(curl -fsS "$manifest_url" | python3 -I -c "import json,sys; m=json.load(sys.stdin); print(next(v['url'] for v in m['versions'] if v['id']=='$VERSION'))")
  read -r jar_url jar_sha1 < <(curl -fsS "$version_url" | python3 -I -c "import json,sys; d=json.load(sys.stdin)['downloads']['server']; print(d['url'], d['sha1'])")
  curl -fsS -o server.jar.part "$jar_url"
  echo "$jar_sha1  server.jar.part" | sha1sum -c -
  mv server.jar.part server.jar
fi

echo "Running the data generator"
"$JAVA" -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports --output generated >/dev/null

CP="versions/$VERSION/server-$VERSION.jar:$(find libraries -name '*.jar' | tr '\n' ':')"

echo "Compiling and running the extractor"
mkdir -p extractor-classes
"$JAVAC" -nowarn -cp "$CP" -d extractor-classes "$ROOT/tools/extractor/Extract.java"
"$JAVA" -cp "extractor-classes:$CP" Extract facts.json

echo "Writing $OUT"
cp generated/reports/packets.json "$OUT/packets.json"
python3 -I - "$OUT" <<'EOF'
import gzip, json, sys
out = sys.argv[1]
facts = json.load(open("facts.json"))
reg = json.load(open("generated/reports/registries.json"))
# Registries the client needs IDs for but that the server never sends during
# configuration (they are static, "built-in" registries).
keep = ["minecraft:block", "minecraft:item", "minecraft:entity_type", "minecraft:menu",
        "minecraft:particle_type", "minecraft:sound_event", "minecraft:mob_effect",
        "minecraft:data_component_type", "minecraft:attribute", "minecraft:block_entity_type",
        "minecraft:fluid", "minecraft:stat_type", "minecraft:custom_stat", "minecraft:game_event",
        "minecraft:command_argument_type", "minecraft:recipe_serializer", "minecraft:villager_type",
        "minecraft:villager_profession", "minecraft:potion", "minecraft:entity_data_serializer",
        "minecraft:recipe_book_category", "minecraft:map_decoration_type"]
registries = {}
for name in keep:
    if name not in reg:
        continue
    entries = reg[name]["entries"]
    ordered = sorted(entries.items(), key=lambda kv: kv[1]["protocol_id"])
    assert [v["protocol_id"] for _, v in ordered] == list(range(len(ordered))), name
    registries[name.removeprefix("minecraft:")] = [k.removeprefix("minecraft:") for k, _ in ordered]
facts["registries"] = registries
# Default item components, from the generator's per-item report. Values every item has by
# default are dropped; code treats a missing component as that default.
COMMON = {"minecraft:attack_animation": {}, "minecraft:interact_animation": {}, "minecraft:lore": [],
          "minecraft:enchantments": {}, "minecraft:repair_cost": 0, "minecraft:tooltip_display": {},
          "minecraft:use_effects": {}, "minecraft:break_sound": "minecraft:entity.item.break",
          "minecraft:attribute_modifiers": [], "minecraft:rarity": "common"}
items = []
for name in registries["item"]:
    comps = json.load(open(f"generated/reports/minecraft/components/item/{name}.json"))["components"]
    items.append({"name": name, "components": {k.removeprefix("minecraft:"): v for k, v in comps.items() if COMMON.get(k, None) != v}})
facts["items"] = items
# Block groups that movement rules depend on. Servers send their own copies of these tags;
# these are the defaults, used until they do and in tests.
import zipfile
jar = zipfile.ZipFile("versions/26.3/server-26.3.jar")
def tag(name):
    out = []
    for v in json.loads(jar.read(f"data/minecraft/tags/block/{name}.json"))["values"]:
        v = v["id"] if isinstance(v, dict) else v
        out += tag(v[1:].removeprefix("minecraft:")) if v.startswith("#") else [v.removeprefix("minecraft:")]
    return out
facts["block_tags"] = {t: sorted(set(tag(t))) for t in ["climbable", "blocks_fluid_flow", "suppresses_bounce", "can_glide_through", "fences", "walls", "fence_gates", "trapdoors"]}
# Which states suffocate: the game's rule is this tag and a full collision shape. Tags only load
# with a world, so the extractor can't ask the block itself.
suffocating = set(tag("causes_suffocation"))
blocks = facts["blocks"]
for i, b in enumerate(blocks):
    end = blocks[i + 1]["first"] if i + 1 < len(blocks) else len(facts["states"])
    for st in facts["states"][b["first"]:end]:
        if st[2] & (1 << 19):
            st[2] &= ~(1 << 19)
        elif b["name"] in suffocating and st[2] & (1 << 7):
            st[2] |= 1 << 18
with gzip.open(f"{out}/game.json.gz", "wt", compresslevel=9) as f:
    json.dump(facts, f, separators=(",", ":"))
print("blocks:", len(facts["blocks"]), "states:", len(facts["states"]), "shapes:", len(facts["shapes"]),
      "items:", len(items), "sound types:", len(facts["sound_types"]), "entity classes:", len(facts["entity_classes"]),
      "registries:", ", ".join(f"{k}={len(v)}" for k, v in registries.items()))
EOF
echo "Done."
