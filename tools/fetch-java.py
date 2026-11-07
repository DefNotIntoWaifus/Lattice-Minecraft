#!/usr/bin/env python3
"""Downloads Mojang's official Java runtime (the one its launcher uses) into a
directory, verifying every file's SHA-1. Only needed for development tools:
tools/gen-data.sh and tools/test-server.sh. Lattice itself never runs Java.

    python3 tools/fetch-java.py <component> <dest>
    e.g. python3 tools/fetch-java.py java-runtime-epsilon target/java
"""
import hashlib, json, os, platform, sys, urllib.request

ALL = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json"

def platform_key():
    s, m = platform.system(), platform.machine().lower()
    if s == "Linux":
        return "linux" if m in ("x86_64", "amd64") else "linux-i386"
    if s == "Darwin":
        return "mac-os-arm64" if m == "arm64" else "mac-os"
    return "windows-arm64" if "arm" in m else "windows-x64"

def main():
    component, dest = sys.argv[1], sys.argv[2]
    entries = json.load(urllib.request.urlopen(ALL))[platform_key()].get(component) or []
    if not entries:
        sys.exit(f"Mojang publishes no {component} runtime for this platform")
    manifest = json.load(urllib.request.urlopen(entries[0]["manifest"]["url"]))
    print(f"Java {entries[0]['version']['name']} -> {dest}")
    files = manifest["files"]
    for path, info in files.items():
        full = os.path.join(dest, path)
        if info["type"] == "directory":
            os.makedirs(full, exist_ok=True)
        elif info["type"] == "file":
            raw = info["downloads"]["raw"]
            if os.path.exists(full) and hashlib.sha1(open(full, "rb").read()).hexdigest() == raw["sha1"]:
                continue
            os.makedirs(os.path.dirname(full), exist_ok=True)
            data = urllib.request.urlopen(raw["url"]).read()
            if hashlib.sha1(data).hexdigest() != raw["sha1"]:
                sys.exit(f"checksum mismatch for {path}")
            with open(full, "wb") as f:
                f.write(data)
            if info.get("executable"):
                os.chmod(full, 0o755)
    for path, info in files.items():
        full = os.path.join(dest, path)
        if info["type"] == "link" and not os.path.lexists(full):
            os.symlink(info["target"], full)

if __name__ == "__main__":
    main()
