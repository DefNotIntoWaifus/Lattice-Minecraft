#!/usr/bin/env python3
"""A server that requires a resource pack, served from here over HTTP. The client
has to fetch it, tell the server, and then really draw with it: the pack turns
dirt bright magenta, and the picture is checked for that colour."""

import hashlib
import http.server
import io
import pathlib
import struct
import sys
import threading
import time
import zipfile
import zlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rig  # noqa: E402

PORT = 25590


def png(width, height, rgba):
    """A one-colour PNG."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    row = b"\0" + bytes(rgba) * width
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(row * height)) + chunk(b"IEND", b"")


def make_pack(blocks, rgba):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as z:
        z.writestr("pack.mcmeta", '{"pack":{"pack_format":80,"description":"test"}}')
        for block in blocks:
            z.writestr(f"assets/minecraft/textures/block/{block}.png", png(16, 16, rgba))
    return out.getvalue()


class Handler(http.server.BaseHTTPRequestHandler):
    pack = b""
    hits = 0

    def do_GET(self):
        Handler.hits += 1
        self.send_response(200)
        self.send_header("Content-Type", "application/zip")
        self.send_header("Content-Length", str(len(Handler.pack)))
        self.end_headers()
        self.wfile.write(Handler.pack)

    def log_message(self, *args):
        pass


def read_png(path):
    """Width, height and rows of an 8-bit RGB or RGBA PNG, with the standard library only."""
    b = pathlib.Path(path).read_bytes()
    i, data = 8, b""
    while i < len(b):
        n = struct.unpack(">I", b[i:i + 4])[0]
        kind, chunk = b[i + 4:i + 8], b[i + 8:i + 8 + n]
        i += 12 + n
        if kind == b"IHDR":
            w, h, _, colour = struct.unpack(">IIBB", chunk[:10])
        elif kind == b"IDAT":
            data += chunk
    channels = {2: 3, 6: 4}[colour]
    raw = zlib.decompress(data)
    stride = w * channels + 1
    rows, previous = [], bytearray(w * channels)
    for y in range(h):
        kind = raw[y * stride]
        row = bytearray(raw[y * stride + 1:(y + 1) * stride])
        for x in range(len(row)):
            a = row[x - channels] if x >= channels else 0
            up = previous[x]
            c = previous[x - channels] if x >= channels else 0
            if kind == 1:
                row[x] = (row[x] + a) & 255
            elif kind == 2:
                row[x] = (row[x] + up) & 255
            elif kind == 3:
                row[x] = (row[x] + (a + up) // 2) & 255
            elif kind == 4:
                p = a + up - c
                pa, pb, pc = abs(p - a), abs(p - up), abs(p - c)
                row[x] = (row[x] + (a if pa <= pb and pa <= pc else up if pb <= pc else c)) & 255
        rows.append(row)
        previous = row
    return w, h, channels, rows


def share_of(path, test):
    """How much of the lower half of a picture passes a colour test."""
    w, h, channels, rows = read_png(path)
    total = found = 0
    for y in range(h // 2, h, 6):
        for x in range(0, w, 6):
            r, g, b = rows[y][x * channels:x * channels + 3]
            total += 1
            if test(r, g, b):
                found += 1
    return found / total


def main():
    Handler.pack = make_pack(["dirt", "grass_block_top", "grass_block_side", "stone"], (255, 0, 255, 255))
    sha = hashlib.sha1(Handler.pack).hexdigest()
    web = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=web.serve_forever, daemon=True).start()
    extra = f"resource-pack=http://127.0.0.1:{PORT}/pack.zip\nresource-pack-sha1={sha}\nrequire-resource-pack=true\n"
    server = rig.Server(fresh=True, extra=extra)
    # The player's own pack, there to be switched on from the settings mid-game.
    own = rig.WORK / "home-Packed/data/resourcepacks"
    own.mkdir(parents=True, exist_ok=True)
    (own / "cyan.zip").write_bytes(make_pack(["sand"], (0, 255, 255, 255)))
    bot = rig.Client("Packed")
    failed = False
    try:
        bot.wait_playing(120)
        time.sleep(1)
        bot.ticks(30)
        print("pack fetched", Handler.hits, "time(s)")
        if Handler.hits < 1:
            print("FAIL: the client never fetched the pack")
            failed = True
        bot.look(0, 60)
        bot.ticks(60)
        picture = bot.capture("resource-pack", world_only=True)
        # Grass tints what it's given, so the magenta comes out dark.
        share = share_of(picture, lambda r, g, b: r > 90 and b > 60 and g < 60)
        print(f"magenta share of the ground: {share:.2f}, picture {picture}")
        if share < 0.3:
            print("FAIL: the pack's textures aren't being drawn")
            failed = True
        # The player's own pack, switched on while playing.
        server.send("op Packed")
        d = bot.dump()
        x, y, z = (int(v) for v in d["pos"])
        server.send(f"fill {x - 12} {y - 1} {z - 12} {x + 12} {y - 1} {z + 12} sand")
        bot.ticks(40)
        bot.press("Escape")
        bot.ticks(3)
        bot.click("Settings")
        bot.ticks(3)
        bot.click("section:Resource packs")
        bot.ticks(3)
        bot.capture("settings-packs")
        bot.click("setting:cyan.zip")
        bot.ticks(3)
        bot.click("Back to game")
        for _ in range(40):
            time.sleep(1)
            bot.ticks(5)
            picture = bot.capture("resource-pack-own", world_only=True)
            share = share_of(picture, lambda r, g, b: g > 120 and b > 120 and r < 90)
            if share > 0.3:
                break
        print(f"cyan share after switching the pack on: {share:.2f}, picture {picture}")
        if share < 0.3:
            print("FAIL: switching a pack on mid-game didn't change the world")
            failed = True
        said = [l for l in server.lines.all if "resource pack" in l.lower() or "lost connection" in l.lower() or "kicked" in l.lower()]
        print("server said:", said[:5])
        if any("lost connection" in l.lower() for l in said):
            print("FAIL: the server dropped the client")
            failed = True
        problems = bot.problems()
        if problems:
            print("client complained:", problems[:5])
    finally:
        bot.quit()
        server.stop()
        web.shutdown()
    print("FAILED" if failed else "all passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
