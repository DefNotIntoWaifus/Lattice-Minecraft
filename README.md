# Lattice

A client for *Minecraft: Java Edition* servers, written from scratch in Rust.
It doesn't launch, wrap or patch the official game: it speaks the network
protocol (26.3, protocol 777), draws the world with its own renderer and has
its own interface. Textures, models and sounds come from Mojang's own files,
which it downloads and checks on first start; none of them are in this repository.

**Status:** playable on Minecraft 26.3 servers and on worlds it runs on your own computer.
Survival and creative, every mob and block, the menus, villagers, command blocks, riding,
weather, maps, signed chat and resource packs work. Some things are still missing or
rough; [docs/ROADMAP.md](docs/ROADMAP.md) lists them, and the
[issues](https://github.com/DefNotIntoWaifus/Lattice-Minecraft/issues) are the place to report more.

| | |
|---|---|
| Plays | Minecraft: Java Edition 26.3 (protocol 777) |
| Runs on | Linux (tested); Windows and macOS builds come from CI with each release and haven't been tried by hand yet |
| Needs | A GPU with Vulkan, Metal or DirectX 12 |

- Design: [docs/DESIGN.md](docs/DESIGN.md)
- What changed in each release: [CHANGELOG.md](CHANGELOG.md)
- Making a release, and moving to a new Minecraft version: [docs/RELEASING.md](docs/RELEASING.md)
- Helping out: [CONTRIBUTING.md](CONTRIBUTING.md)

## Install it

On Linux, download `lattice-…-linux-x86_64.tar.gz` from the
[latest release](https://github.com/DefNotIntoWaifus/Lattice-Minecraft/releases/latest), unpack it and
run `install.sh`. On Windows and macOS, download that system's program from the same page and run it.
From the source, with Rust 1.90 or newer, it's `tools/install.sh`.

Lattice then appears in your application menu under Games, and on your desktop. The launcher
tells you when there's a newer version and can install it. `uninstall.sh` removes the app; your
settings, sign-ins and worlds stay either way.

Each Lattice release plays one Minecraft version, which it says in its name.

Opening it starts the launcher. It stays open while you play: pick who you're playing
as and a server, press **Play**, and the game opens in its own window. Press it
again for another session side by side, each with live frame rate, ping, memory
and logs on the launcher's Sessions page.

On first start Lattice downloads the official game resources (about 41 MB)
from Mojang, checks them, and keeps them in the cache folder, then fetches the sound
effects in the background (about 83 MB; music as it's needed). Textures, mob shapes
and sounds all come from those files; nothing of Mojang's is in this repository.

Elsewhere, or to try a change without installing it, `cargo run --release` opens
the launcher directly.

To open one game window without the launcher:

```sh
lattice --join localhost:25566 --name Alex
```

### Play on your own, no account or server needed

On the launcher's **Play** page, press **New world**, choose a game mode and press
**Create and play**. The world runs on this computer with Mojang's own server program,
which Lattice downloads the first time along with the Java it needs (about 170 MB, checked
against Mojang's published sums). That program's license is Mojang's, so the form asks you
to accept it first. Only this computer can reach the world, and it saves and closes by
itself when the last game window closes. Worlds are ordinary server folders under
`worlds/` in the data folder, so backing one up means copying that folder.

### Try it on a server you run yourself

```sh
tools/test-server.sh          # downloads Java 25 and the 26.3 server once, asks you to accept the EULA
```

Then in the launcher: **Add server** → address `localhost:25566` → **Play**.

### Test with several players

Press Play again on a server you're already on. With an offline name, the new
session joins under the next free name (`Alex2`, `Alex3`, …), and the launcher
tells you which before you press it. Add more names under **Accounts** to choose
them yourself. Windows without focus draw at 30 fps so they don't take GPU time
from the one you're playing; turn that off in Settings to measure them properly.
VSync caps the frame rate at your display's refresh rate, so turn it off too when
you want to see what the GPU can really do.

### Sign in with your Microsoft account

Online-mode servers check your account with Mojang. Mojang only lets
**approved** applications do that, so you need your own application ID:

1. Register an app at [portal.azure.com](https://portal.azure.com) → App
   registrations → *Personal Microsoft accounts only*. Under Authentication,
   turn on *Allow public client flows*. No secret is needed.
2. Ask Mojang to approve that app ID for the Minecraft API:
   <https://aka.ms/mce-reviewappid>.
3. Paste the ID in the launcher under **Accounts**, or set `LATTICE_CLIENT_ID`.

Until it's approved, Microsoft answers with an error Lattice explains on screen,
and offline-mode servers and worlds on this computer keep working. Once signed in, chat
and commands are signed with your account's keys, which servers that enforce secure
chat require.

## Controls

| Key | Action |
|---|---|
| W A S D | move |
| Space / Shift / Ctrl | jump / sneak / sprint |
| Mouse, left / right button | look, break / use or place |
| 1–9, wheel | select hotbar slot |
| E, L | inventory, advancements |
| C or X with 1-9 | in creative: save or load a hotbar |
| T or Enter, / | chat, command; Tab completes |
| Tab | player list |
| Q, F | drop, swap hands |
| F2, F3, F5, F11 | screenshot, details, camera, fullscreen |
| F3 with B, G, H | hit boxes, chunk borders, item details |
| F3 with F4, N | game mode list, spectator (if you're allowed) |
| F3 with C, I, D, T, A | copy position, copy block or mob data, clear chat, reload packs, redraw |
| Esc | pause |

Every one of these can be changed in **Settings → Key bindings**.

## Files

| | Linux | Windows | macOS |
|---|---|---|---|
| Settings, servers, accounts | `~/.config/lattice` | `%APPDATA%\lattice\config` | `~/Library/Application Support/dev.lattice.lattice` |
| Downloaded game files, last frames | `~/.cache/lattice` | `%LOCALAPPDATA%\lattice\cache` | `~/Library/Caches/dev.lattice.lattice` |
| Logs, screenshots | `~/.local/share/lattice` | `%APPDATA%\lattice\data` | `~/Library/Application Support/dev.lattice.lattice` |

`accounts.json` holds refresh tokens and is written with owner-only
permissions. On Linux these paths follow the XDG variables, so an app started
from a sandbox such as Flatpak VS Code keeps its files under that sandbox's
folders (`~/.var/app/<id>/…`) instead.

## Accessibility

Under **Settings → Interface**: dark, light and high contrast themes, interface size up to 2×,
and reduce motion. Every control works from the keyboard (Tab, Enter or Space, Esc to leave;
Ctrl+1 to Ctrl+4 switch launcher pages) and reports itself to screen readers.

## Resource packs

Put pack files in the folder shown under **Settings → Resource packs**, and switch them on there;
they apply straight away, even mid-game. Packs a server offers are used by default, and
that page can decline them instead. Packs only change pictures, sounds and text.

## Develop

```text
crates/lattice-proto    wire format and packets
crates/lattice-net      connections, server-list ping
crates/lattice-auth     Microsoft sign-in, session join
crates/lattice-data     generated game facts (do not edit by hand)
crates/lattice-assets   game resource download and access
crates/lattice-world    chunks, light, registries, player physics
crates/lattice-render   models, atlas, meshing, wgpu renderer
crates/lattice          the launcher and game windows (one binary)
tools/                 data generation, Java fetch, local test server
```

```sh
cargo test --workspace
python3 -I tools/e2e/run.py [names]        # plays scenarios on a real server, no window needed
python3 -I tools/e2e/zoo.py [groups]       # pictures of every mob, item, menu and effect, to look at
python3 -I tools/e2e/soak.py [minutes] [seed] [bots]  # plays badly for a long time, looking for what breaks
python3 -I tools/e2e/packs.py              # a server that requires a resource pack, and switching on your own
python3 -I tools/e2e/local_world.py        # a local world from nothing: download, play, save, reopen
cargo run -p lattice-net --example probe -- localhost:25566 Tester   # headless join, reports decode problems
tools/new-version.py 26.4 --dry-run        # what a new Minecraft version changes (docs/RELEASING.md)
tools/release.py 0.3.0                     # build, test, package and publish a release
```

The test tools need a server jar, a client jar and Java under `target/`; `tools/reference/fetch.py`
fetches them. They use their own port and folders, so they don't touch your worlds or settings.

Development-only environment variables for scripted runs: `LATTICE_HOME`,
`LATTICE_AUTOLAUNCH`, `LATTICE_SCRIPT`, `LATTICE_SHOTS`, `LATTICE_EXIT_AFTER`,
`LATTICE_SCREEN` and the `LATTICE_CHILD_*` versions (see DESIGN §13).

## Credits

- Protocol documentation: [minecraft.wiki](https://minecraft.wiki/w/Java_Edition_protocol) (CC BY-SA 3.0).
- Game window typeface: [Jersey 15](https://github.com/scfried/soft-type-jersey), SIL Open Font License 1.1.
- Typeface: [Atkinson Hyperlegible Next](https://github.com/googlefonts/atkinson-hyperlegible-next), SIL Open Font License (`crates/lattice/assets/fonts/OFL.txt`).

## License

Lattice's code is licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. The fonts in `crates/lattice/assets/fonts` keep their own
SIL Open Font License. Mojang's server program and resource files, which Lattice downloads, are
under Mojang's own terms, and the server's EULA has to be accepted before a local world starts.

NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
