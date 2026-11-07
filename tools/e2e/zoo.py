#!/usr/bin/env python3
"""Summons things in rows on a private server and takes pictures of them, to look at.

    cargo build -p lattice && tools/e2e/zoo.py [text in the group name ...]

Pictures go to target/e2e/shots/zoo-*.png. Anything the client couldn't find a shape or a
texture for shows up as a warning at the end.
"""

import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rig  # noqa: E402

GROUND = -60

GROUPS = {
    "people": ["zombie", "husk", "drowned", "skeleton", "stray", "bogged", "parched", "wither_skeleton", "piglin", "piglin_brute", "zombified_piglin", "enderman"],
    "traders": ["villager", "wandering_trader", "witch", "zombie_villager", "pillager", "vindicator", "evoker", "illusioner", "vex", "allay", "armor_stand", "mannequin"],
    "farm": ["cow", "mooshroom", "pig", "sheep", "chicken", "goat", "rabbit", "horse", "donkey", "mule", "llama", "trader_llama"],
    "wild": ["wolf", "cat", "ocelot", "fox", "panda", "polar_bear", "turtle", "frog", "tadpole", "axolotl", "bee", "parrot"],
    "monsters": ["creeper", "spider", "cave_spider", "slime", "magma_cube", "silverfish", "endermite", "blaze", "breeze", "phantom", "ghast", "happy_ghast"],
    "big": ["ravager", "iron_golem", "snow_golem", "copper_golem", "creaking", "warden", "hoglin", "zoglin", "sniffer", "camel", "camel_husk", "armadillo"],
    "water": ["cod", "salmon", "tropical_fish", "pufferfish", "dolphin", "squid", "glow_squid", "guardian", "elder_guardian", "nautilus", "zombie_nautilus", "strider"],
    "undead": ["skeleton_horse", "zombie_horse", "shulker", "wither", "giant", "sulfur_cube"],
    "objects": ["oak_boat", "spruce_chest_boat", "bamboo_raft", "minecart", "chest_minecart", "furnace_minecart", "tnt_minecart", "hopper_minecart", "armor_stand", "end_crystal", "leash_knot", "experience_orb"],
    "projectiles": ["arrow", "spectral_arrow", "trident", "snowball", "egg", "ender_pearl", "fireball", "small_fireball", "wither_skull", "shulker_bullet", "llama_spit", "wind_charge"],
}

# Blocks drawn like entities, placed on a line in front of the camera.
PLACED = [
    "chest[facing=south]", "chest[facing=south,type=left]", "chest[facing=south,type=right]", "trapped_chest[facing=south]",
    "ender_chest[facing=south]", "copper_chest[facing=south]", "shulker_box", "red_shulker_box",
    "white_banner", "player_head", "skeleton_skull", "wither_skeleton_skull",
    "zombie_head", "creeper_head", "dragon_head", "piglin_head",
    "bell[attachment=floor]", "decorated_pot", "enchanting_table", "lectern[has_book=true,facing=south]",
    "conduit", "oak_sign", "white_bed[facing=south]", "end_portal",
]

EXTRAS = {
    "babies": ["zombie", "husk", "drowned", "piglin", "zombified_piglin", "villager", "cow", "pig", "sheep", "chicken", "wolf", "cat", "fox", "horse", "llama", "rabbit", "panda", "polar_bear", "goat", "bee", "axolotl", "hoglin", "strider", "turtle"],
}


def row(s, names, z, nbt="", spacing=3.5, y=GROUND):
    x = -(len(names) - 1) * spacing / 2
    for n in names:
        extra = "{NoAI:1b,NoGravity:1b,Rotation:[0f,0f],Invulnerable:1b" + nbt + "}"
        s.send(f"summon {n} {x:.1f} {y} {z} {extra}")
        x += spacing


ARMOR = "ArmorItems"


def world_look(s, bot):
    """Sky, weather, water and things in the world that move."""
    s.send(f"gamemode spectator {bot.name}")
    s.send("kill @e[type=!player]")
    s.send(f"fill -30 {GROUND} -40 30 {GROUND + 12} 10 air")
    # A pool, a waterfall off a wall, lava, a burning mob, and a portal.
    s.send(f"fill -12 {GROUND - 1} -14 -2 {GROUND - 1} -6 water")
    s.send(f"fill 2 {GROUND - 1} -14 8 {GROUND - 1} -6 lava")
    s.send(f"fill -12 {GROUND} -20 -6 {GROUND + 6} -20 stone")
    s.send(f"setblock -9 {GROUND + 6} -19 water")
    s.send(f"setblock 1 {GROUND} -8 water")
    s.send(f"summon zombie 0 {GROUND} -5 {{NoAI:1b,Rotation:[0f,0f],Fire:1000s}}")
    s.send(f"setblock 12 {GROUND} -12 fire")
    time.sleep(1.5)
    s.send(f"tp {bot.name} 0.5 {GROUND + 7} 3.5 180 22")
    time.sleep(1.0)
    bot.look(180, 22)
    bot.ticks(30)
    print("zoo water", bot.capture("zoo-world-water", world_only=True))
    for name, t, look in [("noon", 6000, -40), ("sunset", 12300, -8), ("night", 18000, -50)]:
        s.send(f"time set {t}")
        s.send(f"effect clear {bot.name}")
        time.sleep(0.5)
        yaw = 90 if name == "sunset" else 180
        bot.look(yaw, look)
        bot.ticks(6)
        print("zoo sky", name, bot.capture(f"zoo-sky-{name}", world_only=True))
    s.send("time set 6000")
    s.send("weather rain")
    time.sleep(0.5)
    bot.look(180, 10)
    bot.ticks(100)
    print("zoo rain", bot.capture("zoo-sky-rain", world_only=True))
    s.send("weather clear")
    s.send(f"effect give {bot.name} minecraft:night_vision infinite 0 true")
    time.sleep(0.3)
    # Breaking a block shows cracks.
    s.send(f"gamemode survival {bot.name}")
    s.send(f"setblock 40 {GROUND} 45 obsidian")
    s.send(f"tp {bot.name} 40.5 {GROUND} 43.5 0 25")
    time.sleep(0.8)
    bot.look(0, 25)
    bot.ticks(5)
    bot.button(0, True)
    bot.ticks(40)
    print("zoo cracks", bot.capture("zoo-cracks", world_only=True))
    bot.button(0, False)
    s.send(f"gamemode spectator {bot.name}")


def motes(s, bot):
    """Particles: things that make them on their own and ones the server sends."""
    s.send(f"gamemode spectator {bot.name}")
    s.send("kill @e[type=!player]")
    s.send(f"fill -20 {GROUND} -20 20 {GROUND + 8} 5 air")
    time.sleep(0.2)
    for i, block in enumerate(["torch", "soul_torch", "fire", "campfire", "furnace[lit=true]", "nether_portal", "end_rod", "candle[lit=true,candles=1]"]):
        s.send(f"setblock {-9 + i * 2} {GROUND} -8 {block}")
    s.send(f"fill -9 {GROUND - 1} -5 -5 {GROUND - 1} -3 lava")
    for name, x in [("heart", -6), ("flame", -4), ("smoke", -2), ("large_smoke", 0), ("explosion", 2), ("crit", 4), ("happy_villager", 6), ("angry_villager", 8), ("poof", 10)]:
        s.send(f"particle minecraft:{name} {x} {GROUND + 2} -6 0.3 0.3 0.3 0.01 12 force")
    s.send(f"particle minecraft:dust{{color:[1.0,0.3,0.0],scale:2.0}} -8 {GROUND + 2} -6 0.3 0.3 0.3 0 20 force")
    s.send(f"particle minecraft:block{{block_state:\"minecraft:gold_block\"}} -10 {GROUND + 2} -6 0.3 0.3 0.3 0 30 force")
    s.send(f"tp {bot.name} 0.5 {GROUND + 4} 3.5 180 20")
    time.sleep(0.5)
    bot.look(180, 20)
    bot.ticks(6)
    print("zoo particles", bot.capture("zoo-particles", world_only=True), bot.dump()["particles"])
    # Breaking a block in survival showers bits.
    s.send(f"gamemode survival {bot.name}")
    s.send(f"setblock 40 {GROUND} 44 gold_block")
    s.send(f"tp {bot.name} 40.5 {GROUND} 41.5 0 20")
    time.sleep(0.8)
    bot.look(0, 20)
    bot.ticks(4)
    s.send(f"item replace entity {bot.name} hotbar.0 with diamond_pickaxe")
    time.sleep(0.2)
    bot.button(0, True)
    bot.ticks(30)
    bot.button(0, False)
    print("zoo bits", bot.capture("zoo-bits", world_only=True), bot.dump()["particles"])
    s.send(f"gamemode spectator {bot.name}")


def menus(s, bot):
    """Each kind of menu and settings page, opened the way a player would."""
    s.send(f"gamemode survival {bot.name}")
    s.send(f"tp {bot.name} 60.5 {GROUND} 60.5 0 30")
    s.send(f"fill 55 {GROUND} 55 66 {GROUND + 4} 66 air")
    s.send(f"clear {bot.name}")
    for item in ["diamond_sword", "stone", "oak_planks", "iron_ingot", "coal", "lapis_lazuli", "emerald", "paper", "leather_helmet", "white_banner", "blaze_powder", "potion", "iron_pickaxe", "written_book", "gold_ingot", "cobblestone"]:
        s.send(f"give {bot.name} {item} 16")
    time.sleep(0.8)
    bot.look(0, 40)
    bot.ticks(4)
    kinds = ["furnace", "brewing_stand", "anvil", "stonecutter", "loom", "beacon", "crafter", "chest", "crafting_table", "enchanting_table", "smithing_table", "grindstone", "cartography_table", "hopper", "dispenser", "barrel", "shulker_box"]
    for kind in kinds:
        s.send(f"setblock 60 {GROUND} 62 {kind}")
        time.sleep(0.4)
        bot.click_mouse(1)
        bot.ticks(12)
        print("zoo menu", kind, bot.capture(f"zoo-menu-{kind}"))
        bot.press("Escape")
        bot.ticks(4)
    s.send(f"setblock 60 {GROUND} 62 lectern[has_book=true]")
    time.sleep(0.3)
    s.send(f'data merge block 60 {GROUND} 62 {{Book:{{id:"written_book",count:1,components:{{"minecraft:written_book_content":{{title:"A Book",author:"Someone",pages:["First page of the book, with enough words on it to wrap over several lines of text.","Second page"]}}}}}},Page:0}}')
    time.sleep(0.3)
    bot.click_mouse(1)
    bot.ticks(12)
    print("zoo menu lectern", bot.capture("zoo-menu-lectern"))
    bot.press("Escape")
    bot.ticks(4)
    # Your own screen, and a trader with more to sell than fits.
    s.send(f"setblock 60 {GROUND} 62 air")
    bot.press("KeyE")
    bot.ticks(12)
    print("zoo inventory", bot.capture("zoo-inventory"))
    bot.press("Escape")
    bot.ticks(4)
    recipes = ",".join(f'{{buy:{{id:"emerald",count:{1 + n}}},buyB:{{id:"book",count:1}},sell:{{id:"{item}",count:1}},maxUses:12,uses:{12 if n == 2 else 0}}}' for n, item in enumerate(["bookshelf", "lantern", "paper", "clock", "compass", "name_tag", "glass", "ink_sac", "feather", "arrow", "bow", "shield"]))
    s.send(f'summon villager 60.5 {GROUND} 62.5 {{NoAI:1b,VillagerData:{{profession:"minecraft:librarian",level:3,type:"minecraft:plains"}},Xp:90,Offers:{{Recipes:[{recipes}]}}}}')
    time.sleep(0.5)
    bot.click_mouse(1)
    bot.ticks(12)
    print("zoo trader", bot.capture("zoo-trader"))
    bot.press("Escape")
    bot.ticks(4)
    s.send("kill @e[type=villager]")
    # The creative inventory.
    s.send(f"setblock 60 {GROUND} 62 air")
    s.send(f"gamemode creative {bot.name}")
    time.sleep(0.4)
    bot.press("KeyE")
    bot.ticks(12)
    print("zoo creative", bot.capture("zoo-creative"))
    bot.press("Escape")
    bot.ticks(4)
    s.send(f"gamemode survival {bot.name}")
    # Pause and settings.
    time.sleep(0.3)
    bot.press("Escape")
    bot.ticks(8)
    print("zoo pause", bot.capture("zoo-pause"))
    bot.click("Settings")
    bot.ticks(8)
    for section in ["Display", "Quality", "Camera and effects", "Mouse and movement", "Key bindings", "Sound", "Chat", "Player", "Interface", "Details"]:
        bot.click(f"section:{section}")
        bot.ticks(6)
        print("zoo settings", section, bot.capture(f"zoo-settings-{section.lower().split()[0]}"))
    bot.press("Escape")
    bot.ticks(4)
    bot.press("Escape")
    bot.ticks(4)
    s.send(f"gamemode spectator {bot.name}")


def labels(s, bot):
    """Writing in the world, and icons of items only code can draw."""
    s.send(f"gamemode spectator {bot.name}")
    s.send("kill @e[type=!player]")
    s.send(f"fill -20 {GROUND} -30 20 {GROUND + 8} 0 air")
    time.sleep(0.3)
    s.send(f'summon zombie -3 {GROUND} -6 {{NoAI:1b,Rotation:[0f,0f],CustomName:"Grumbles",CustomNameVisible:1b}}')
    s.send(f'summon pig 0 {GROUND} -6 {{NoAI:1b,Rotation:[0f,0f],CustomName:{{text:"Bacon",color:"gold",bold:true}},CustomNameVisible:1b}}')
    s.send(f"setblock 3 {GROUND} -6 oak_sign[rotation=0]")
    s.send(f'data merge block 3 {GROUND} -6 {{front_text:{{messages:["Hello","there",{{text:"glowing",color:"red"}},"sign"],color:"black",has_glowing_text:0b}}}}')
    s.send(f"setblock 5 {GROUND} -6 birch_sign[rotation=0]")
    s.send(f'data merge block 5 {GROUND} -6 {{front_text:{{messages:["Glow","ink","","on sign"],color:"cyan",has_glowing_text:1b}}}}')
    s.send(f'summon text_display -6 {GROUND + 1} -6 {{text:"Floating words",billboard:"center",background:1073741824}}')
    s.send(f"tp {bot.name} 0.5 {GROUND + 2} 0.5 180 8")
    time.sleep(1.2)
    bot.look(180, 8)
    bot.ticks(8)
    print("zoo labels", bot.capture("zoo-labels", world_only=True))
    # Icons.
    s.send(f"gamemode survival {bot.name}")
    s.send(f"tp {bot.name} 40.5 {GROUND} 40.5 0 10")
    time.sleep(0.8)
    for i, item in enumerate(["chest", "trapped_chest", "ender_chest", "shield", "white_banner", "red_banner", "skeleton_skull", "player_head", "dragon_head", "creeper_head", "piglin_head", "decorated_pot", "conduit", "shulker_box", "red_shulker_box", "copper_chest", "trident", "copper_golem_statue", "white_bed", "oak_sign", "lectern", "enchanting_table", "bell", "barrier", "spawner", "torch", "oak_stairs", "potion", "leather_helmet", "diamond_sword"]):
        s.send(f"give {bot.name} {item}")
    time.sleep(0.8)
    bot.press("KeyE")
    bot.ticks(10)
    print("zoo icons", bot.capture("zoo-icons"))
    bot.press("Escape")
    bot.ticks(4)
    s.send(f"clear {bot.name}")
    s.send(f"gamemode spectator {bot.name}")


def gear(s, bot):
    """Worn and held things, in the world and in the player's own hands."""
    s.send("kill @e[type=!player]")
    s.send(f"fill -20 {GROUND} -30 20 {GROUND + 8} 0 air")
    time.sleep(0.3)
    equip = (
        '{NoAI:1b,Rotation:[0f,0f],equipment:{head:{id:"iron_helmet",count:1},chest:{id:"diamond_chestplate",count:1},'
        'legs:{id:"golden_leggings",count:1},feet:{id:"leather_boots",count:1},mainhand:{id:"iron_sword",count:1},offhand:{id:"shield",count:1}}}'
    )
    s.send(f"summon zombie -6 {GROUND} -6 {equip}")
    s.send(f"summon skeleton -3 {GROUND} -6 {equip}")
    s.send(f"summon piglin 0 {GROUND} -6 {equip}")
    s.send(f"summon armor_stand 3 {GROUND} -6 {{Rotation:[0f,0f],ShowArms:1b,equipment:{{head:{{id:\"carved_pumpkin\",count:1}},chest:{{id:\"elytra\",count:1}},mainhand:{{id:\"bow\",count:1}}}}}}")
    s.send(f"summon villager 6 {GROUND} -6 {{NoAI:1b,Rotation:[0f,0f],VillagerData:{{type:\"minecraft:desert\",profession:\"minecraft:librarian\",level:3}}}}")
    s.send(f"summon horse -6 {GROUND} -10 {{NoAI:1b,Rotation:[0f,0f],Variant:515,equipment:{{body:{{id:\"diamond_horse_armor\",count:1}},saddle:{{id:\"saddle\",count:1}}}}}}")
    s.send(f"summon wolf -2 {GROUND} -10 {{NoAI:1b,Rotation:[0f,0f],Owner:[I;1,2,3,4],equipment:{{body:{{id:\"wolf_armor\",count:1}}}},CollarColor:14b}}")
    s.send(f"summon pig 2 {GROUND} -10 {{NoAI:1b,Rotation:[0f,0f],equipment:{{saddle:{{id:\"saddle\",count:1}}}}}}")
    s.send(f"summon llama 6 {GROUND} -10 {{NoAI:1b,Rotation:[0f,0f],Variant:2,equipment:{{body:{{id:\"red_carpet\",count:1}}}}}}")
    for i, (item, count) in enumerate([("diamond", 1), ("stone", 64), ("oak_planks", 3), ("torch", 1), ("oak_stairs", 1), ("chest", 1), ("shield", 1), ("trident", 1), ("leather_chestplate", 1), ("potion", 1)]):
        s.send(f"summon item {-6 + i * 1.3} {GROUND} -3 {{Item:{{id:\"{item}\",count:{count}}},PickupDelay:32767,Age:-32768}}")
    s.send(f"setblock -3 {GROUND + 1} -13 stone")
    s.send(f"setblock 3 {GROUND + 2} -13 stone")
    s.send(f"summon item_frame -3 {GROUND + 1} -12 {{Facing:3b,Item:{{id:\"diamond_sword\",count:1}}}}")
    s.send(f"summon painting 3 {GROUND + 2} -12 {{facing:0b,variant:\"minecraft:kebab\"}}")
    s.send(f"tp {bot.name} 0.5 {GROUND + 3} 2.5 180 15")
    time.sleep(1.2)
    bot.look(180, 15)
    bot.ticks(10)
    print("zoo gear", bot.capture("zoo-gear", world_only=True))
    # The player's own view: bare hand, then holding things.
    s.send(f"gamemode survival {bot.name}")
    s.send(f"tp {bot.name} 40.5 {GROUND} 40.5 0 10")
    s.send(f"fill 35 {GROUND} 35 45 {GROUND + 5} 50 air")
    time.sleep(1.0)
    bot.look(0, 10)
    bot.ticks(10)
    print("zoo hand", bot.capture("zoo-hand-empty", world_only=True))
    for slot, (item, name) in enumerate([("diamond_sword", "sword"), ("stone", "block"), ("bow", "bow"), ("shield", "shield"), ("apple", "apple")]):
        s.send(f"item replace entity {bot.name} hotbar.{slot} with {item}")
        time.sleep(0.2)
        bot.press(f"Digit{slot + 1}")
        bot.ticks(14)
        print("zoo hand", name, bot.capture(f"zoo-hand-{name}", world_only=True))
    s.send(f"gamemode spectator {bot.name}")


def effects(s, bot):
    """Shine on enchanted things, a beacon's beam, a bolt of lightning."""
    n = bot.name
    s.send("kill @e[type=!player]")
    s.send("time set noon")
    s.send(f"fill -12 {GROUND} -20 12 {GROUND + 30} 10 air")
    s.send(f"item replace entity {n} weapon.mainhand with minecraft:diamond_sword[enchantments={{sharpness:3}}]")
    s.send(f"item replace entity {n} hotbar.1 with minecraft:enchanted_book[stored_enchantments={{mending:1}}]")
    s.send(f"item replace entity {n} hotbar.2 with minecraft:golden_apple")
    s.send(f"item replace entity {n} hotbar.3 with minecraft:enchanted_golden_apple")
    s.send(
        f"summon zombie 0.5 {GROUND} -3.5 "
        '{NoAI:1b,PersistenceRequired:1b,Rotation:[0f,0f],equipment:{'
        'head:{id:"minecraft:diamond_helmet",count:1,components:{"minecraft:enchantments":{protection:1}}},'
        'chest:{id:"minecraft:iron_chestplate",count:1,components:{"minecraft:enchantments":{protection:1}}},'
        'legs:{id:"minecraft:iron_leggings",count:1},'
        'mainhand:{id:"minecraft:golden_sword",count:1,components:{"minecraft:enchantments":{sharpness:1}}}}}'
    )
    s.send(f"summon item 3.5 {GROUND + 0.5} -3.5 " + '{PickupDelay:32000s,Item:{id:"minecraft:diamond_pickaxe",count:1,components:{"minecraft:enchantments":{efficiency:1}}}}')
    s.send(f"tp {n} 0.5 {GROUND} 1.5 180 10")
    time.sleep(1.0)
    bot.look(180, 10)
    bot.ticks(20)
    print("zoo shine", bot.capture("zoo-shine"))
    bot.press("KeyE", "e")
    bot.ticks(5)
    print("zoo shine menu", bot.capture("zoo-shine-menu"))
    bot.press("Escape")
    bot.ticks(3)
    s.send("kill @e[type=!player]")
    # A beacon with one layer under it, and coloured glass partway up one.
    s.send(f"fill -1 {GROUND} -9 1 {GROUND} -7 iron_block")
    s.send(f"setblock 0 {GROUND + 1} -8 beacon")
    s.send(f"fill 5 {GROUND} -9 7 {GROUND} -7 iron_block")
    s.send(f"setblock 6 {GROUND + 1} -8 beacon")
    s.send(f"setblock 6 {GROUND + 4} -8 red_stained_glass")
    s.send(f"setblock 6 {GROUND + 7} -8 blue_stained_glass")
    s.send(f"setblock -3 {GROUND} -5 redstone_wire")
    s.send(f"setblock -4 {GROUND} -5 redstone_wire")
    s.send(f"setblock -5 {GROUND} -5 redstone_block")
    s.send(f"setblock -3 {GROUND} -4 redstone_wire")
    s.send(f"fill 3 {GROUND} -5 5 {GROUND + 1} -5 lime_stained_glass_pane")
    time.sleep(1.0)
    s.send(f"tp {n} 3.5 {GROUND} 6.5 180 -25")
    time.sleep(0.5)
    bot.look(180, -25)
    bot.ticks(20)
    print("zoo beam", bot.capture("zoo-beam", world_only=True))
    s.send(f"summon lightning_bolt -6 {GROUND} -12")
    bot.ticks(2)
    print("zoo bolt", bot.capture("zoo-bolt", world_only=True))
    bot.ticks(8)


def extras(s, bot):
    """Trims, leads, what's cooking, what's in a spawner, what's stuck in a player."""
    n = bot.name
    s.send("kill @e[type=!player]")
    s.send(f"fill -12 {GROUND} -20 12 {GROUND + 30} 10 air")
    trim = lambda pattern, metal: f'components:{{"minecraft:trim":{{pattern:"minecraft:{pattern}",material:"minecraft:{metal}"}}}}'
    s.send(
        f"summon zombie -2.5 {GROUND} -3.5 "
        '{NoAI:1b,PersistenceRequired:1b,Rotation:[0f,0f],equipment:{'
        f'head:{{id:"minecraft:iron_helmet",count:1,{trim("wild", "gold")}}},'
        f'chest:{{id:"minecraft:iron_chestplate",count:1,{trim("coast", "redstone")}}},'
        f'legs:{{id:"minecraft:iron_leggings",count:1,{trim("rib", "iron")}}},'
        f'feet:{{id:"minecraft:iron_boots",count:1,{trim("snout", "emerald")}}}}}}}'
    )
    s.send(f"setblock 2 {GROUND} -3 campfire" + '{Items:[{Slot:0b,id:"minecraft:beef",count:1},{Slot:2b,id:"minecraft:cod",count:1}],CookingTotalTimes:[I;6000,6000,6000,6000]}')
    s.send(f"setblock 5 {GROUND} -3 spawner" + '{SpawnData:{entity:{id:"minecraft:zombie"}}}')
    s.send(f"setblock -6 {GROUND} -3 oak_fence")
    s.send(f"summon leash_knot -6 {GROUND} -3")
    s.send(f"summon pig -3.5 {GROUND} 0.5 " + "{NoAI:1b,leash:[I;-6," + str(GROUND) + ",-3]}")
    s.send(f"summon sheep 5.5 {GROUND} 0.5 " + "{NoAI:1b}")
    s.send(f"setblock 8 {GROUND} -3 oak_shelf[facing=south]" + '{Items:[{Slot:0b,id:"minecraft:diamond",count:1},{Slot:1b,id:"minecraft:stone",count:1},{Slot:2b,id:"minecraft:golden_apple",count:1}]}')
    s.send(f"tp {n} 0.5 {GROUND + 1} 4.5 180 12")
    time.sleep(1.5)
    bot.look(180, 12)
    bot.ticks(15)
    print("zoo extras", bot.capture("zoo-extras", world_only=True))
    # A piston pushing a block, caught in the act over a few frames.
    s.send(f"fill -9 {GROUND} 1 -6 {GROUND} 1 air")
    s.send(f"setblock -8 {GROUND} 1 piston[facing=east]")
    s.send(f"setblock -7 {GROUND} 1 gold_block")
    time.sleep(0.3)
    s.send(f"setblock -9 {GROUND} 1 redstone_block")
    bot.ticks(1)
    print("zoo piston", bot.capture("zoo-piston", world_only=True))
    bot.ticks(6)
    print("zoo piston done", bot.capture("zoo-piston-done", world_only=True))
    # A cape, from behind.
    s.send(f"gamemode survival {n}")
    s.send(f"effect give {n} minecraft:speed 5 1 true")
    s.send(f"effect give {n} minecraft:poison 30 0 false")
    s.send(f"effect give {n} minecraft:regeneration 8 0 false")
    bot.cmd(t="test_cape")
    bot.ticks(5)
    bot.press("F5")
    bot.key("KeyW", True)
    bot.ticks(25)
    print("zoo cape", bot.capture("zoo-cape"))
    bot.key("KeyW", False)
    bot.press("F5")
    bot.press("F5")
    bot.ticks(5)
    s.send(f"effect clear {n}")
    s.send(f"gamemode spectator {n}")
    buddy = rig.Client("Buddy")
    try:
        buddy.wait_playing()
        s.send(f"gamemode survival Buddy")
        s.send(f"tp Buddy 0.5 {GROUND} -2.5 0 0")
        time.sleep(0.5)
        for dx, dz in [(0.1, 0.1), (-0.2, 0.05), (0.05, -0.1)]:
            s.send(f"summon arrow {0.5 + dx} {GROUND + 3} {-2.5 + dz} " + "{Motion:[0.0,-1.5,0.0]}")
            time.sleep(0.4)
        s.send(f"kill @e[type=arrow]")
        time.sleep(0.5)
        bot.look(180, 12)
        bot.ticks(10)
        print("zoo stuck", bot.capture("zoo-stuck", world_only=True))
    finally:
        buddy.quit()


def fixes(s, bot):
    """Things that used to be missing, each where it can be seen clearly."""
    s.send(f"gamemode creative {bot.name}")
    s.send(f"tp {bot.name} 90.5 {GROUND} 90.5 0 20")
    s.send(f"fill 84 {GROUND} 92 98 {GROUND + 5} 104 air")
    # The End portal's stars, and a gateway beside them.
    s.send(f"fill 86 {GROUND} 95 88 {GROUND} 97 end_portal")
    s.send(f"setblock 92 {GROUND + 1} 96 end_gateway")
    # Copper golem statues in each pose, and each state of oxidation.
    poses = ["standing", "sitting", "running", "star"]
    levels = ["copper_golem_statue", "exposed_copper_golem_statue", "weathered_copper_golem_statue", "oxidized_copper_golem_statue"]
    for i, (pose, block) in enumerate(zip(poses, levels)):
        s.send(f"setblock {86 + i * 2} {GROUND} 93 {block}[copper_golem_pose={pose},facing=north]")
    time.sleep(1.0)
    bot.look(0, 20)
    bot.ticks(20)
    print("zoo fixes blocks", bot.capture("zoo-fix-blocks", world_only=True))
    s.send("fill 84 %d 92 98 %d 104 air" % (GROUND, GROUND + 5))
    # Energy over a charged creeper and a weakened wither.
    s.send(f"summon creeper 89 {GROUND} 95 {{powered:1b,NoAI:1b,Rotation:[180f,0f]}}")
    s.send(f"summon wither 93 {GROUND} 97 {{NoAI:1b,Health:100f,Invul:0,Rotation:[180f,0f]}}")
    time.sleep(0.6)
    bot.ticks(20)
    print("zoo fixes energy", bot.capture("zoo-fix-energy", world_only=True))
    s.send("kill @e[type=creeper]")
    s.send("kill @e[type=wither]")
    # Item icons: trimmed armour, and a banner with patterns.
    s.send(f"clear {bot.name}")
    s.send(f"gamemode survival {bot.name}")
    s.send(f"tp {bot.name} 90.5 {GROUND} 90.5 0 0")
    for slot, item in enumerate([
        'iron_chestplate[trim={material:"minecraft:gold",pattern:"minecraft:sentry"}]',
        'diamond_helmet[trim={material:"minecraft:redstone",pattern:"minecraft:coast"}]',
        'netherite_boots[trim={material:"minecraft:emerald",pattern:"minecraft:wild"}]',
        'golden_leggings[trim={material:"minecraft:gold",pattern:"minecraft:ward"}]',
        'white_banner[banner_patterns=[{pattern:"minecraft:creeper",color:"green"},{pattern:"minecraft:stripe_top",color:"red"}]]',
        'blue_banner[banner_patterns=[{pattern:"minecraft:cross",color:"yellow"}]]',
        'shield[base_color="red",banner_patterns=[{pattern:"minecraft:cross",color:"white"}]]',
        'shield',
    ]):
        s.send(f"item replace entity {bot.name} hotbar.{slot} with minecraft:{item}")
    bot.ticks(10)
    bot.press("KeyE", "e")
    bot.ticks(8)
    print("zoo fixes icons", bot.capture("zoo-fix-icons"))
    bot.press("Escape")
    bot.ticks(4)
    # The line from a rod to its float.
    s.send(f"clear {bot.name}")
    s.send(f"item replace entity {bot.name} weapon.mainhand with fishing_rod")
    bot.look(0, -10)
    bot.ticks(6)
    bot.click_mouse(1)
    bot.ticks(40)
    print("zoo fixes fishing", bot.capture("zoo-fix-fishing"))
    bot.click_mouse(1)
    # Darkness pulses, and fog closes in for a boss that asks for it.
    s.send(f"effect give {bot.name} darkness 20 0 true")
    bot.ticks(30)
    print("zoo fixes darkness", bot.capture("zoo-fix-darkness", world_only=True))
    s.send(f"effect clear {bot.name}")
    # Armour and air on the HUD.
    s.send(f"item replace entity {bot.name} armor.chest with iron_chestplate")
    s.send(f"item replace entity {bot.name} armor.head with iron_helmet")
    s.send(f"data merge entity {bot.name} {{Air:120s}}")
    s.send(f"fill 88 {GROUND} 88 93 {GROUND + 3} 93 water")
    bot.ticks(30)
    print("zoo fixes hud", bot.capture("zoo-fix-hud"))
    s.send(f"fill 88 {GROUND} 88 93 {GROUND + 3} 93 air")
    s.send(f"clear {bot.name}")


def main():
    wanted = sys.argv[1:]
    server = rig.Server(fresh=True)
    bot = None
    try:
        bot = rig.Client("Zoo")
        bot.wait_playing()
        shoot(server, bot, wanted)
    finally:
        if bot:
            bot.quit()
        server.stop()


def shoot(s, bot, wanted):
    s.send("time set noon")
    s.send("weather clear")
    s.send(f"op {bot.name}")
    s.send(f"gamemode spectator {bot.name}")
    s.send(f"effect give {bot.name} minecraft:night_vision infinite 0 true")
    time.sleep(1.0)
    for name, entities in GROUPS.items():
        if wanted and not any(w in name for w in wanted):
            continue
        s.send("kill @e[type=!player]")
        time.sleep(0.3)
        half = len(entities) // 2
        z_front, z_back = -9, -16
        row(s, entities[:half], z_front)
        row(s, entities[half:], z_back, spacing=5.0)
        # The camera stands to the north and looks back along the rows.
        s.send(f"tp {bot.name} 0.5 {GROUND + 5} 0.5 180 18")
        time.sleep(1.0)
        bot.look(180, 18)
        bot.ticks(8)
        print("zoo", name, bot.capture(f"zoo-{name}", world_only=True))
    if not wanted or any("placed" in w for w in wanted):
        s.send("kill @e[type=!player]")
        s.send(f"fill -20 {GROUND} -30 20 {GROUND + 8} 0 air")
        time.sleep(0.3)
        for i, block in enumerate(PLACED):
            x = -9 + (i % 8) * 2.5
            z = -5 - (i // 8) * 3
            s.send(f"setblock {int(x)} {GROUND} {z} {block}")
        s.send(f"data merge block -9 {GROUND} -8 {{patterns:[{{pattern:\"minecraft:cross\",color:\"red\"}},{{pattern:\"minecraft:border\",color:\"blue\"}}]}}")
        s.send(f"tp {bot.name} 0.5 {GROUND + 4} 3.5 180 22")
        time.sleep(1.0)
        bot.look(180, 22)
        bot.ticks(8)
        print("zoo placed", bot.capture("zoo-placed", world_only=True))
    if not wanted or any("fixes" in w for w in wanted):
        s.send("kill @e[type=!player]")
        fixes(s, bot)
    if not wanted or any("gear" in w for w in wanted):
        gear(s, bot)
    if not wanted or any("labels" in w for w in wanted):
        labels(s, bot)
    if not wanted or any("effects" in w for w in wanted):
        effects(s, bot)
    if not wanted or any("extras" in w for w in wanted):
        extras(s, bot)
    if not wanted or any("sky" in w for w in wanted):
        world_look(s, bot)
    if not wanted or any("motes" in w for w in wanted):
        motes(s, bot)
    if not wanted or any("menus" in w for w in wanted):
        menus(s, bot)
    if not wanted or any("babies" in w for w in wanted):
        s.send("kill @e[type=!player]")
        time.sleep(0.3)
        kinds = EXTRAS["babies"]
        row(s, kinds[:12], -7, ",IsBaby:1b,Age:-24000", spacing=1.7)
        row(s, kinds[12:], -11, ",IsBaby:1b,Age:-24000", spacing=1.7)
        s.send(f"tp {bot.name} 0.5 {GROUND + 3} 0.5 180 18")
        time.sleep(1.0)
        bot.look(180, 18)
        bot.ticks(8)
        print("zoo babies", bot.capture("zoo-babies", world_only=True))
    problems = bot.problems()
    if problems:
        print("\nthe client complained:")
        for p in problems:
            print(" ", p)


if __name__ == "__main__":
    main()
