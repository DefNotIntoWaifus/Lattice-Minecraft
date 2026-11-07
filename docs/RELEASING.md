# Releasing Lattice

Lattice has two version numbers. Its own (`0.2.0`) changes whenever there's something new to
ship. The Minecraft version it speaks (`26.3`) changes when Mojang releases one. Each release says
both: "Lattice 0.2.0 for Minecraft 26.3".

## A normal release

1. Write what changed under a new `## 0.3.0` heading in `CHANGELOG.md`, for people who play.
2. Run `tools/release.py 0.3.0`.

That sets the version, builds, runs the unit tests and every scenario on a real server, packages
the Linux build into `dist/`, commits, tags and creates the GitHub release. The tag starts
`.github/workflows/release.yml`, which adds the Windows and macOS builds a few minutes later.

Add `--no-publish` to stop before anything leaves this computer.

## What's in a release

| File | For |
|---|---|
| `lattice-X-linux-x86_64.tar.gz` | A first install: the program and `install.sh` |
| `lattice-X-linux-x86_64`, `-windows-x86_64.exe`, `-macos-arm64` | Each system's program by itself, which is what the update button fetches |
| `*.sha256` | Their checksums; the update button refuses anything that doesn't match |

The launcher asks GitHub for the newest release when it starts, unless **Check for updates** is
off under Settings, Interface. That only works if the repository is public: GitHub answers
everyone else with "not found", and the launcher then says nothing. The address is at the top of
`crates/lattice/src/launcher/update.rs`.

## When a new Minecraft version comes out

`.github/workflows/minecraft-version.yml` looks at Mojang's list every day and opens an issue when
there's a new release. To see for yourself: `tools/new-version.py --latest`.

1. **See what changed**, with nothing touched:

   ```sh
   tools/new-version.py 26.4 --dry-run
   ```

   This downloads the new server from Mojang, checks its hash, runs Mojang's data generator and
   Lattice's extractor, and compares the result with what's in the repository. The report lists
   every new, removed and renumbered packet, block, item, entity, menu and data component, then
   what each difference means for the code. It works on snapshots too, so a release can be
   prepared before it's out.

2. **Move to it**, which rewrites the facts and every place that names the version:

   ```sh
   tools/new-version.py 26.4
   ```

3. **Work through the report's list.** New block states, item ids and renumbered packets follow on
   their own. These need a person:
   - A new or changed packet layout. The real server fails loudly in scenarios if a layout's wrong:
     a decode with bytes left over fails the run. Read the truth from the jar with
     `tools/jread.py`, not from memory; the wiki has been wrong before.
   - A new data component, which items can't be read without.
   - A new menu, which otherwise shows plain rows of slots.
   - New mobs: most follow from the game's own models, but check `tools/e2e/zoo.py`'s pictures.

4. **Test it**: `cargo test --workspace`, `tools/e2e/run.py`, `tools/e2e/zoo.py` and look at the
   pictures, `tools/e2e/soak.py 10 1 3`.

5. **Release it** as above. A Minecraft update is a new Lattice version too, so people can move in
   step with the servers they play on.

Lattice only speaks one version at a time. Its server list shows clearly when a server's on another.
