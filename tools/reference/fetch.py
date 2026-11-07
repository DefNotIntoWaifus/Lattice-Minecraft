#!/usr/bin/env python3
"""Downloads what the official 26.3 client needs to run on this machine as a test
reference: its jar, libraries and asset index, from Mojang's servers, SHA-1 checked.

Everything lands in target/reference/, outside the project's sources. Lattice itself
never uses any of this; see docs/ROADMAP.md, "How everything gets tested".

    tools/reference/fetch.py [target/reference]
"""

import concurrent.futures
import hashlib
import json
import pathlib
import platform
import sys
import urllib.request

MANIFEST = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
VERSION = "26.3"
OS_NAME = {"Linux": "linux", "Darwin": "osx", "Windows": "windows"}[platform.system()]


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "lattice-reference-fetch"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def download(url: str, dest: pathlib.Path, sha1: str | None) -> None:
    if dest.exists() and (sha1 is None or hashlib.sha1(dest.read_bytes()).hexdigest() == sha1):
        return
    data = fetch(url)
    if sha1 is not None and hashlib.sha1(data).hexdigest() != sha1:
        raise RuntimeError(f"checksum mismatch for {url}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)


def allowed(lib: dict) -> bool:
    rules = lib.get("rules")
    if not rules:
        return True
    ok = False
    for rule in rules:
        os_rule = rule.get("os", {})
        if os_rule.get("name", OS_NAME) == OS_NAME:
            ok = rule["action"] == "allow"
    return ok


def main() -> None:
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/reference").resolve()
    root.mkdir(parents=True, exist_ok=True)
    manifest = json.loads(fetch(MANIFEST))
    entry = next(v for v in manifest["versions"] if v["id"] == VERSION)
    version = json.loads(fetch(entry["url"]))
    (root / "version.json").write_text(json.dumps(version, indent=1))

    client = version["downloads"]["client"]
    download(client["url"], root / "client.jar", client["sha1"])

    libs = [lib["downloads"]["artifact"] for lib in version["libraries"] if allowed(lib) and "artifact" in lib["downloads"]]
    with concurrent.futures.ThreadPoolExecutor(8) as pool:
        futures = [pool.submit(download, a["url"], root / "libraries" / a["path"], a["sha1"]) for a in libs]
        for f in futures:
            f.result()

    index = version["assetIndex"]
    download(index["url"], root / "assets" / "indexes" / f"{index['id']}.json", index["sha1"])

    # Everything except sounds and other languages; the client refuses to start without the rest.
    objects = json.loads((root / "assets" / "indexes" / f"{index['id']}.json").read_text())["objects"]
    wanted = {name: o for name, o in objects.items() if not name.startswith(("minecraft/sounds/", "minecraft/lang/"))}
    with concurrent.futures.ThreadPoolExecutor(8) as pool:
        futures = [
            pool.submit(download, f"https://resources.download.minecraft.net/{o['hash'][:2]}/{o['hash']}", root / "assets" / "objects" / o["hash"][:2] / o["hash"], o["hash"])
            for o in wanted.values()
        ]
        for f in futures:
            f.result()
    print(f"{len(wanted)} of {len(objects)} asset objects")

    classpath = [str(root / "libraries" / a["path"]) for a in libs]
    (root / "classpath.txt").write_text("\n".join(classpath) + "\n")
    print(f"{len(libs)} libraries, asset index {index['id']}, main class {version['mainClass']}")
    print("game arguments:", " ".join(a for a in version["arguments"]["game"] if isinstance(a, str)))
    jvm = [a for a in version["arguments"]["jvm"] if isinstance(a, str)]
    print("jvm arguments:", " ".join(jvm))


if __name__ == "__main__":
    main()
