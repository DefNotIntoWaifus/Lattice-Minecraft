# Lattice — design

Lattice is a game client written from scratch in Rust that connects to
*Minecraft: Java Edition* multiplayer servers. It does not run, wrap, patch or
embed any Mojang code: it talks the network protocol, draws the world with its
own renderer and has its own interface. Its launcher starts Lattice's own game
windows, not the official game.

"Lattice" is a working name. It is referenced in `Cargo.toml` package names, the
window title and the config directory; renaming is a search-and-replace.

---

## 1. What it is, what it is not

| In scope | Out of scope (and why) |
|---|---|
| Joining Java Edition servers, online-mode (Microsoft account) and offline-mode | Its own world simulation. A local world runs Mojang's server program, downloaded on request; Lattice never reimplements the game rules. |
| One protocol version at a time, currently **26.3 (protocol 777)** | Supporting many versions at once. Servers that want older/newer clients already run ViaVersion. |
| Own renderer, own UI, own font, own entity models | Pixel-identical vanilla visuals. |
| Vanilla resource-pack *format* for block/item textures and models | Shipping Mojang art. Textures are fetched from Mojang's CDN on first run, per user, from the user's own purchase (§6). |
| Bedrock/Realms | Different protocol and different auth relying party. |
| Mods / Fabric / Forge | Those are Java-side mod loaders for the official client. |

## 2. Principles

1. **Nothing made up.** Packet IDs, block state IDs, collision boxes and physics
   constants come from Mojang's own server jar (data generator + an extractor,
   §5). Field layouts come from the community protocol documentation
   (minecraft.wiki, CC BY-SA) and are verified against a real server in tests.
   No guessed numbers in code.
2. **Every pixel does a job.** No news feed, no fake friend list, no "featured
   servers", no splash text, no placeholder stats. If a feature isn't wired up,
   its control is not on screen.
3. **Keep as little of the original as possible.** Own UI language, own font,
   own HUD, own entity models, own sky. Keep only what servers and resource
   packs depend on: protocol, block/item model format, text-component colors.
4. **One version, done properly.** Version-specific facts live in generated
   data files, so moving to the next release is: regenerate data, fix the
   packets that changed (the wiki's protocol history lists them), run the
   integration test.
5. **Fail with directions.** Every error tells the user what happened and what
   to do next, in plain words.

## 3. Architecture

### 3.1 Crates

```
crates/
  lattice-proto    wire format: VarInt, strings, NBT, text components,
                  framing, zlib, AES/CFB8, packet structs per connection state
  lattice-net      connection threads, login/configuration/play state machine,
                  server-list ping
  lattice-auth     Microsoft device-code → Xbox Live → XSTS → Minecraft,
                  profile, refresh, session join, token storage
  lattice-data     generated version data (blocks, shapes, registries) + lookups
  lattice-assets   version manifest, client-jar download + SHA-1 verify,
                  resource lookup (textures, models, blockstates, lang)
  lattice-world    chunk storage, palettes, light, entities, player physics
  lattice-render   wgpu: texture atlas, block model baking, chunk meshing,
                  sky/fog, selection outline, offscreen capture
  lattice          the application: the launcher and game windows (one binary)
tools/
  extractor/      Java program run at *development time* against the official
                  server jar to export block facts (§5)
  gen-data.sh     runs the data generator + extractor, writes crates/lattice-data/data/
  fetch-java.py   downloads Mojang's own Java runtime (dev tools only)
  test-server.sh  runs the official server locally in offline mode for testing
```

Dependency direction is one-way:
`lattice → {net, render, world, auth, assets} → {proto, data}`.
`lattice-render` never touches the network; `lattice-net` never touches the GPU.

### 3.2 Processes

```
lattice (launcher window)                 stays open, draws only on change
  ├─ lattice --session  (game window)     one process per session
  ├─ lattice --session  (game window)
  └─ …
```

* Each session is its own process, so a crash, hang or GPU reset in one can't
  touch the launcher or the others. Sessions outlive the launcher if it closes.
* The launcher owns accounts, the server list and the game-file download (so
  sessions never race to fetch it). A session is told who it is and where to
  go through one line of JSON on stdin, never on the command line, because
  command lines are visible to every process on the machine and it can contain
  an access token.
* After that, stdin carries commands (stop, reload settings), stdout carries a
  status line four times a second (phase, fps, frame times, ping, position),
  and stderr carries logs for the launcher's log view. See `ipc.rs`.
* `lattice --join ADDRESS [--name NAME]` opens one game window without a
  launcher.

Two sessions can't share a player on one server, because the server would
disconnect the first. The launcher sees this coming: with an offline name it
offers another session under the next free name (`Alex`, `Alex2`, …) and says
so before you press the button; with a Microsoft account it warns instead.

### 3.3 Threads (inside a game window)

```
             ┌──────────────┐  ClientboundPacket   ┌──────────────────────┐
  TCP ──────▶│ net: reader  │─────────────────────▶│                      │
             │ decrypt,     │  (decoded, chunks    │   main thread        │
             │ inflate,     │   pre-parsed)        │   winit event loop   │
             │ decode       │                      │   20 Hz game tick    │
             └──────────────┘                      │   + render per frame │
             ┌──────────────┐  ServerboundPacket   │   UI (egui)          │
  TCP ◀──────│ net: writer  │◀─────────────────────│                      │
             │ encode,      │                      └──────────┬───────────┘
             │ deflate,     │                                 │ dirty sections
             │ encrypt      │                                 ▼
             └──────────────┘                      ┌──────────────────────┐
                                                   │ mesh workers (rayon) │
  HTTPS (auth, assets, skins) on short-lived       │ section + neighbours │
  worker threads; results come back over channels  │ → vertex buffers     │
                                                   └──────────────────────┘
```

* No async runtime. One TCP connection, a handful of HTTPS calls: blocking I/O
  on dedicated threads is simpler and has no executor to tune.
* Keep-alive and ping are answered **on the reader thread**, so a long frame
  or a stalled GPU never gets the player timed out.
* The main thread owns the world. Workers receive immutable snapshots of a
  section plus its 26 neighbours (cheap `Arc` clones), never locks.

### 3.4 Game loop

Fixed 20 Hz simulation (the server's tick rate) with an accumulator; rendering
runs as fast as the display allows and interpolates the camera between the
last two ticks. Each tick: drain packets → physics → send movement → send
`client_tick_end`.

A window without focus draws at most 30 fps (a setting), and never waits on
VSync, so a covered window can't stall its own game loop or take GPU time from
the session being played.

## 4. Protocol

* States: handshake → status | login → configuration ⇄ play.
* Framing: VarInt length; after `login_compression`, a VarInt data-length
  prefix with zlib when above the threshold; after `key`, AES-128-CFB8 in both
  directions with the shared secret as key and IV.
* Packet IDs are **not** hard-coded by hand: they are generated from
  `packets.json` (Mojang data generator) into `lattice-proto/src/ids.rs`.
* Unknown or unimplemented clientbound packets are skipped by length, logged
  once per ID at debug level. Unexpected bytes left in a known packet are a
  bug and are logged at warn level with the packet name.
* Registries: the client answers `select_known_packs` with an **empty list**,
  so the server sends every synchronized registry in full (biomes, dimension
  types, damage types …). We therefore never need vanilla datapack files.
* Chat: 26.x servers default to `enforce-secure-profile=true`. Joining works
  without keys; sending chat needs signed messages, so the client fetches a
  profile key pair from `api.minecraftservices.com/player/certificates`, sends
  `chat_session_update`, and signs messages (SHA256withRSA). Offline-mode
  servers get unsigned chat. *(Signing is not built yet; until it is, the chat
  box tells the player the server needs signed chat and commands still work.)*

### 4.1 How protocol facts are verified

The community documentation is good but sometimes lags a release. Every
packet the client decodes is checked against a real 26.3 server: the headless
probe (`cargo run -p lattice-net --example probe`) joins, stays in, and warns
whenever a decoded packet leaves bytes unread. When something doesn't fit,
the server's own serializer is read with `javap` (26.x ships unobfuscated);
no game code is copied. Two places where the docs were behind 26.3, found
this way:

| What | Docs said | 26.3 actually | Where it matters |
|---|---|---|---|
| `BitSet` | VarInt count of longs | VarInt-prefixed **byte** array (`BitSet.toByteArray`) | light masks in chunk and light packets |
| `Player Action` status | 0 start, 1 cancel, 2 finish … | 0 start, **1 change direction**, 2 abort, 3 finish, 4 drop all, 5 drop, 6 release use, 7 swap, 8 stab | breaking blocks |

Both are fixed in `lattice-proto` with comments pointing at the evidence.

Join and teleport timing matters as much as layouts, and the docs only hint at
it. Read from the 26.3 server and confirmed by `tools/stress-join.sh`:

* A position packet sent before a `client_tick_end` has cleared the last one
  disconnects the client ("Invalid move player packet received"). So teleports
  are echoed with the next tick's movement, never immediately.
* The position inside `accept_teleportation` goes through the movement check.
  If it reaches the server before that player's first server tick, the check
  compares against (0, 0, 0) and reports "moved too quickly". So the client
  waits three ticks after arriving before it answers.
* `player_loaded` goes before the first simulated tick, and no movement goes
  out until it has.

A build that broke the first rule was kicked 9 times out of 9 by that test; the
fixed build had no kicks or corrections in 18 joins with 36 teleports.

## 5. Data pipeline (development time only)

`tools/gen-data.sh` needs a JDK and the official server jar. It:

1. Runs the server's built-in data generator (`--reports`) →
   `packets.json`, `blocks.json`, `registries.json`.
2. Compiles and runs `tools/extractor/Extract.java` against the server's
   classes, which bootstraps the registries and writes, per block state:
   collision boxes, outline boxes, friction, speed/jump factor, light emission,
   light dampening, `canOcclude`, fluid kind and level, offset type, render
   shape; and per entity type its hitbox size and eye height.
3. Writes compact JSON into `crates/lattice-data/data/`, which is committed.

End users never need Java. Contributors only need it when bumping versions.
The output is interoperability data (IDs and box sizes), the same kind of data
that PrismarineJS/minecraft-data has published for years.

## 6. Assets

On first launch (and after a version bump) the client:

1. Fetches `version_manifest_v2.json` from `piston-meta.mojang.com`, finds the
   pinned version, fetches its version JSON.
2. Downloads `client.jar` (≈41 MB) from `piston-data.mojang.com`, verifies the
   SHA-1 from the version JSON, stores it in the cache directory.
3. Reads only `assets/minecraft/{blockstates,models,textures,items,lang}/` out
   of the jar. No class file is ever read or executed.
4. Sounds (later): the asset index + object store, also SHA-1 verified.

User resource packs are layered on top with the vanilla precedence rules.

Download progress is a real progress bar with bytes and speed; a failed or
corrupt download is retried and then reported with the URL that failed.

## 7. Accounts and auth

### 7.1 Flow

```
device code ─▶ user enters code at microsoft.com/link
   │ poll token endpoint (interval from response, honour slow_down)
   ▼
MSA access + refresh token
   ▼ user.auth.xboxlive.com/user/authenticate      (RpsTicket "d=<token>")
XBL token + user hash
   ▼ xsts.auth.xboxlive.com/xsts/authorize         (rp://api.minecraftservices.com/)
XSTS token          ── 401 XErr → mapped to a sentence (no Xbox profile,
   ▼                    child account, region, banned)
api.minecraftservices.com/authentication/login_with_xbox
   │ ── 403 → "this app's client ID is not approved by Mojang yet"
   ▼
Minecraft access token (24 h)
   ▼ /entitlements/mcstore, /minecraft/profile   (404 → no Java profile yet)
UUID, name, skin
```

### 7.2 The client ID requirement (read this)

Microsoft sign-in needs an Azure application ID. Mojang only lets **approved**
app IDs call `login_with_xbox`; unapproved IDs get HTTP 403. To use your own
Microsoft account:

1. Register an app at <https://portal.azure.com> → App registrations →
   *Personal Microsoft accounts only*. Under Authentication, enable
   *Allow public client flows* (device code needs it). No secret.
2. Request Minecraft API access for that app ID: <https://aka.ms/mce-reviewappid>.
3. Put the ID in `config.toml` (`[auth] client_id = "…"`) or the
   `LATTICE_CLIENT_ID` environment variable.

Until approval arrives, offline-mode servers (including a local test server)
work without signing in. The client will **not** ship another launcher's app
ID: that impersonates someone else's application and breaks their terms.

### 7.3 Storage

* Refresh tokens are stored in `accounts.json` inside the config directory,
  file mode `0600` on Unix. Minecraft access tokens are kept in memory only
  and refreshed on demand (expiry − 5 min).
* Planned: OS keychain (Secret Service / Keychain / Credential Manager) with
  the file as fallback.
* Several accounts, one active. Sign-out deletes the stored refresh token.

### 7.4 Joining online-mode servers

On `hello` (encryption request): generate a 16-byte secret, compute the
server hash (SHA-1 of server ID + secret + server public key, Minecraft's
signed hex), `POST sessionserver.mojang.com/session/minecraft/join`, then send
the RSA-encrypted secret and verify token. Failures map to sentences
("Your session expired — sign in again", "This server is in online mode and
you are using an offline profile").

## 8. World

* Column = 16×16 × world height (from the dimension type registry entry,
  e.g. −64..320 = 24 sections).
* Section block storage: single-value sections stay a single `u16`; mixed
  sections are decoded to a flat `[u16; 4096]` (8 KiB). Biomes: 64 entries.
  Light: sky + block nibble arrays, including the two border sections.
* Block updates (`block_update`, `section_blocks_update`) mark the section and
  the touching neighbours dirty for remeshing.
* Entities: map from entity ID → kind, position, rotation, interpolation
  target, metadata we use (pose, name, equipment later).

## 9. Rendering

* wgpu (Vulkan/Metal/DX12), one render pass for the world + one for UI.
* **Block models**: vanilla JSON format (blockstates `variants`/`multipart`,
  model inheritance, elements, face UVs, rotations, `cullface`, `tintindex`).
  Baked once per block state into quads in a shared texture atlas.
* **Meshing**: per 16³ section on workers; face culled against neighbours'
  `canOcclude` + full-face check; smooth lighting and AO from light data on
  full faces, flat light elsewhere; biome tint (3×3 biome-cell blend) for
  grass/foliage/water — which blocks take which tint is the table from the
  game client's `BlockColors`, read from bytecode; opaque, cutout and
  translucent buckets chosen from the texture's own alpha; translucent
  sections drawn back to front.
* Terrain uses a less-or-equal depth test with an invariant position, because
  models layer coplanar quads. Grass block sides are the clearest case: a base
  texture with a fixed green strip, then a tinted overlay exactly on top. With
  a strict test the overlay never draws and the strip keeps its fixed colour.
* The surface is a plain 8-bit format (egui expects that); world shaders do
  their lighting in linear space and encode gamma on output.
* **Sky**: own design — a gradient from the biome's sky colour to the fog
  colour, sun/moon as plain discs, no vanilla sky textures.
* Fog at render distance; frustum culling per section.
* **Offscreen capture**: the renderer can draw a frame to a texture and write
  a PNG. Used by the screenshot key and by automated visual checks.
* Entities: own low-poly models (box-based, sized from the extracted hitboxes)
  with player skins applied using the standard 64×64 skin layout.

## 10. Movement and physics

Vanilla servers are authoritative but trust client movement within limits;
diverging gets you teleported back ("moved wrongly") or kicked. The physics
therefore reproduces vanilla behaviour using extracted per-block values
(friction, speed/jump factors, collision boxes) and these documented constants:
gravity 0.08 blocks/tick², vertical drag 0.98, horizontal drag
`0.91 × friction` on ground, base walk speed 0.1, sprint ×1.3, jump velocity
0.42, step height 0.6, player box 0.6 × 1.8, eye height 1.62.

Movement packets: position/rotation each tick when changed, otherwise a
status-only packet at least once per second; `accept_teleportation` on every
`player_position`; `player_input` when keys change; `client_tick_end` every
tick; `player_loaded` once the spawn area is meshed.

## 11. Interface

### 11.1 Point of view

The product's job is "get me back to where I was, and let me see how it's running".
There is no title screen: **the launcher's first page is Play**, with the place you
last played filling the top of it, then your worlds and servers. It's an ordinary
desktop app: pages down the side, who you're playing as at the bottom.

The one place the design spends boldness: when you leave a world or server, the game
window saves a frame of where you were standing, and that's the picture on the Play page.
Nothing is invented. Before there's a frame, Lattice shows its own generated drawing.

### 11.2 Tokens

Colour comes from the thing the name refers to: a lattice is the 3D grid a block
world is made of, so the interface is a technical drawing of one. Surfaces are
cyanotype blue, or drafting paper in the light theme. Amber (burnt orange on
paper) is kept for what you can press, so the main action on a screen is never in
doubt. Every pair below is held to WCAG AA by a unit test in `ui/theme.rs`: 4.5:1
for text, 3:1 for control edges and focus rings.

| Token | Cyanotype (dark) | Drafting (light) | High contrast | Use |
|---|---|---|---|---|
| ground | `#0E2740` | `#E4EBF1` | `#000000` | page background |
| surface | `#14324F` | `#F6F9FB` | `#000000` | sidebar, rows |
| raised | `#1D4366` | `#D5E0E9` | `#1A1A1A` | controls |
| hover | `#26527B` | `#C5D4E0` | `#333333` | controls under the pointer |
| edge | `#7BA3C7` | `#5F7C96` | `#FFFFFF` | control outlines, slider rails |
| ink | `#F4F7F5` | `#0C2740` | `#FFFFFF` | text |
| ink muted | `#BBCDDC` | `#3B546B` | `#E6E6E6` | secondary text |
| accent | `#8AD4F5` | `#0F5C94` | `#66E0FF` | the drawing's lines, on-switches, links |
| action | `#FFC247` | `#B34700` | `#FFD400` | the one main action |
| warn | `#FFC247` | `#7F4400` | `#FFD400` | "outdated server" |
| danger | `#FFA595` | `#A8231B` | `#FF9E94` | errors, deleting |

The theme is a setting: System, Dark, Light or High contrast. A game window stays
dark over the world, unless high contrast is chosen. The HUD's own colours (health,
food, armour, air, experience) carry meaning and don't change with the theme.

Type: **Atkinson Hyperlegible Next** (OFL, Braille Institute), chosen because it
stays readable over a moving 3D scene and its letterforms are hard to confuse.
One family, two weights. Scale (logical px): 14 / 16 / 20 / 28 / 44. Nothing is
smaller than 14. Sentence case everywhere.

Shape: 6 px radius on controls, 0 on rows and panels (radius means "you can press
this"). No shadows; depth comes from the surface steps. Hit targets are at least
36 px tall.

Focus and access: every control is reachable with Tab, pressed with Enter or Space,
draws the same 2 px focus ring and tells screen readers what it is (AccessKit). State
is always in words or shape as well as colour: switches move their knob, server
status is written out, HUD bars have numbers. "Reduce motion" snaps animations and
stops anything in the menus moving by itself.

Motion: only in answer to an action.

The memorable thing: the big picture on the Play page, the last frame you saw in the
place you last played, with one amber button that takes you back. With no frame yet,
it's Lattice's own drawing of a block grid, generated from the place's name
(`widgets::lattice_field`), which also stands behind the joining screens.

### 11.3 Screens

```
Launcher
+-----------+------------------------------------------------+
| (mark)    | [last frame seen, full width]                  |
| Lattice   |                                                |
|           |  Test place                          [ Play ]  |
| > Play    |  Survival, played yesterday                    |
|   Sessions+------------------------------------------------+
|   Accounts|  Alex on Test place   In game   143 fps [Stop] |
|   Settings|                                                |
|           |  Your worlds                    [New world]    |
|           |  [picture] My world   Creative  [Play][Delete] |
|           |                                                |
|           |  Servers                        [Add server]   |
|           |  [ Join by address ...       ] [Join]          |
| Playing as|  [icon] My SMP  A vanilla server               |
| [face][v] |         3 of 20 players, 12 ms [Play][Edit][X] |
+-----------+------------------------------------------------+
```
* Pages: Play, Sessions, Accounts, Settings (Ctrl+1 to Ctrl+4). Forms open within
  Play, and Esc leaves them. Settings save when you leave the page.
* Enter on the Play page plays the place in the picture, unless a control has focus.
* Rows show only data the server sent: favicon, MOTD, version name, players,
  measured round-trip time. A version mismatch is written out ("Runs 1.21.11. Lattice
  plays 26.3, so it can't join."), not shown as a red X.
* Session rows show only what the session measured and reported. Memory comes
  from `/proc` and is left out on systems without it. A running session also gets one
  line on the Play page, so it's never out of sight.
* A session that closes normally leaves no row. One that was kicked keeps its
  window and row, with the server's reason, until closed. One that crashed
  keeps its row and log, with Restart and Dismiss.
* The launcher redraws only when something changes (at most a few times a
  second while sessions report), and prefers the low-power GPU.

```
Sign in with Microsoft
  1  Open microsoft.com/link on any device      [ Open in browser ]
  2  Enter the code     K7Q-M2PX                [ Copy code ]
     The code expires in 14:32.
  Waiting for approval…
```
After approval the same panel shows each real step as it completes:
Microsoft → Xbox Live → Minecraft services → profile.

```
Game window while joining            When the connection ends
  Joining My SMP                       Disconnected by My SMP
  Logging in…                          "You are banned: …" (server's text)
  [ Cancel ]                           [ Rejoin ] [ Close window ]
```

In game:
* HUD: hotbar at bottom centre (own frame, item models from the resource
  pack), health / food / armour as **labelled bars with numbers**, XP level as a
  number. Crosshair is a 2 px plus with a dark outline.
* Chat bottom-left, server colours preserved, lines fade after 10 s, `T`/`/`
  opens input with command suggestions from the server's command tree.
* Live frame rate in the top right, on by default: fps, frame time, worst frame
  in the last quarter second, and why it's capped ("VSync", "background limit").
* `Esc`: Back to game, Settings (over the running game, so changes show live),
  Disconnect and close.
* `F3`: debug overlay with coordinates, chunk, facing, biome, light, frame
  times, ping, loaded sections, mesh queue: real counters only.

### 11.4 Copy rules

Verbs on buttons, sentence case, no exclamation marks, no apologies.
An action keeps its name through the flow ("Join" → "Joining My SMP…").
Errors state cause and fix: *"Couldn't reach play.example.net:25565. The
server is offline or the address is wrong."*

### 11.5 Game windows

The launcher is a desktop app; a game window is part of a game. Its menus don't share
the launcher's flat panels (ui/chrome.rs, used whenever `theme::game_look()` is on):

- **One pixel grid.** `chrome::unit()` is a whole number of real pixels, worked out from the
  window: the largest that fits the tallest menu. A slot is 18 units, its picture 16, so flat item
  pictures land on whole pixels at any size. Everything in a menu is placed in these units.
- **Panels are slabs of pale limestone**: flecked (a small repeating picture multiplied over the
  face), with an outline, cut corners, a lit top and left edge and a shaded bottom and right.
  Words on them are dark: palette `LIMESTONE`, entered with `theme::on_panel`, and held to the same
  contrast test as the rest. HUD text over the world stays on `STONE`, light with a hard shadow.
- **Slots are cut into the panel**, dark so what's in them stands out. What comes out of a menu
  gets a larger, framed slot. Empty armour, fuel and similar slots show a faint outline of what
  goes there; the name appears on hover, and every slot tells a screen reader what's in it.
- **Keys sink when pressed**, and a chosen option stays pushed in, filled with honey. The one main
  action on a screen is copper.
- **A menu gets the screen to itself**: the world dims, and chat, hotbar and bars go. The pause
  and death menus have no box, only a title and big keys standing on the dimmed world.
- **Type is Jersey 15 at whole multiples of 27 points**, the only sizes where its pixels land
  on the screen's. `theme::font` snaps to them, so anything else would blur. "Pixel font in
  game" in settings switches back to the launcher's plainer face.
- **You stand in your own inventory**, drawn from your skin as boxes (ui/doll.rs), wearing your
  armour and turning to follow the pointer.
- **Clicks behave as the game's do**: pressing picks up, letting go puts down, a drag spreads with
  a preview. That's in `draw_menu` in ui/inventory.rs.

All of this is drawn here, not taken from Mojang's interface art.

Screens the client opens itself, because the server only sends what's in a block: command blocks
and the other game-master blocks live in game/gm_blocks.rs (state, reading from the block, packets,
unit-testable) and ui/block_screens.rs (forms).

## 12. Settings and files

| Platform | Config | Cache (client jar, skins, frames) | Logs |
|---|---|---|---|
| Linux | `~/.config/lattice/` | `~/.cache/lattice/` | `~/.local/share/lattice/logs/` |
| Windows | `%APPDATA%\lattice\config` | `%LOCALAPPDATA%\lattice\cache` | `%APPDATA%\lattice\data\logs` |
| macOS | `~/Library/Application Support/dev.lattice.lattice/` | `~/Library/Caches/dev.lattice.lattice/` | same as config |

`config.toml` (human-editable, written atomically), `servers.json`,
`accounts.json` (0600). Unknown keys are preserved on save. Both the launcher
and game windows can change settings: a window's change reaches the launcher
through the file's modification time, and the launcher tells running sessions
to reload when you leave its Settings screen.

Paths follow the XDG variables on Linux, so an app started from a sandbox such
as Flatpak VS Code stores its files under that sandbox's folders instead.

Logs: `logs/launcher.log` and `logs/session-<name>.log`, each keeping one
`.previous.log`.

## 13. Errors, logs, tests

* Libraries return typed errors (`thiserror`); the app turns them into the
  sentences above. Logs go to stderr and a rotating file; tokens and shared
  secrets are never logged.
* **Unit tests** (`cargo test --workspace`): VarInt/VarLong vectors, NBT and
  modified-UTF-8 round-trips, text components (translations, legacy `§`
  codes, NBT lists), framing with compression and encryption, CFB8 against
  NIST SP 800-38A, MD5 against RFC 1321, Minecraft SHA-1 digests ("Notch",
  "jeb_", "simon"), paletted containers, block-state ID layout for all 35 723
  states, `Mth.getSeed` and plant offsets against values produced by the real
  game, walking speed (0.2158 blocks/tick) and jump apex (1.2522), step-up,
  wall collision, ray–box faces, config key preservation.
* **Server test**: `tools/test-server.sh` runs the official server locally in
  offline mode. `cargo run -p lattice-net --example probe -- localhost:25566`
  joins headless and reports every packet kind received and any decode
  problem.
* **Scripted play test**: development-only environment variables drive the
  real client so a run can be checked without a person at the keyboard.
  Game window: `lattice --join host:port --name X` with
  `LATTICE_SCRIPT="1:w+,1:sprint+,5:jump+,7:yaw=90,10:chat=hello,12:attack+"`,
  `LATTICE_SHOTS="8:a.png,12:b.png"` (frame captures), `LATTICE_EXIT_AFTER`.
  The server log then shows whether any move was corrected ("moved wrongly")
  and that chat arrived.
* **Join stress test**: `tools/stress-join.sh` joins several sessions at once on
  a server that was just paused, while they sprint and get teleported, then
  counts kicks and corrections in the server log.
* **Launcher test**: `LATTICE_AUTOLAUNCH="host:port,host:port"` starts sessions
  when the game files are ready; `LATTICE_SHOTS`, `LATTICE_EXIT_AFTER` and
  `LATTICE_SCREEN=accounts|settings` apply to the launcher, and
  `LATTICE_CHILD_SHOTS` / `_SCRIPT` / `_EXIT_AFTER` are passed to its sessions
  (`{n}` becomes the session number). `LATTICE_HOME` isolates config and cache.

## 14. Singleplayer — options for later

1. Not at all (current decision).
2. "Play locally" button that downloads and runs the official server jar with
   Mojang's Java runtime, then connects to it. Works, but brings back the
   original's code (on the user's machine, not in this repo).
3. Write a server. A separate, larger project.

## 15. Legal

* No Mojang code or assets in this repository. Textures/models are fetched
  per user from Mojang's CDN and stay on that machine.
* Not affiliated with or endorsed by Mojang or Microsoft; "Minecraft" appears
  only to say what the client is compatible with, per Mojang's usage
  guidelines.
* The brand string sent to servers is `lattice`. Servers may refuse unknown
  clients; that is their call.
* Protocol documentation used under CC BY-SA 3.0 (minecraft.wiki).
