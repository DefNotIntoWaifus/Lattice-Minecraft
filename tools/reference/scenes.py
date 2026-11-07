"""Movement scenes recorded from the official client; see run.py.

Each scene is a small world built around (0, 101, 0) plus input over time. Together they
cover the movement rules Lattice has to match. The recordings are plain numbers (positions
and velocities per tick) and are replayed by crates/lattice-world/tests/golden.rs.
"""

from dataclasses import dataclass, field

AREA = (-16, 99, -16, 16, 125, 16)
FLOOR = "fill -16 100 -16 16 100 16 minecraft:stone"
Seg = tuple[int, tuple[str, ...], float | None, float | None]


def seg(ticks: int, keys: str = "", yaw: float | None = None, pitch: float | None = None) -> Seg:
    return (ticks, tuple(k for k in keys.split(",") if k), yaw, pitch)


@dataclass
class Scene:
    name: str
    about: str
    build: list[str]
    start: tuple[float, float, float]
    segments: list[Seg]
    yaw: float = 0.0
    pitch: float = 0.0
    commands: list[str] = field(default_factory=list)
    cleanup: list[str] = field(default_factory=list)
    settle: float = 1.0
    attrs: dict = field(default_factory=dict)
    effects: dict = field(default_factory=dict)
    mode: str = "survival"
    # Passed to the physics as outside knowledge: worn items and such.
    worn: dict = field(default_factory=dict)


def pool(depth: int = 8, block: str = "minecraft:water") -> list[str]:
    """A walled pool on the floor, 13 wide, filled to the given depth."""
    top = 100 + depth
    return [
        FLOOR,
        f"fill -7 101 -7 7 {top + 3} 7 minecraft:stone",
        f"fill -6 101 -6 6 {top + 3} 6 minecraft:air",
        f"fill -6 101 -6 6 {top} 6 {block}",
    ]


def all_scenes() -> list[Scene]:
    s: list[Scene] = []

    def add(*a, **k):
        s.append(Scene(*a, **k))

    # --- control: land, so a mismatch here means the rig and not the water code
    add("land_walk", "Walking and sprinting on stone", [FLOOR], (0.5, 101.0, 0.5),
        [seg(10), seg(30, "forward"), seg(20, "forward,sprint"), seg(15)])
    add("land_jump", "Standing and sprinting jumps, strafing", [FLOOR], (0.5, 101.0, -10.5),
        [seg(5), seg(14, "jump"), seg(20, "forward,sprint,jump"), seg(14, "left,forward"), seg(14)])
    add("land_sneak", "Sneaking", [FLOOR], (0.5, 101.0, 0.5),
        [seg(5), seg(30, "forward,sneak"), seg(10)])
    add("land_step", "Stepping up a slab and running into a wall",
        [FLOOR, "fill -5 101 3 5 101 3 minecraft:stone_slab", "fill -5 101 8 5 104 8 minecraft:stone"],
        (0.5, 101.0, 0.5), [seg(5), seg(50, "forward"), seg(10)])

    # --- still water
    add("water_sink", "Sinking through deep still water without input", pool(8), (0.5, 107.0, 0.5),
        [seg(60)])
    add("water_walk_floor", "Walking along the bottom of a pool", pool(8), (0.5, 101.0, -4.5),
        [seg(10), seg(50, "forward")])
    add("water_swim_up", "Holding jump underwater, surfacing and bobbing", pool(8), (0.5, 101.0, 0.5),
        [seg(5), seg(100, "jump")])
    add("water_sneak_down", "Holding sneak underwater", pool(8), (0.5, 107.0, 0.5),
        [seg(40, "sneak")])
    add("water_forward_surface", "Holding forward at the surface", pool(8), (0.5, 108.2, -5.5),
        [seg(50, "forward")])
    add("water_sprint_swim", "Sprint-swimming underwater looking level, down and up", pool(8), (0.5, 104.5, -5.0),
        [seg(3), seg(30, "forward,sprint", None, 0.0), seg(15, "forward,sprint", 180.0, 40.0), seg(25, "forward,sprint", 0.0, -40.0), seg(20)],
        settle=0.6)
    add("water_swim_look_down", "Sprint-swimming with a steep look down", pool(8), (0.5, 107.0, -5.0),
        [seg(3), seg(25, "forward,sprint", 0.0, 75.0), seg(10)], settle=0.6)
    add("water_climb_out", "Swimming into a pool edge and climbing out",
        pool(1) + ["fill -7 101 -7 7 104 7 minecraft:air", "fill -6 100 -6 6 100 6 minecraft:stone", "fill -6 101 -6 6 101 6 minecraft:water", "fill -16 101 -16 16 101 -7 minecraft:stone"],
        (0.5, 101.5, -3.0), [seg(5), seg(40, "forward,jump", 180.0, 0.0), seg(20, "forward", 180.0, 0.0)])

    # --- shallow water
    add("water_shallow_walk", "Walking and jumping in one block of water",
        [FLOOR, "fill -8 101 -8 8 101 8 minecraft:water"], (0.5, 101.0, -6.5),
        [seg(5), seg(25, "forward"), seg(25, "forward,jump"), seg(10)])
    add("water_shallow_sprint", "Sprinting through one block of water",
        [FLOOR, "fill -8 101 -8 8 101 8 minecraft:water"], (0.5, 101.0, -6.5),
        [seg(5), seg(40, "forward,sprint"), seg(10)])
    add("water_step", "Walking up a step on the pool floor",
        pool(4) + ["fill -3 101 0 3 101 6 minecraft:stone_slab[waterlogged=true]"],
        (0.5, 101.0, -4.5), [seg(5), seg(40, "forward")])

    # --- currents
    flow = ["fill -8 101 -8 8 101 8 minecraft:stone", "fill -6 101 -6 6 101 6 minecraft:air"]
    channel = [FLOOR] + flow + [f"fill -6 101 {z} 6 101 {z} minecraft:water[level={min(7, max(0, z + 6))}]" for z in range(-6, 7)]
    add("water_current", "Carried along by flowing water", channel, (0.5, 101.0, -4.5), [seg(50)])
    add("water_current_against", "Walking against flowing water", channel, (0.5, 101.0, 3.5),
        [seg(3), seg(40, "forward", 180.0, 0.0)])
    fall = [FLOOR, "fill -2 101 -2 2 112 2 minecraft:stone", "fill -1 101 -1 1 112 1 minecraft:air", "fill 0 101 0 0 112 0 minecraft:water[level=8]"]
    add("water_falling", "Dropping through a waterfall", fall, (0.5, 110.0, 0.5), [seg(40)])
    add("water_bubbles_up", "Lifted by a bubble column",
        pool(8) + ["fill 0 100 0 0 100 0 minecraft:soul_sand", "fill 0 101 0 0 108 0 minecraft:bubble_column[drag=false]"],
        (0.5, 101.2, 0.5), [seg(60)])
    add("water_bubbles_down", "Pulled down by a bubble column",
        pool(8) + ["fill 0 100 0 0 100 0 minecraft:magma_block", "fill 0 101 0 0 108 0 minecraft:bubble_column[drag=true]"],
        (0.5, 107.5, 0.5), [seg(40)])

    # --- lava
    add("lava_sink", "Sinking and walking in lava", pool(5, "minecraft:lava"), (0.5, 104.5, 0.5),
        [seg(30), seg(30, "forward")])
    add("lava_swim_up", "Holding jump in lava", pool(5, "minecraft:lava"), (0.5, 101.0, 0.5),
        [seg(5), seg(60, "jump")])

    # --- climbing
    ladder = [FLOOR, "fill -2 101 3 2 110 3 minecraft:stone", "fill 0 101 2 0 110 2 minecraft:ladder[facing=north]"]
    add("climb_ladder_up", "Climbing a ladder by walking into it", ladder, (0.5, 101.0, 0.5),
        [seg(5), seg(50, "forward"), seg(15)])
    add("climb_ladder_sneak", "Holding on to a ladder with sneak", ladder, (0.5, 101.0, 1.6),
        [seg(25, "forward"), seg(25, "sneak")])
    add("climb_ladder_down", "Sliding down a ladder", ladder, (0.5, 107.0, 2.5), [seg(40)])

    # --- block effects
    add("block_soul_sand", "Walking on soul sand", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:soul_sand"], (0.5, 101.0, -6.5),
        [seg(5), seg(30, "forward"), seg(10, "forward,jump")])
    add("block_honey", "Walking and jumping on honey", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:honey_block"], (0.5, 101.0, -6.5),
        [seg(5), seg(30, "forward"), seg(15, "forward,jump")])
    add("block_slime", "Bouncing on slime", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:slime_block"], (0.5, 106.0, 0.5), [seg(60)])
    add("block_ice", "Sliding on ice", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:ice"], (0.5, 101.0, -6.5),
        [seg(5), seg(15, "forward"), seg(30)])
    add("block_cobweb", "Walking through cobwebs", [FLOOR, "fill -8 101 -3 8 104 3 minecraft:cobweb"], (0.5, 101.0, -5.5),
        [seg(5), seg(50, "forward")])
    add("block_berry", "Walking through a berry bush", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:grass_block", "fill -8 101 -3 8 101 3 minecraft:sweet_berry_bush[age=0]"],
        (0.5, 101.0, -5.5), [seg(5), seg(40, "forward")])
    add("block_powder_snow", "Standing and sinking in powder snow", [FLOOR, "fill -8 101 -8 8 103 8 minecraft:powder_snow"], (0.5, 104.0, 0.5),
        [seg(40)])
    add("block_scaffolding", "Climbing scaffolding with jump", [FLOOR, "fill 0 101 0 0 106 0 minecraft:scaffolding[bottom=false,distance=0]"], (0.5, 101.0, 0.5),
        [seg(30, "jump"), seg(20, "sneak")])

    # --- sneak edge guard and crawling
    add("edge_guard", "Sneaking off a ledge stops at the edge", [FLOOR, "fill -8 101 -8 8 103 2 minecraft:stone"], (0.5, 104.0, -1.5),
        [seg(5), seg(40, "forward,sneak"), seg(10)])

    # --- effects and attributes
    add("effect_speed", "Speed II", [FLOOR], (0.5, 101.0, -10.5), [seg(5), seg(30, "forward"), seg(10)],
        commands=["effect give Ref minecraft:speed infinite 1 true"], cleanup=["effect clear Ref minecraft:speed"], effects={"speed": 1}, settle=1.5)
    add("effect_jump", "Jump Boost II", [FLOOR], (0.5, 101.0, 0.5), [seg(5), seg(30, "jump")],
        commands=["effect give Ref minecraft:jump_boost infinite 1 true"], cleanup=["effect clear Ref minecraft:jump_boost"], effects={"jump_boost": 1}, settle=1.5)
    add("effect_slow_falling", "Slow Falling", [FLOOR], (0.5, 115.0, 0.5), [seg(40)],
        commands=["effect give Ref minecraft:slow_falling infinite 0 true"], cleanup=["effect clear Ref minecraft:slow_falling"], effects={"slow_falling": 0})
    add("effect_levitation", "Levitation I", [FLOOR], (0.5, 101.0, 0.5), [seg(40)],
        commands=["effect give Ref minecraft:levitation infinite 0 true"], cleanup=["effect clear Ref minecraft:levitation"], effects={"levitation": 0}, settle=1.5)
    add("effect_dolphin", "Dolphin's Grace", pool(8), (0.5, 104.5, -5.0), [seg(3), seg(30, "forward,sprint", 0.0, 0.0), seg(5)],
        commands=["effect give Ref minecraft:dolphins_grace infinite 0 true"], cleanup=["effect clear Ref minecraft:dolphins_grace"], effects={"dolphins_grace": 0}, settle=0.6)

    # --- sprinting by double tap, diagonals, stairs, fences, falling
    add("land_double_tap", "Double-tapping forward starts a sprint", [FLOOR], (0.5, 101.0, -10.5),
        [seg(3), seg(2, "forward"), seg(2), seg(25, "forward"), seg(5)])
    add("land_diagonal", "Forward and strafing together, turning while running", [FLOOR], (0.5, 101.0, 0.5),
        [seg(5), seg(20, "forward,left", 30.0, 0.0), seg(20, "forward,right,sprint", -70.0, 0.0), seg(10, "back"), seg(5)])
    stairs = [FLOOR] + [f"fill -3 {101 + i} {i * 1} 3 {101 + i} {i * 1} minecraft:oak_stairs[facing=south]" for i in range(6)]
    add("land_stairs", "Running up stairs", stairs, (0.5, 101.0, -3.5), [seg(5), seg(40, "forward,sprint"), seg(5)])
    add("land_fence", "Walking into a fence and jumping at it",
        [FLOOR, "fill -5 101 3 5 101 3 minecraft:oak_fence[north=true,south=true]"], (0.5, 101.0, 0.5),
        [seg(5), seg(25, "forward"), seg(25, "forward,jump")])
    add("land_low_ceiling", "Jumping under a low ceiling", [FLOOR, "fill -5 104 -5 5 104 5 minecraft:stone"], (0.5, 101.0, 0.5),
        [seg(5), seg(20, "jump")])
    add("land_fall_far", "Falling a long way, then into water", [FLOOR, "fill -3 101 -3 3 103 3 minecraft:water"], (0.5, 124.0, 0.5), [seg(50)], settle=0.2)
    add("land_crouch_gap", "Walking under a 1.5 block gap", [FLOOR, "fill -3 102 2 3 110 8 minecraft:stone", "fill -3 102 2 3 102 8 minecraft:stone_slab[type=top]"],
        (0.5, 101.0, 0.5), [seg(3), seg(30, "forward,sneak"), seg(20, "forward")])
    add("land_crawl", "Crawling after swimming into a one block gap",
        pool(3) + ["fill -6 102 2 6 103 6 minecraft:stone", "fill -6 101 2 6 101 6 minecraft:air", "fill -6 101 -6 6 101 1 minecraft:water", "fill -6 101 7 6 103 7 minecraft:stone"],
        (0.5, 101.0, -3.5), [seg(3), seg(40, "forward,sprint", 0.0, 0.0), seg(20, "forward", 0.0, 0.0)], settle=0.5)

    # --- flying and spectating
    add("fly_toggle", "Double-tapping jump to fly, moving, and landing", [FLOOR], (0.5, 101.0, 0.5),
        [seg(3), seg(2, "jump"), seg(2), seg(2, "jump"), seg(10), seg(20, "forward"), seg(15, "forward,sprint"), seg(10, "jump"), seg(30, "sneak")],
        commands=["gamemode creative Ref"], cleanup=["gamemode survival Ref"], mode="creative")
    add("spectator_noclip", "Flying through walls as a spectator", [FLOOR, "fill -3 101 3 3 110 5 minecraft:stone"], (0.5, 101.5, 0.5),
        [seg(30, "forward"), seg(10, "sneak"), seg(10, "jump")],
        commands=["gamemode spectator Ref"], cleanup=["gamemode survival Ref"], mode="spectator")

    # --- more blocks
    add("climb_vine", "Climbing vines", [FLOOR, "fill -2 101 3 2 110 3 minecraft:stone", "fill 0 101 2 0 110 2 minecraft:vine[south=true]"], (0.5, 101.0, 1.5),
        [seg(30, "forward"), seg(10)])
    add("climb_trapdoor", "A trapdoor above a ladder climbs like a ladder",
        [FLOOR, "fill -2 101 3 2 110 3 minecraft:stone", "fill 0 101 2 0 102 2 minecraft:ladder[facing=north]", "fill 0 103 2 0 103 2 minecraft:oak_trapdoor[facing=north,open=true]"],
        (0.5, 101.0, 1.5), [seg(40, "forward")])
    add("block_bed", "Bouncing on a bed", [FLOOR, "fill 0 101 0 0 101 0 minecraft:white_bed[part=foot,facing=south]", "fill 0 101 1 0 101 1 minecraft:white_bed[part=head,facing=south]"], (0.5, 106.0, 0.5), [seg(40)])
    add("block_slime_sneak", "Landing on slime while sneaking doesn't bounce", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:slime_block"], (0.5, 104.0, 0.5), [seg(40, "sneak")])
    add("block_slime_walk", "Walking on slime is slow", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:slime_block"], (0.5, 101.0, -6.5), [seg(3), seg(40, "forward")])
    add("block_honey_wall", "Sliding down the side of honey",
        [FLOOR, "fill -2 101 2 2 112 3 minecraft:honey_block"], (0.5, 111.0, 1.2), [seg(50, "forward")], settle=0.2)
    add("block_blue_ice", "Sprinting on blue ice", [FLOOR, "fill -8 100 -8 8 100 8 minecraft:blue_ice"], (0.5, 101.0, -6.5), [seg(3), seg(15, "forward,sprint"), seg(20)])
    add("block_powder_boots", "Walking on powder snow in leather boots", [FLOOR, "fill -8 101 -8 8 103 8 minecraft:powder_snow"], (0.5, 104.0, 0.5),
        [seg(20), seg(20, "forward"), seg(25, "sneak")],
        commands=["item replace entity Ref armor.feet with minecraft:leather_boots"], cleanup=["item replace entity Ref armor.feet with minecraft:air"], worn={"leather_boots": True}, settle=1.5)
    add("block_powder_fall", "Falling onto powder snow from high up", [FLOOR, "fill -8 101 -8 8 103 8 minecraft:powder_snow"], (0.5, 115.0, 0.5), [seg(40)], settle=0.2)

    # --- enchantments and gliding
    add("depth_strider", "Depth Strider III", pool(4), (0.5, 101.0, -5.5), [seg(3), seg(30, "forward"), seg(15, "forward,jump")],
        commands=["item replace entity Ref armor.feet with minecraft:diamond_boots[minecraft:enchantments={'minecraft:depth_strider':3}]"], cleanup=["item replace entity Ref armor.feet with minecraft:air"],
        worn={"water_movement_efficiency": 1.0}, settle=1.5)
    add("elytra_glide", "Gliding with an elytra, diving and pulling up", [FLOOR], (0.5, 124.0, -12.0),
        [seg(4), seg(2, "jump"), seg(15, "", 0.0, 10.0), seg(10, "", 0.0, 45.0), seg(15, "", 0.0, -30.0)],
        commands=["item replace entity Ref armor.chest with minecraft:elytra"], cleanup=["item replace entity Ref armor.chest with minecraft:air"], worn={"can_glide": True}, settle=0.1)

    return s
