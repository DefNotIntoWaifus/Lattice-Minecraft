#!/usr/bin/env python3
"""Packages a release of Lattice, and publishes it to GitHub.

    tools/release.py 0.3.0             build, test, package, tag and publish
    tools/release.py 0.3.0 --no-publish  everything up to the upload, and leave it in dist/
    tools/release.py 0.3.0 --skip-tests  for when the scenarios have just passed

Before running this, describe the release under its version in CHANGELOG.md: the notes come from
there, and a release with nothing written about it is refused. The version in Cargo.toml is set
here. The program for other systems comes from the workflow in .github/workflows/release.yml,
which starts when the tag arrives on GitHub and adds its files to the same release.
"""

import hashlib
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"


def run(*args, **kw):
    print("+", " ".join(str(a) for a in args))
    return subprocess.run(args, check=True, cwd=ROOT, **kw)


def host(*args):
    """`gh` lives outside a sandboxed editor."""
    prefix = ["flatpak-spawn", "--host"] if pathlib.Path("/.flatpak-info").exists() else []
    return prefix + list(args)


def game_version():
    text = (ROOT / "crates/lattice-proto/src/packets/handshake.rs").read_text()
    return re.search(r'GAME_VERSION: &str = "([^"]+)"', text).group(1)


def notes_for(version):
    text = (ROOT / "CHANGELOG.md").read_text()
    match = re.search(rf"^## {re.escape(version)}\b.*?$(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not match or not match.group(1).strip():
        sys.exit(f"CHANGELOG.md has nothing under '## {version}'. Write what changed first.")
    return match.group(1).strip()


def set_version(version):
    file = ROOT / "Cargo.toml"
    text = file.read_text()
    changed, n = re.subn(r'(\[workspace\.package\]\nversion = ")[^"]+(")', rf"\g<1>{version}\g<2>", text)
    if n != 1:
        sys.exit("Couldn't find the version in Cargo.toml.")
    file.write_text(changed)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checksum(path):
    line = f"{sha256(path)}  {path.name}\n"
    (path.parent / f"{path.name}.sha256").write_text(line)


def package_linux(version):
    """The bare program, for updating in place, and an archive with its installer, for a first install."""
    DIST.mkdir(exist_ok=True)
    target = f"lattice-{version}-linux-x86_64"
    bare = DIST / target
    shutil.copy(ROOT / "target/release/lattice", bare)
    bare.chmod(0o755)
    checksum(bare)
    archive = DIST / f"{target}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        def add(source, name, mode=0o644):
            info = tar.gettarinfo(source, arcname=f"{target}/{name}")
            info.mode = mode
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            with open(source, "rb") as f:
                tar.addfile(info, f)
        add(ROOT / "target/release/lattice", "lattice", 0o755)
        add(ROOT / "tools/install.sh", "install.sh", 0o755)
        add(ROOT / "tools/uninstall.sh", "uninstall.sh", 0o755)
        add(ROOT / "crates/lattice/assets/icon/lattice.svg", "lattice.svg")
        add(ROOT / "crates/lattice/assets/icon/lattice-256.png", "lattice-256.png")
        add(ROOT / "README.md", "README.md")
    checksum(archive)
    return [archive, archive.with_name(archive.name + ".sha256"), bare, bare.with_name(bare.name + ".sha256")]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    if len(args) != 1 or not re.fullmatch(r"\d+\.\d+\.\d+", args[0]):
        sys.exit(__doc__)
    version = args[0]
    notes = notes_for(version)
    game = game_version()
    set_version(version)
    run("cargo", "build", "--release", "-p", "lattice")
    if "--skip-tests" not in flags:
        run("cargo", "test", "--workspace", "--release")
        run("cargo", "build", "-p", "lattice")
        run(sys.executable, "-I", "tools/e2e/run.py")
    files = package_linux(version)
    title = f"Lattice {version} for Minecraft {game}"
    body = f"{notes}\n\nFor Minecraft: Java Edition {game}. On Linux, unpack the archive and run `install.sh`; Lattice then offers later updates itself.\n"
    notes_file = DIST / f"notes-{version}.md"
    notes_file.write_text(body)
    print(f"\nPackaged {title} in {DIST}")
    if "--no-publish" in flags:
        return
    run("git", "add", "-A")
    if subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=ROOT).returncode != 0:
        run("git", "commit", "-m", f"Lattice {version}\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>")
    run("git", "tag", "-a", f"v{version}", "-m", title)
    # With the desktop's sign-in, which a sandboxed editor's git can't reach.
    subprocess.run(host("git", "-C", str(ROOT), "push", "--follow-tags", "origin", "main"), check=True)
    subprocess.run(host("gh", "release", "create", f"v{version}", "--title", title, "--notes-file", str(notes_file), *map(str, files)), check=True, cwd=ROOT)


if __name__ == "__main__":
    main()
