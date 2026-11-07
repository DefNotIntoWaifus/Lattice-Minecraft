# Lattice — roadmap

Status as of 2026-11-07. "Verified" means it was exercised against the official
26.3 server (offline mode, local) or a live Mojang/Microsoft endpoint, not just
compiled. Items are ticked as they finish, so this file is also the progress log.

## Where it stands

Lattice is playable: survival and creative on a server or a world on this computer, with mobs,
items, menus, riding, sound, particles and weather. Everything below marked done has a
scenario in `tools/e2e/` that plays it on the real 26.3 server, or was compared picture by
picture with the official client.

What's left is listed at the end of the work order. What the first play test reported, and
what became of it:

| Reported 2026-11-03 | Now |
|---|---|
| Can't use chests, crafting tables, villagers | Every menu works, with a recipe book |
| Mobs have no body or texture, can't be attacked | Every mob is drawn from the game's own shapes and animations, and takes hits |
| Water physics wrong | Ported from the game: currents, swimming, swim pose, shallow-water jump |
| Don't know the version | Launcher, server rows, session rows, pause menu and F3 |
| Can't open the inventory; `/give` seems broken | Inventory and creative work; `/give` always worked, the item just couldn't be shown |
| Too few settings | About 80, organised and searchable, plus every key binding |

## Decisions

1. **Nothing of Mojang's goes in the project.** Mob shapes, textures, sounds and music are
   read at run time from the user's own copy of the game, the way block textures already
   are, and cached on their machine. Official textures only line up with official
   geometry, so Lattice reads that geometry out of the downloaded jar on first start
   instead of storing it. The project holds only functional facts needed to talk to
   servers (packet IDs, block states, collision shapes).
2. **Menus keep Lattice's own design**, not the vanilla GUI textures, per the original
   brief. They should still read at once as a game's menus, not a web page's.
3. **The official client is a test reference only**, on this machine, for comparison. It
   is never bundled or shipped.

## Working rules

- The user's server on port 25566 and their world stay untouched. Tests use a separate
  server on 25567 and an isolated `LATTICE_HOME`, and leave nothing in the user's cache.
- Everything compiles and `cargo test --workspace` passes at the end of each item.
- Releases and moving to a new Minecraft version are described in `docs/RELEASING.md`.
- Every option, button and screen must do something real. Nothing is drawn as a
  placeholder.
- Game facts come from the 26.3 jars (bytecode via `javap`, generated reports). The wiki
  is a starting point and has already been wrong twice.
- Bugs found along the way get fixed, and listed here if they can't be fixed right away.

## How everything gets tested

Each phase's "done when" is checked with this rig, not by reading the code.

- **Headless game**: off-screen rendering with scripted input through the same code paths
  as real input. Needs no window, no focus and no screen.
- **Scenario runner**: starts a fresh server, drives its console and the clients, checks
  results from the server's side (`/data get`, `/execute if items`, log lines) and from
  the client's reported state, and saves captures for review.
- **Packet ledger**: every packet is marked handled, or ignored with a reason. A gap fails
  a test. A decode error or leftover bytes fails any run.
- **Official client as reference**: movement traces, same-view screenshots, and packet
  recordings through a local proxy. If it can't run here, the fallback is bytecode plus
  worked-out expected values.
- **Soak bots**: wander, dig, build and open menus across biomes and dimensions. Any kick,
  correction, `standing on air`, panic, decode error or memory growth is a bug.
- **Decoder fuzzing**: mutated real packets must never panic.
- **Capture review**: each screen and scene is captured at several sizes and GUI scales,
  and the images are read before anything counts as done.

## Work order

Ordered by what blocks real play, so stopping at any point leaves the most useful parts
done.

### Phase 0 — Test rig and the log findings

- [x] Headless game mode
- [x] Scenario runner
- [x] Strict decode failures: a packet with bytes left over is an error in the test logs
- [x] Official client reference (time-boxed; fall back if it can't run here)
- [x] Soak bot
- [x] Decoder fuzzing
- [x] Find and fix `standing on air`
- [x] Chat and command keys follow the typed character on any keyboard layout

*Done when* one command runs the whole suite without a window and reports pass or fail.

### Phase 1 — Moving and fighting

- [x] Minecraft version visible: launcher, each server row (with the server's own version
      and a clear message on a mismatch), session row, pause menu, F3
- [x] Water and lava from the 26.3 code: currents, swimming up and down, sprint-swimming
      with the swim pose and low hit box, shallow-water jump, step-up, climbing out at
      edges, Dolphin's Grace, Depth Strider, bubble columns
- [x] Other movement: ladders, vines, scaffolding, cobweb, powder snow, berry bushes, soul
      sand, honey, slime, ice, sneak edge guard, crawling, effects (Speed, Slowness, Jump
      Boost, Slow Falling, Levitation), attributes, double-tap flight and sprint, auto-jump,
      elytra
- [x] Server-set rotation reaches the camera (teleports, rejoining)
- [x] Entity targeting with real hit boxes and reach, attack with cooldown, `interact`
- [x] Respawn and dimension changes

*Done when* movement traces match the official client to 0.001 blocks per tick in each of
those scenes, a summoned zombie loses health when attacked (confirmed by the server), and
a villager interaction reaches the server.

### Phase 2 — Settings that are complete and organised

- [x] Settings screen: sections, search, reset per setting and per section, live effect.
      The same screen in the launcher and over the game
- [x] Key binding editor: every action, mouse buttons, conflict warnings, reset
- [x] Everything that can be wired today. Later phases add theirs as the feature lands

The checklist is the official option list; each entry needs a real effect:

- **Video**: render and simulation distance, frame limit, background limit, VSync, display
  mode and resolution, GUI scale, brightness, FOV and FOV effects, view bobbing, screen
  effects, damage tilt, darkness pulse, vignette, lightning flash, glint, clouds and their
  distance, particles, smooth lighting, AO, biome blend, chunk fade, cut-out leaves,
  transparency, entity distance, shadows, anisotropy, mipmaps, texture filtering, chunk
  update priority, weather radius
- **Controls**: all key bindings, sensitivity, invert X and Y, raw input, wheel sensitivity,
  toggle or hold for sprint and sneak, sprint window, auto-jump, attack indicator
- **Audio**: master and all 10 channels, output device, directional audio, subtitles, music
  frequency
- **Chat**: visibility, colours, links and link prompt, text size, opacity, background,
  line spacing, width, height, delay, command suggestions, secure-only, name hiding
- **Player**: skin layers, main hand, language, server listing
- **Accessibility and interface**: text contrast, font options, notification time, menu
  background, debug scale

*Done when* each listed option exists and demonstrably changes something, or is listed with
the reason it doesn't apply (final audit in Phase 10).

### Phase 3 — Items and inventory

- [x] Item stack decoding: every 26.3 data component including nested ones, raw bytes kept
      so stacks can be re-encoded exactly
- [x] Item defaults from the reports: stack size, durability, food, tool, armour
- [x] Inventory and menu state: `container_set_content`, `container_set_slot`,
      `container_set_data`, `set_cursor_item`, `set_player_inventory`, `set_held_slot`,
      `open_screen`, `container_close`
- [x] Item drawing: `items/*.json` with every selector and tint type, flat and block-shaped
      models, baked into icons
- [x] Hotbar with real items, offhand, counts, durability, name pop-up, full tooltips
- [x] Inventory screen (E), cursor item, all click types (pick, place, split, shift-click,
      double-click, drag, number keys, drop, offhand) with local prediction
- [x] Creative inventory: tabs and contents extracted from the game, search, saved toolbars,
      `set_creative_mode_slot`

*Done when* every item in the registry, given with `/give`, decodes with no leftover bytes
and has an icon (grid captures reviewed), and moving stacks round-trips with the server,
confirmed by `/execute if items` and by comparing packets with the official client.

### Phase 4 — Using the world

- [x] Every menu: chests, shulker, ender chest, hopper, dispenser, crafting table, furnaces,
      brewing, enchanting, anvil, grindstone, smithing, stonecutter, loom, cartography,
      beacon, lectern, villager trading, horse
- [x] Recipe book
- [x] Signs, books (reading and writing), maps
- [x] Using items: eating, drinking, bows, crossbows, tridents, shields, spyglass, cooldowns,
      slowdown while using
- [x] Placement with prediction and acknowledgements, doors and buttons, sneak-use, both hands
- [x] Mining speed from tool, enchantments, effects, water and air; crack overlay; creative
      instant-break
- [x] Pick block, drop, swap hands, beds, riding boats, minecarts and horses, spectator actions

*Done when* one scripted playthrough completes on a real server, each step confirmed from
the server's side: punch a tree, craft planks, a table and a pickaxe, mine stone, smelt iron,
trade with a villager, enchant, rename, write a sign, sleep.

### Phase 5 — Bodies: mobs, players, block entities

- [x] Entity data: `set_entity_data` with all serializers and per-type fields, equipment,
      passengers, effects, `animate`, `hurt_animation`, `damage_event`, `entity_event`
- [x] Geometry reader that runs on first start against the user's jar, plus a table from
      each entity type to its model, texture and registry-driven variants (pig, cow, wolf,
      cat and so on)
- [x] Renderer for part-based models: animation, hurt flash, death, shadows, name tags
- [x] Non-mob entities: dropped items, XP, projectiles, falling blocks, TNT, boats, minecarts,
      armour stands, paintings, item frames, display entities, lightning
- [x] Players: default and downloaded skins, layers, capes, armour with trims, elytra, held items,
      third-person camera, first-person hand and held item
- [x] Block entities: chests with lids, signs with text, beds, banners, heads, shulker boxes,
      lectern and enchanting books, bell, conduit, beacon beam, decorated pot, campfire items,
      moving pistons, spawner mob, end portal

*Done when* "zoo" scenes with every entity type and block entity pass review against official
client captures.

### Phase 6 — The world

- [x] Animated textures
- [x] Fluid surfaces: real heights, flow direction, sides
- [x] Translucency sorting
- [x] Particles: all server-sent types, client-only ambience (torches, fire), block dust, splashes
- [x] Weather, clouds, sky per dimension, lightning, world border wall and collision
- [x] Screen overlays: underwater, fire, pumpkin, powder snow, portal, spyglass, vignette; server
      post effects

*Done when* about 20 fixed scenes (forest, ocean, village, desert, jungle, ice, Nether, End,
night, rain) pass review against the official client from the same viewpoint.

### Phase 7 — Sound

- [x] Asset index and on-demand download, SHA-1 checked
- [x] Engine: Ogg decoding, mixer, positional audio, channels, device choice
- [x] `sound`, `sound_entity`, `stop_sound`; steps, landing, block and UI sounds; ambience;
      music, records, underwater

*Done when* every sound event in a scripted run resolves to a file and appears in an offline
capture of the mix. The final judgement by ear is the user's.

### Phase 8 — Everything else a server can send

- [x] Boss bars, scoreboard, teams, tab list, effect icons, cooldowns. Player heads in the tab
      list and the locator bar aren't there yet
- [x] Advancements and toasts, statistics
- [x] Chat: history, scrolling, click and hover actions, command tree with tab completion and
      argument hints
- [x] Dialogs without typed inputs, cookies, transfer, tick rate and freezing. Server links, typed
      inputs in dialogs and resource packs aren't there yet

*Done when* the packet ledger has no unhandled entries.

### Phase 9 — Online servers

- [x] Signed chat and commands, from the game's code, with the signature checked in unit tests
      against a local key. Not yet run against a real online server: that needs an approved app ID
- [x] Certificates, token refresh, skins from signed profiles (untested end to end, as above)
- [ ] End-to-end Microsoft sign-in (**needs the user's Mojang-approved app ID**)

### Phase 10 — Hardening

- [ ] Hour-long soak with several bots across dimensions: no kicks, stable memory (three bots for 12 minutes and two for 20 have passed clean; `soak.py [minutes] [seed] [bots]`)
- [ ] Performance budgets: high render distance, crowds of entities, many sessions
- [x] Hosting a local world from the launcher with the official server, without a terminal
- [ ] Windows and macOS builds checked as far as possible from here
- [x] Settings audit against the official list; review of every screen at several sizes. Not
      offered, because they don't apply here: narrator, Realms notifications, telemetry,
      splash texts, menu panorama and blur, chunk fade-in, exclusive fullscreen, Japanese glyphs
- [x] Docs brought up to date

## Interface redesign and known-bug pass

- [x] Renamed to Lattice; existing settings, sign-ins and worlds move over on first start
- [x] Runtime themes (dark, light, high contrast, following the desktop), contrast tested in `theme.rs`
- [x] Launcher: sidebar pages, Play page with the last-frame picture, Sessions as its own page, new forms
- [x] Settings: controls in a fixed column, switches, segmented choices, per-row reset, confirmation for reset all
- [x] Accessibility: AccessKit in both windows, keyboard focus on every control, reduce motion, 14 px minimum text
- [x] HUD: cell bars with numbers, armour and air, F1 hides it; centred pause and death menus
- [x] Server and user resource packs, with live reload (`tools/e2e/packs.py`)
- [x] Player heads in the tab list, locator bar, server links, typed dialog inputs, page-turning words in books
- [x] End portal and gateway star field, energy on charged creepers and withers, placed copper golem statues in each pose
- [x] Trim colours on item icons, banner patterns on banner and shield icons and when held, cape-coloured elytra, fishing lines
- [x] Stray dashes on block seams when the camera lined up with the grid (texture coordinates now sit a hair inside each tile)
- [x] Boss darkening and fog, the darkness effect's pulse, the tint for reduced screen distortion

## Menus and interaction pass (2026-11-07)

- [x] Every menu rebuilt on one pixel grid, in the limestone look (DESIGN 11.5)
- [x] Trading: scrolling, real prices, sold-out marks, preview of the chosen trade
- [x] Clicks as in the game: press to pick up, release to put down, drag with a preview, shift-double-click
- [x] Command blocks and minecarts with completion, structure, jigsaw, test and test instance blocks
- [x] Game rules, difficulty, game mode switcher, more F3 shortcuts, spectators following mobs and teleporting
- [x] Crafter slots and its inventory being one slot out, bundles, saved hotbars, beacon confirmation
- [x] Leaving a bed, the End's closing story (you'd get stuck without it), totems, compasses
- [x] Releases: versions, changelog, packaging, an update check and installer in the launcher, a daily
      check for new Minecraft versions, `tools/new-version.py` to move to one

## Still to do

Known gaps, none of which stop normal play:

- Dialogs a server opens during configuration, before the world loads, show a notice
  instead (dialogs during play are complete, typed inputs included)
- A seat's small height shift while a horse rears or a camel changes pose
- The character in the inventory doesn't hold what's in their hand yet
- The elder guardian's curse plays its sound, without the ghostly face on screen
- Chat messages a server takes back by short number rather than by full signature stay on screen
- The smithing table has no armour stand to preview on, and the cartography table no map preview:
  what they'd make only shows in the result slot
- A second launch can only flag the open launcher on the taskbar: Wayland doesn't let a window bring itself forward
- Windows and macOS builds have never been run; everything's been tested on Linux with an RTX 2060.
  GitHub builds them for each release, which shows they compile and nothing more
- Screen reader labels are checked by reading the accessibility tree, not yet with a
  person using Orca or NVDA

## Coming with Minecraft 26.4

A dry run of `tools/new-version.py` on 26.4-snapshot-7 (2026-11-03) shows what's ahead. All of it
waits until 26.4 is released, as snapshots still change:

- `tools/extractor/Extract.java` stops compiling: block sounds move into a registry (`SoundType`
  becomes `BlockSoundSet`), and `Fluids.FLOWING_WATER` and `FLOWING_LAVA` go away
- Login packets are renamed and joined by `apply_options`, `options_applied` and `key`
- A new jukebox menu, new `frostbite` and `ice_ball` entities, a `freezing` effect and particle
- Command argument types lose their `brigadier:` prefix, which the command tree reader looks for

## What only the user can do

- Get an app ID approved by Mojang (README, "Sign in with your Microsoft account"). Without it,
  Lattice can't join online-mode servers.
- Judge sound and feel: audio by ear, mouse and movement by hand.

## Done so far

### Milestone 0 — Groundwork

| Item | State |
|---|---|
| Data pipeline from the official server: packet IDs, 35 723 block states, 933 collision/outline shapes, physics factors, light, fluids, entity sizes, player attributes, registries | Done; ID layout and two game functions checked against the real code |
| Wire format: VarInt/Long, strings, UUIDs, positions, NBT (modified UTF-8), text components (NBT + JSON + `§` codes), LpVec3, bit sets | Done, unit-tested |
| Framing, zlib compression, AES/CFB8, RSA key exchange, session-server hash | Done, unit-tested against published vectors |
| Server list ping with SRV lookup, MOTD in server colours, favicon, latency, version mismatch | Verified |
| Microsoft sign-in chain (device code → Xbox → XSTS → Minecraft → ownership → profile), refresh, session join | Written; the live Microsoft endpoint was reached and its errors map correctly. The full chain needs a Mojang-approved app ID to verify (see DESIGN §7.2) |
| Game resources: client jar from Mojang's CDN, SHA-1 verified, cached | Verified |

### Milestone 1 — Join and see

| Item | State |
|---|---|
| Login, configuration (known packs, registries, tags, code of conduct prompt, cookies, resource-pack decline), play | Verified |
| Chunk and light decoding, block updates, chunk unloading | Verified (213 columns, thousands of packets, no leftover bytes) |
| Block models: variants, multipart with AND/OR, parents, element and variant rotation, UV lock, shade overrides, random plant offsets, coplanar overlays | Verified visually |
| Meshing on worker threads: culling, smooth light + AO, biome tints from the client's tint table, cut-out and translucent layers, flat water and lava surfaces | Verified visually |
| Renderer: mip-mapped atlas, frustum culling, own sky with sun/moon and day–night from the server clock, fog, underwater fog, block outline | Verified visually, ~180 fps on an RTX 2060 |
| Movement on land: walking, sprinting, jumping, step-up, collisions, friction/speed/jump factors, creative flight, spectator | Verified: no kicks or corrections in 18 stress-test joins with 36 teleports while moving. Water, climbing and special blocks are approximate until Phase 1 |
| Breaking (empty-hand timing) and using/placing (server decides) | Breaking verified; placing needs inventory to test meaningfully |
| Chat in/out with server chat types and the game's translations; commands | Verified for unsigned chat |
| HUD: crosshair, held slot, health/food/XP bars, action bar, titles, tab list, F3 details, player name tags, toasts | Verified visually |
| Launcher window that stays open: server list with live status, accounts, offline names, settings | Verified visually; buttons driven by script, not yet clicked by a person in testing |
| Several sessions at once, each its own process, with automatic names for a second session on a server | Verified with two sessions on one server |
| Live numbers per session: phase, fps, frame time, ping, memory, uptime, position; log view | Verified |
| Stop, reload settings, kick, crash handling | Verified: stop and reload over the real pipe, a server kick, and a killed process |
| Live frame rate in the game window; 30 fps limit without focus | Verified |
| Game window screens: joining, connection ended, pause, death, settings over the game | Verified visually |

Not yet exercised: joining an online-mode server (needs an approved app ID), `transfer`,
mid-game reconfiguration.

## Out of scope (see DESIGN §1, §14)

A singleplayer game engine (Phase 10's local world runs the official server instead),
Bedrock/Realms, Java mod loaders, multi-version support.
