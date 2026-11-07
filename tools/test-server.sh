#!/usr/bin/env bash
# Runs the official 26.3 server locally in offline mode, for trying Lattice
# without a Microsoft account. Everything lives in ./target/test-server.
#
#   tools/test-server.sh            # first run downloads Java + the server
#   then in Lattice: Add server → address "localhost:25566"
#
# Stop it by typing "stop" in this terminal.
set -euo pipefail

VERSION="26.3"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="$ROOT/target/test-server"
JAVA_DIR="$ROOT/target/java"
mkdir -p "$DIR"
cd "$DIR"

JAVA="$(command -v java || true)"
if [[ -z "$JAVA" ]] || ! "$JAVA" -version 2>&1 | grep -qE 'version "2[5-9]'; then
  if [[ ! -x "$JAVA_DIR/bin/java" ]]; then
    echo "Downloading Mojang's Java 25 runtime (about 50 MB, once)"
    python3 -I "$ROOT/tools/fetch-java.py" java-runtime-epsilon "$JAVA_DIR"
  fi
  JAVA="$JAVA_DIR/bin/java"
fi

if [[ ! -f server.jar ]]; then
  echo "Downloading the official $VERSION server from Mojang"
  manifest="https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
  version_url=$(curl -fsS "$manifest" | python3 -I -c "import json,sys; print(next(v['url'] for v in json.load(sys.stdin)['versions'] if v['id']=='$VERSION'))")
  read -r url sha1 < <(curl -fsS "$version_url" | python3 -I -c "import json,sys; d=json.load(sys.stdin)['downloads']['server']; print(d['url'], d['sha1'])")
  curl -fsS -o server.jar.part "$url"
  echo "$sha1  server.jar.part" | sha1sum -c - >/dev/null
  mv server.jar.part server.jar
fi

if ! grep -qs '^eula=true' eula.txt; then
  echo
  echo "Running the server requires accepting the Minecraft EULA:"
  echo "  https://aka.ms/MinecraftEULA"
  read -r -p "Type 'yes' to accept it: " answer
  [[ "$answer" == "yes" ]] || { echo "Not accepted; nothing was started."; exit 1; }
  echo "eula=true" > eula.txt
fi

if [[ ! -f server.properties ]]; then
  cat > server.properties <<'PROPS'
online-mode=false
server-ip=127.0.0.1
server-port=25566
motd=§aLocal test server §7(offline mode)
white-list=false
spawn-protection=0
view-distance=10
simulation-distance=6
PROPS
fi

exec "$JAVA" -Xms512M -Xmx2G -jar server.jar nogui
