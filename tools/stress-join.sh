#!/usr/bin/env bash
# Regression test for join and teleport timing. Needs an offline-mode server you
# can send console commands to through a named pipe, with
# pause-when-empty-seconds=2 in server.properties so every round joins a server
# that was just paused:
#
#   mkfifo console.fifo
#   exec 3<>console.fifo; java -jar server.jar nogui <&3 > server.log 2>&1
#
#   tools/stress-join.sh ADDRESS SERVER_LOG CONSOLE_FIFO [ROUNDS]
#
# Each round joins three sessions that sprint, turn and jump while being
# teleported. It then counts what the server said about them. Everything except
# the count of joins must be zero. Run Lattice once first, so the game files are
# in its cache: sessions here only live for a few seconds.
set -euo pipefail

ADDRESS="$1"; LOG="$2"; FIFO="$3"; ROUNDS="${4:-5}"
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/lattice"
before=$(wc -l < "$LOG")

for _ in $(seq 1 "$ROUNDS"); do
  for n in 1 2 3; do
    LATTICE_SCRIPT="0.3:w+,0.3:sprint+,4:yaw=120,6:jump+,8:yaw=250" LATTICE_EXIT_AFTER=11 \
      timeout 40 "$BIN" --join "$ADDRESS" --name "Stress$n" >/dev/null 2>&1 &
  done
  ( sleep 6; for _ in 1 2 3 4 5 6; do echo "tp @a ~ ~1 ~" > "$FIFO"; sleep 0.5; done ) &
  wait
  sleep 4 # long enough for the server to pause while empty
done

new=$(tail -n +$((before + 1)) "$LOG")
count() { grep -c "$1" <<<"$new" || true; }
echo "joins:          $(count 'joined the game')"
echo "kicked:         $(count 'Invalid move')"
echo "moved too fast: $(count 'moved too quickly')"
echo "moved wrongly:  $(count 'moved wrongly')"
if [[ "$(count 'joined the game')" == 0 ]]; then
  echo "FAILED: nobody joined, so this run tested nothing." >&2
  exit 1
fi
if [[ "$(count 'Invalid move')$(count 'moved too quickly')$(count 'moved wrongly')" != 000 ]]; then
  echo "FAILED: the server rejected movement." >&2
  exit 1
fi
echo "passed"
