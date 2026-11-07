"""What gets played. Each function plays one thing through on a real server.

`t` holds `server`, `bot` and `check`. Scenarios run in order on the same world, each in
its own area, and leave the bot healthy and in survival mode.
"""

import json
import rig
import re
import time

SCENARIOS = []


def scenario(f):
    SCENARIOS.append(f)
    return f


# Flat world: grass on top at y = -61, so the player stands at y = -60.
GROUND = -60


def arena(t, x=0, z=0):
    """Clears an area and puts the bot in the middle of it, looking south."""
    s = t.server
    d = t.bot.dump()
    if d["dead"]:
        t.bot.click("Respawn")
        t.bot.ticks(20)
    # A scenario that stopped early may have left something open.
    for _ in range(4):
        d = t.bot.dump()
        if "chat" in d["overlays"] or d["paused"] or d["menu"] or d["modal"] or d["own_screen"]:
            t.bot.press("Escape")
            t.bot.ticks(3)
    s.send(f"fill {x - 10} {GROUND} {z - 10} {x + 10} {GROUND + 12} {z + 10} air")
    s.send(f"kill @e[type=!player]")
    s.send(f"clear {t.bot.name}")
    s.send(f"gamemode survival {t.bot.name}")
    s.send(f"effect clear {t.bot.name}")
    # From wherever an earlier scenario left them, even another dimension.
    s.send(f"execute in minecraft:overworld run tp {t.bot.name} {x + 0.5} {GROUND} {z + 0.5} 0 0")
    time.sleep(0.8)
    t.bot.look(0, 0)
    t.bot.ticks(3)


def inventory_item(d, slot):
    for s in d["slots"]:
        if s["slot"] == slot:
            return s
    return None


@scenario
def joins_and_stands_on_the_ground(t):
    d = t.bot.dump()
    t.check(d["phase"] == "Playing", f"phase is {d['phase']}")
    arena(t)
    d = t.bot.dump()
    t.check(d["on_ground"], "standing on the ground")
    t.check(abs(d["pos"][1] - GROUND) < 0.01, f"at y {d['pos'][1]}")


@scenario
def walks_and_the_server_agrees(t):
    arena(t)
    t.bot.key("KeyW", True)
    t.bot.ticks(20)
    t.bot.key("KeyW", False)
    t.bot.ticks(10)
    d = t.bot.dump()
    t.check(d["pos"][2] > 2.5, f"walked to z {d['pos'][2]}")
    answer = t.server.ask(f"data get entity {t.bot.name} Pos", "has the following")
    z = float(re.findall(r"(-?[\d.]+)d", answer)[2])
    t.check(abs(z - d["pos"][2]) < 0.05, f"the server has z {z}, the client {d['pos'][2]}")


@scenario
def teleport_turns_the_camera(t):
    arena(t)
    t.server.send(f"tp {t.bot.name} 0.5 {GROUND} 0.5 123 -20")
    t.bot.ticks(10)
    d = t.bot.dump()
    t.check(abs(d["yaw"] - 123) < 0.5, f"yaw {d['yaw']}")
    t.check(abs(d["pitch"] + 20) < 0.5, f"pitch {d['pitch']}")
    t.bot.ticks(5)
    d = t.bot.dump()
    t.check(abs(d["yaw"] - 123) < 0.5, f"the camera kept the rotation: yaw {d['yaw']}")


@scenario
def teleport_far_away_falls_and_lands(t):
    arena(t)
    t.server.send(f"effect give {t.bot.name} minecraft:resistance 30 255 true")
    t.server.send(f"tp {t.bot.name} 3000.5 {GROUND + 40} 3000.5")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(not d["on_ground"], "not on the ground in mid-air")
    t.bot.ticks(100)
    d = t.bot.dump()
    t.check(d["on_ground"], f"landed: y {d['pos'][1]}")
    t.server.send(f"tp {t.bot.name} 0.5 {GROUND} 0.5")
    t.bot.ticks(40)


@scenario
def swims_in_water_and_the_server_agrees(t):
    arena(t)
    s = t.server
    s.send(f"fill -6 {GROUND} -6 6 {GROUND + 5} 6 stone")
    s.send(f"fill -5 {GROUND} -5 5 {GROUND + 5} 5 water")
    s.send(f"tp {t.bot.name} 0.5 {GROUND + 1} 0.5 0 0")
    d = t.wait(lambda d: d["in_water"], 3)
    t.check(d["in_water"], f"in the water at {d['pos']}, on ground {d['on_ground']}")
    # Hold jump to rise to the surface.
    t.bot.key("Space", True)
    t.bot.ticks(40)
    t.bot.key("Space", False)
    d = t.bot.dump()
    t.check(d["pos"][1] > GROUND + 3.5, f"rose to y {d['pos'][1]}")
    # Sprint-swim.
    s.send(f"tp {t.bot.name} 0.5 {GROUND + 2} -3.5 0 0")
    t.bot.ticks(6)
    t.bot.key("KeyW", True)
    t.bot.key("ControlLeft", True)
    t.bot.ticks(15)
    d = t.bot.dump()
    t.check(d["swimming"], "swimming")
    t.check(d["pose"] == "Swimming", f"pose {d['pose']}")
    t.bot.key("KeyW", False)
    t.bot.key("ControlLeft", False)
    t.bot.ticks(5)
    s.send(f"fill -6 {GROUND} -6 6 {GROUND + 5} 6 air")
    s.send(f"tp {t.bot.name} 0.5 {GROUND} 0.5")
    t.bot.ticks(10)


@scenario
def give_shows_up_in_the_inventory(t):
    arena(t)
    t.server.send(f"give {t.bot.name} minecraft:diamond 5")
    t.server.send(f"give {t.bot.name} minecraft:command_block")
    t.bot.ticks(6)
    d = t.bot.dump()
    first = inventory_item(d, 36)
    t.check(first is not None and first["item"] == "diamond" and first["count"] == 5, f"hotbar slot 1 holds {first}")
    second = inventory_item(d, 37)
    t.check(second is not None and second["item"] == "command_block", f"hotbar slot 2 holds {second}")
    t.check(first["name"] == "Diamond", f"called {first['name']}")
    t.bot.capture("hotbar")


@scenario
def items_with_every_kind_of_data_decode(t):
    arena(t)
    b = t.bot.name
    specials = [
        "diamond_sword[enchantments={'minecraft:sharpness':5},custom_name='Blade',lore=['one','two'],damage=7,unbreakable={}]",
        "leather_chestplate[dyed_color=255]",
        "potion[potion_contents={potion:'minecraft:strength'}]",
        "splash_potion[potion_contents={custom_color:16711680,custom_effects:[{id:'minecraft:speed',duration:100,amplifier:1}]}]",
        "enchanted_book[stored_enchantments={'minecraft:mending':1}]",
        "written_book[written_book_content={title:'T',author:'A',pages:['page one','page two'],resolved:true}]",
        "writable_book[writable_book_content={pages:['draft']}]",
        "firework_rocket[fireworks={flight_duration:2,explosions:[{shape:'star',colors:[255],fade_colors:[65280],has_trail:true,has_twinkle:true}]}]",
        "firework_star[firework_explosion={shape:'burst',colors:[255]}]",
        "player_head[profile={name:'Notch'}]",
        "shield[banner_patterns=[{pattern:'minecraft:stripe_top',color:'red'}],base_color='blue']",
        "white_banner[banner_patterns=[{pattern:'minecraft:cross',color:'black'}]]",
        "shulker_box[container=[{slot:0,item:{id:'minecraft:stone',count:3}},{slot:5,item:{id:'minecraft:dirt',count:1}}]]",
        "bundle[bundle_contents=[{id:'minecraft:apple',count:2}]]",
        "crossbow[charged_projectiles=[{id:'minecraft:arrow',count:1}]]",
        "iron_chestplate[trim={material:'minecraft:gold',pattern:'minecraft:coast'}]",
        "suspicious_stew[suspicious_stew_effects=[{id:'minecraft:night_vision',duration:100}]]",
        "goat_horn[instrument='minecraft:ponder_goat_horn']",
        "music_disc_cat",
        "painting[painting/variant='minecraft:kebab']",
        "compass[lodestone_tracker={target:{dimension:'minecraft:overworld',pos:[1,2,3]},tracked:false}]",
        "decorated_pot[pot_decorations=['minecraft:brick','minecraft:angler_pottery_sherd','minecraft:brick','minecraft:brick']]",
        "beehive[bees=[{entity_data:{id:'minecraft:bee'},ticks_in_hive:1,min_ticks_in_hive:2}]]",
        "wolf_spawn_egg[entity_data={id:'minecraft:wolf'}]",
        "oak_sign[block_entity_data={id:'minecraft:sign'}]",
        "tropical_fish_bucket[tropical_fish/pattern='flopper',tropical_fish/base_color='red',tropical_fish/pattern_color='blue']",
        "stone[max_stack_size=3,rarity='epic',custom_model_data={floats:[1.5],flags:[true],strings:['x'],colors:[1]},item_model='minecraft:dirt',enchantment_glint_override=true]",
        "apple[food={nutrition:2,saturation:1,can_always_eat:true},consumable={consume_seconds:1,animation:'eat',has_consume_particles:false,on_consume_effects:[{type:'minecraft:clear_all_effects'}]},use_cooldown={seconds:1,cooldown_group:'minecraft:x'},use_remainder={id:'minecraft:bowl'}]",
        "stick[tool={rules:[{blocks:'#minecraft:mineable/axe',speed:9,correct_for_drops:true},{blocks:['stone','dirt'],speed:2}],default_mining_speed:1.5,damage_per_block:2},weapon={item_damage_per_attack:2},attribute_modifiers=[{type:'minecraft:attack_damage',amount:3,id:'minecraft:x',operation:'add_value',slot:'mainhand'}]]",
        "elytra[equippable={slot:'chest',equip_sound:'minecraft:item.armor.equip_generic'},glider={},repairable={items:'minecraft:phantom_membrane'},death_protection={death_effects:[{type:'minecraft:play_sound',sound:'minecraft:entity.pig.ambient'}]}]",
        "stone_pickaxe[can_break={blocks:'stone'},can_place_on={blocks:['dirt','grass_block']},tooltip_display={hidden_components:['minecraft:unbreakable']},tooltip_style='minecraft:x']",
        "shield[blocks_attacks={block_delay_seconds:0.5,disable_cooldown_scale:1,damage_reductions:[{base:0,factor:0.5}],item_damage:{threshold:1,base:0,factor:1}}]",
        "bee_nest[block_state={honey_level:'3'}]",
        "filled_map[map_id=1,map_post_processing=1,map_color=255]",
    ]
    t.server.drain()
    for item in specials:
        t.server.send(f"give {b} {item}")
    t.bot.ticks(20)
    bad = [l for l in t.server.lines.all[-len(specials) * 3:] if "Unknown" in l or "Expected" in l or "Malformed" in l or "<--[HERE]" in l or "Failed" in l]
    t.note(f"server rejected {len(bad)} of {len(specials)} test items: {bad[:2]}" if bad else f"server accepted all {len(specials)} test items")
    d = t.bot.dump()
    t.check(len(d["slots"]) >= 30, f"only {len(d['slots'])} stacks arrived")
    t.check(not t.bot.problems(), f"decode problems: {t.bot.problems()[:3]}")


@scenario
def every_item_in_the_game_decodes(t):
    arena(t)
    b = t.bot.name
    data = t.item_names()
    seen = set()
    for i in range(0, len(data), 30):
        batch = data[i : i + 30]
        t.server.send(f"clear {b}")
        for name in batch:
            t.server.send(f"give {b} minecraft:{name}")
        t.bot.ticks(8)
        d = t.bot.dump()
        seen.update(s["item"] for s in d["slots"])
    missing = [n for n in data if n not in seen and n not in ("air",)]
    t.check(not t.bot.problems(), f"decode problems: {t.bot.problems()[:3]}")
    t.check(len(missing) < 5, f"{len(missing)} items never showed up: {missing[:20]}")


@scenario
def attacking_hurts_a_mob(t):
    arena(t)
    s = t.server
    s.send("summon minecraft:zombie 0.5 %d 2.5 {NoAI:1b,Health:20f,PersistenceRequired:1b}" % GROUND)
    t.bot.ticks(10)
    d = t.bot.dump()
    z = [e for e in d["entities"] if e["kind"] == "zombie"]
    t.check(z, "the client knows the zombie")
    t.bot.look(0, 0)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["target_entity"] == z[0]["id"], f"crosshair on {d['target_entity']}, block {d['target_block']}")
    t.bot.click_mouse(0)
    t.bot.ticks(10)
    answer = s.ask("data get entity @e[type=zombie,limit=1] Health", "has the following")
    health = float(re.findall(r"(-?[\d.]+)f", answer)[0])
    t.check(health < 20, f"the zombie's health is {health}")
    d = t.bot.dump()
    z = [e for e in d["entities"] if e["kind"] == "zombie"][0]
    t.check(z["hurt_ticks"] < 100, f"the client saw it hurt: {z['hurt_ticks']}")


@scenario
def using_a_villager_opens_trading(t):
    arena(t)
    s = t.server
    s.send(
        "summon minecraft:villager 0.5 %d 2.5 {NoAI:1b,VillagerData:{profession:'minecraft:farmer',level:2,type:'minecraft:plains'},Offers:{Recipes:[{buy:{id:'minecraft:emerald',count:1},sell:{id:'minecraft:bread',count:3},maxUses:10}]}}" % GROUND
    )
    t.bot.ticks(10)
    t.bot.look(0, 0)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["target_entity"] is not None, "crosshair on the villager")
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    d = t.bot.dump()
    t.check(d["menu"] == "merchant", f"menu is {d['menu']}")
    t.bot.capture("villager")
    t.bot.press("KeyE")
    t.bot.ticks(5)
    t.check(t.bot.dump()["menu"] is None, "closed")


@scenario
def chest_opens_and_items_move(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 chest[facing=north]")
    s.send(f"item replace block 0 {GROUND} 2 container.0 with minecraft:emerald 7")
    s.send(f"give {t.bot.name} minecraft:gold_ingot 4")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["target_block"] == "chest", f"looking at {d['target_block']}")
    t.bot.click_mouse(1)
    t.bot.ticks(8)
    d = t.bot.dump()
    t.check(d["menu"] == "generic_9x3", f"menu is {d['menu']}")
    t.check(inventory_item(d, 0)["item"] == "emerald", "the emeralds are in the chest")
    t.check(inventory_item(d, 54)["item"] == "gold_ingot", f"the gold is in the hotbar: {d['slots']}")
    t.bot.capture("chest")
    # Move the gold into the chest by clicking, as a person would.
    t.bot.click("slot:54")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["cursor"] is not None and d["cursor"]["item"] == "gold_ingot", f"holding {d['cursor']}")
    t.bot.click("slot:1")
    t.bot.ticks(4)
    d = t.bot.dump()
    t.check(d["cursor"] is None, f"still holding {d['cursor']}")
    answer = s.ask(f"data get block 0 {GROUND} 2 Items", "has the following")
    t.check("gold_ingot" in answer, f"the server's chest has: {answer}")
    # Shift-click the emeralds into the inventory.
    t.bot.click("slot:0", shift=True)
    t.bot.ticks(4)
    answer = s.ask(f"data get block 0 {GROUND} 2 Items", "has the following")
    t.check("emerald" not in answer, f"shift-click moved them: {answer}")
    d = t.bot.dump()
    t.check(d["cursor"] is None, f"shift-click left {d['cursor']} in hand")
    t.check(any(x["item"] == "emerald" and x["slot"] >= 27 for x in d["slots"]), f"the emeralds came over: {d['slots']}")
    t.bot.press("KeyE")
    t.bot.ticks(4)
    t.check(t.bot.dump()["menu"] is None, "closed")
    s.send(f"setblock 0 {GROUND} 2 air")


@scenario
def crafting_table_crafts(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 crafting_table")
    s.send(f"give {t.bot.name} minecraft:oak_log 1")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(8)
    d = t.bot.dump()
    t.check(d["menu"] == "crafting", f"menu is {d['menu']}")
    # Hotbar slot 1 is slot 37 here: 10 crafting slots, 27 main, then the hotbar.
    t.bot.click("slot:37")
    t.bot.ticks(3)
    t.bot.click("slot:1")
    t.bot.ticks(6)
    d = t.bot.dump()
    result = inventory_item(d, 0)
    t.check(result is not None and result["item"] == "oak_planks", f"the result slot holds {result}")
    t.bot.capture("crafting")
    t.bot.click("slot:0", shift=True)
    t.bot.ticks(6)
    d = t.bot.dump()
    t.check(d["cursor"] is None, f"shift-click left {d['cursor']} in hand")
    t.check(any(x["item"] == "oak_planks" and x["slot"] > 9 for x in d["slots"]), f"shift-click moved the planks: {d['slots']}")
    t.bot.press("KeyE")
    t.bot.ticks(5)
    answer = s.ask(f"execute if items entity {t.bot.name} container.* minecraft:oak_planks run say has planks", "has planks")
    t.check("has planks" in answer, answer)


@scenario
def inventory_screen_moves_items(t):
    arena(t)
    s = t.server
    s.send(f"give {t.bot.name} minecraft:iron_ingot 10")
    t.bot.ticks(6)
    t.bot.press("KeyE")
    t.bot.ticks(5)
    d = t.bot.dump()
    t.check(d["own_screen"], "the inventory is open")
    t.bot.capture("inventory")
    t.bot.click("slot:36")
    t.bot.ticks(3)
    t.bot.click("slot:9")
    t.bot.ticks(5)
    t.bot.press("KeyE")
    t.bot.ticks(5)
    answer = s.ask(f"execute if items entity {t.bot.name} inventory.0 minecraft:iron_ingot run say moved", "moved", timeout=3)
    t.check("moved" in answer, answer)


@scenario
def the_arm_keeps_moving_while_mining(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 obsidian")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.button(0, True)
    seen = []
    end = time.time() + 1.5
    while time.time() < end:
        seen.append(t.bot.dump()["swing"])
        time.sleep(0.02)
    t.bot.button(0, False)
    # Restarting the swing every tick left it stuck near the start of its motion.
    t.check(max(seen) >= 0.5, f"the arm only got to {max(seen):.2f}: {seen[:20]}")
    t.check(len({round(v, 2) for v in seen}) >= 4, f"the arm barely moves: {seen[:20]}")
    t.bot.ticks(12)
    t.check(t.bot.dump()["swing"] == 0.0, "the arm comes to rest once the button's let go")


@scenario
def blocks_placed_show_at_once_and_a_refused_one_goes_away(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:stone 16")
    # Grass that's been covered up by an earlier scenario turns to dirt.
    s.send(f"setblock 0 {GROUND - 1} 2 grass_block")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["target_block"] == "grass_block", f"looking at {d['target_block']}")
    t.bot.click_mouse(1)
    t.bot.ticks(1)
    t.check(t.bot.dump()["target_block"] == "stone", "the placed block shows straight away")
    answer = s.ask(f"execute if block 0 {GROUND} 2 stone run say placed", "placed", timeout=2)
    t.check("placed" in answer, "the server agrees it's there")
    s.send(f"setblock 0 {GROUND} 2 air")
    # Outside the world border the server refuses, and the client's guess has to go.
    s.send("worldborder center 0.5 0.5")
    s.send("worldborder set 3")
    t.bot.ticks(10)
    try:
        t.bot.click_mouse(1)
        t.bot.ticks(10)
        d = t.bot.dump()
        t.check(d["target_block"] == "grass_block", f"a refused block stays on screen as {d['target_block']}")
    finally:
        s.send("worldborder set 59999968")
        s.send("worldborder center 0 0")
    t.bot.ticks(4)


@scenario
def the_inventory_key_opens_and_closes_it(t):
    arena(t)
    t.bot.press("KeyE", "e")
    t.bot.ticks(10)
    t.check(t.bot.dump()["own_screen"], "it stays open after the key that opened it")
    t.bot.press("KeyE", "e")
    t.bot.ticks(6)
    t.check(not t.bot.dump()["own_screen"], "the same key closes it")


@scenario
def typing_in_a_menu_doesnt_close_it(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 anvil")
    s.send(f"give {t.bot.name} minecraft:diamond_sword")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["menu"] == "anvil", "anvil open")
    t.bot.click("rename")
    t.bot.press("KeyE", "e")
    t.bot.ticks(6)
    t.check(t.bot.dump()["menu"] == "anvil", "typing the inventory key's letter leaves it open")
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send(f"setblock 0 {GROUND} 2 air")
    s.send(f"clear {t.bot.name}")
    t.bot.ticks(4)


@scenario
def a_long_list_of_trades_can_be_scrolled_and_used(t):
    arena(t)
    s = t.server
    items = ["bookshelf", "lantern", "paper", "clock", "compass", "name_tag", "glass", "ink_sac", "feather", "arrow", "bow", "shield"]
    recipes = ",".join(f"{{buy:{{id:'minecraft:emerald',count:{n + 1}}},sell:{{id:'minecraft:{i}',count:1}},maxUses:9}}" for n, i in enumerate(items))
    s.send(f"summon minecraft:villager 0.5 {GROUND} 2.5 {{NoAI:1b,VillagerData:{{profession:'minecraft:librarian',level:2,type:'minecraft:plains'}},Offers:{{Recipes:[{recipes}]}}}}")
    s.send(f"give {t.bot.name} minecraft:emerald 40")
    t.bot.ticks(10)
    t.bot.look(0, 0)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["menu"] == "merchant", "trading open")
    t.check(not t.bot.has_widget("trade:11"), "the last trade doesn't fit at first")
    # The wheel over the list brings it into view.
    box = t.bot.widget("trade:0")
    t.bot.cmd(t="pointer", x=(box[0] + box[2]) / 2, y=(box[1] + box[3]) / 2)
    t.bot.ticks(2)
    for _ in range(12):
        t.bot.cmd(t="wheel", delta=1)
        t.bot.ticks(1)
    t.bot.ticks(3)
    t.check(t.bot.has_widget("trade:11"), "the last trade scrolls into view")
    t.bot.click("trade:11")
    t.bot.ticks(8)
    # Choosing it moves the payment across, and taking the result pays for it.
    t.bot.click("slot:2", shift=True)
    t.bot.ticks(8)
    answer = s.ask(f"execute if items entity {t.bot.name} container.* minecraft:shield run say bought", "bought", timeout=3)
    t.check("bought" in answer, "the last trade was bought")
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send("kill @e[type=villager]")
    s.send(f"clear {t.bot.name}")
    t.bot.ticks(4)


@scenario
def a_crafter_lines_up_and_its_slots_switch_off(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 crafter")
    s.send(f"clear {t.bot.name}")
    s.send(f"item replace entity {t.bot.name} hotbar.0 with minecraft:oak_planks 3")
    t.bot.ticks(8)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["menu"] == "crafter_3x3", "crafter open")
    # The first hotbar slot is 36 in this menu, as it is in any other with nine of its own.
    t.bot.click("slot:36")
    t.bot.ticks(4)
    t.check((t.bot.dump().get("cursor") or {}).get("count") == 3, "the hotbar shows where it really is")
    t.bot.click("slot:0")
    t.bot.ticks(6)
    answer = s.ask(f"execute if items block 0 {GROUND} 2 container.0 minecraft:oak_planks run say placed", "placed", timeout=2)
    t.check("placed" in answer, "planks went in the crafter's first slot")
    # An empty slot pressed with nothing in hand switches off.
    t.bot.click("slot:4")
    t.bot.ticks(8)
    got = s.ask(f"data get block 0 {GROUND} 2 disabled_slots", "data", timeout=2)
    t.check("4" in got, f"the server has that slot off: {got}")
    t.bot.click("slot:4")
    t.bot.ticks(8)
    got = s.ask(f"data get block 0 {GROUND} 2 disabled_slots", "data", timeout=2)
    t.check("4" not in got.split("data")[-1], f"and on again: {got}")
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send(f"setblock 0 {GROUND} 2 air")
    s.send(f"clear {t.bot.name}")
    t.bot.ticks(4)


def game_master(t):
    s = t.server
    s.send(f"op {t.bot.name}")
    s.send(f"gamemode creative {t.bot.name}")
    t.bot.ticks(10)


def not_game_master(t):
    """Survival, as every scenario starts. They run as an operator, so that stays."""
    t.server.send(f"gamemode survival {t.bot.name}")
    t.bot.ticks(8)


@scenario
def command_blocks_can_be_edited_and_run(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 command_block")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(8)
    t.check(t.bot.dump()["modal"] is None, "not in survival")
    s.send(f"deop {t.bot.name}")
    s.send(f"gamemode creative {t.bot.name}")
    t.bot.ticks(8)
    t.bot.click_mouse(1)
    t.bot.ticks(8)
    t.check(t.bot.dump()["modal"] is None, "not for players who aren't allowed")
    game_master(t)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "block", f"the screen opens: {t.bot.dump()['modal']}")
    t.bot.capture("command-block")
    t.bot.cmd(t="type", text=f"setblock 3 {GROUND} 3 gold_block")
    t.bot.ticks(3)
    t.bot.click("Triggered by:Always Active")
    t.bot.click("Done")
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] is None, "it closes")
    got = s.ask(f"data get block 0 {GROUND} 2 Command", "gold_block", timeout=3)
    t.check("gold_block" in got, f"the command reached the block: {got}")
    answer = s.ask(f"execute if block 3 {GROUND} 3 gold_block run say ran", "ran", timeout=3)
    t.check("ran" in answer, "and it ran by itself, being always active")
    # Opening it again shows what's in it.
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.has_widget("Console Command"), "the field's there")
    t.bot.press("Escape")
    t.bot.ticks(6)
    t.check(t.bot.dump()["modal"] is None, "escape leaves without changing it")
    s.send(f"setblock 0 {GROUND} 2 air")
    s.send(f"setblock 3 {GROUND} 3 air")
    not_game_master(t)


@scenario
def minecart_command_blocks_can_be_edited(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 rail")
    s.send(f"summon minecraft:command_block_minecart 0.5 {GROUND} 2.5 {{CustomName:'editable'}}")
    game_master(t)
    t.bot.look(0, 25)
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "block", f"the screen opens: {t.bot.dump()['modal']}")
    t.bot.cmd(t="type", text="say from a cart")
    t.bot.ticks(3)
    t.bot.click("Done")
    t.bot.ticks(8)
    got = s.ask("data get entity @e[type=command_block_minecart,limit=1] Command", "from a cart", timeout=3)
    t.check("from a cart" in got, f"the minecart took it: {got}")
    s.send("kill @e[type=command_block_minecart]")
    s.send(f"setblock 0 {GROUND} 2 air")
    not_game_master(t)


@scenario
def structure_blocks_save_and_load(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 structure_block[mode=save]")
    s.send(f"setblock 0 {GROUND + 1} 3 diamond_block")
    game_master(t)
    t.bot.look(0, 25)
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "block", "the screen opens")
    t.bot.capture("structure-block")
    t.bot.click("Structure Name")
    t.bot.cmd(t="type", text="lattice_test")
    t.bot.ticks(2)
    for axis, value in zip("xyz", ("1", "2", "1")):
        t.bot.click(f"Structure Size {axis}")
        t.bot.cmd(t="ui_key", key="Backspace")
        t.bot.cmd(t="type", text=value)
        t.bot.ticks(2)
    t.bot.click("SAVE")
    t.bot.ticks(10)
    got = s.ask(f"data get block 0 {GROUND} 2 name", "lattice_test", timeout=3)
    t.check("lattice_test" in got, f"the block took its name: {got}")
    s.send(f"setblock 0 {GROUND} 2 air")
    s.send(f"setblock 0 {GROUND + 1} 3 air")
    not_game_master(t)


@scenario
def jigsaw_and_test_blocks_take_what_you_set(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 jigsaw")
    game_master(t)
    t.bot.look(0, 25)
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "block", "the jigsaw screen opens")
    t.bot.capture("jigsaw")
    t.bot.click("Selection Priority")
    t.bot.cmd(t="ui_key", key="Backspace")
    t.bot.cmd(t="type", text="7")
    t.bot.ticks(2)
    t.bot.click("Done")
    t.bot.ticks(8)
    got = s.ask(f"data get block 0 {GROUND} 2 selection_priority", "following", timeout=3)
    t.check(got.rstrip().endswith("7"), f"priority stored: {got}")
    s.send(f"setblock 0 {GROUND} 2 test_block")
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "block", "the test block screen opens")
    t.bot.click("Message")
    t.bot.cmd(t="type", text="hello")
    t.bot.ticks(2)
    t.bot.click("Done")
    t.bot.ticks(8)
    got = s.ask(f"data get block 0 {GROUND} 2 message", "hello", timeout=3)
    t.check("hello" in got, f"message stored: {got}")
    s.send(f"setblock 0 {GROUND} 2 air")
    not_game_master(t)


@scenario
def game_rules_and_game_modes_can_be_changed(t):
    arena(t)
    s = t.server
    s.send(f"deop {t.bot.name}")
    t.bot.ticks(8)
    t.bot.press("Escape")
    t.bot.ticks(6)
    t.check(not t.bot.has_widget("Game rules"), "only for those allowed")
    t.bot.press("Escape")
    t.bot.ticks(4)
    game_master(t)
    t.bot.press("Escape")
    t.bot.ticks(6)
    t.bot.click("Game rules")
    t.bot.ticks(10)
    t.check(t.bot.dump()["modal"] == "game_rules", "the screen opens")
    t.bot.capture("game-rules")
    t.bot.click("Done")
    t.bot.ticks(6)
    t.check(t.bot.dump()["modal"] is None, "it closes")
    # Holding the details key, F4 offers the game modes and letting go picks.
    t.bot.key("F3", True)
    t.bot.ticks(2)
    t.bot.press("F4")
    t.bot.ticks(4)
    t.check(t.bot.dump()["modal"] == "game_mode", "the game mode list opens")
    t.bot.capture("game-mode")
    t.bot.press("F4")
    t.bot.ticks(2)
    t.bot.key("F3", False)
    t.bot.ticks(8)
    d = t.bot.dump()
    t.check(d["modal"] is None, "letting go closes it")
    t.check(d["game_mode"] == 2, f"it moved on from creative to adventure, not {d['game_mode']}")
    t.bot.key("F3", True)
    t.bot.ticks(2)
    t.bot.press("KeyN")
    t.bot.ticks(2)
    t.bot.key("F3", False)
    t.bot.ticks(8)
    t.check(t.bot.dump()["game_mode"] == 3, "F3 and N makes you a spectator")
    t.check(not t.bot.dump()["debug"], "shortcuts don't leave the details open")
    not_game_master(t)


@scenario
def leaving_the_end_shows_the_story_and_comes_home(t):
    arena(t)
    s = t.server
    s.send(f"op {t.bot.name}")
    s.send(f"effect give {t.bot.name} minecraft:slow_falling 60 0 true")
    s.send(f"execute in minecraft:the_end run tp {t.bot.name} 100.5 64 100.5")
    # A floor to stand on, once that part of the End has been made.
    for _ in range(40):
        time.sleep(0.5)
        answer = s.ask("execute in minecraft:the_end run fill 95 60 95 105 63 105 obsidian", "", timeout=2)
        if "not loaded" not in answer:
            break
    s.send("execute in minecraft:the_end run fill 95 64 95 105 70 105 air")
    s.send(f"execute in minecraft:the_end run tp {t.bot.name} 100.5 64 100.5")
    t.bot.ticks(20)
    t.check(t.bot.dump()["dimension"] == "minecraft:the_end", f"in the End: {t.bot.dump()['dimension']}")
    s.send("execute in minecraft:the_end run setblock 100 64 102 end_portal")
    t.bot.ticks(4)
    t.bot.look(0, 0)
    t.bot.key("KeyW", True)
    t.bot.ticks(14)
    t.bot.key("KeyW", False)
    t.bot.ticks(10)
    d = t.bot.dump()
    t.check(d["modal"] == "credits", f"the story starts: {d['modal']}")
    t.bot.capture("credits")
    t.bot.click("Skip")
    t.bot.ticks(40)
    d = t.bot.dump()
    t.check(d["modal"] is None, "skipping it ends it")
    t.check(d["dimension"] == "minecraft:overworld", f"and it comes home: {d['dimension']}")
    t.bot.ticks(4)


@scenario
def bundles_show_what_they_hold(t):
    arena(t)
    s = t.server
    s.send(f"clear {t.bot.name}")
    s.send(f"item replace entity {t.bot.name} hotbar.0 with minecraft:bundle[bundle_contents=[{{id:'minecraft:diamond',count:3}},{{id:'minecraft:apple',count:5}}]]")
    t.bot.ticks(8)
    t.bot.press("KeyE", "e")
    t.bot.ticks(6)
    box = t.bot.widget("slot:36")
    t.bot.cmd(t="pointer", x=(box[0] + box[2]) / 2, y=(box[1] + box[3]) / 2)
    t.bot.ticks(3)
    t.bot.capture("bundle-tooltip")
    t.bot.cmd(t="wheel", delta=-1)
    t.bot.ticks(3)
    t.bot.capture("bundle-picked")
    # Pressing the right button takes out the one that's picked.
    t.bot.click("slot:36", button=1)
    t.bot.ticks(6)
    t.check((t.bot.dump().get("cursor") or {}).get("count") in (3, 5), "something came out of the bundle")
    t.bot.click("slot:9")
    t.bot.ticks(4)
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send(f"clear {t.bot.name}")
    t.bot.ticks(4)


@scenario
def getting_out_of_bed(t):
    arena(t)
    s = t.server
    s.send("time set 13000")
    s.send("gamerule players_sleeping_percentage 101")
    s.send(f"setblock 0 {GROUND} 2 white_bed[facing=south,part=foot]")
    s.send(f"setblock 0 {GROUND} 3 white_bed[facing=south,part=head]")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(20)
    t.check(t.bot.has_widget("Leave bed"), "there's a way out")
    t.check(not t.bot.dump()["captured"], "and a pointer to press it with")
    t.bot.capture("in-bed")
    t.bot.click("Leave bed")
    t.bot.ticks(12)
    t.check(not t.bot.has_widget("Leave bed"), "it gets you up")
    t.check(t.bot.dump()["captured"], "and the mouse looks around again")
    s.send("time set 1000")
    s.send(f"fill 0 {GROUND} 2 0 {GROUND} 3 air")
    t.bot.ticks(4)


@scenario
def armour_shows_on_your_character_and_a_totem_saves_you(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} armor.head with minecraft:diamond_helmet")
    s.send(f"item replace entity {t.bot.name} armor.chest with minecraft:iron_chestplate")
    s.send(f"item replace entity {t.bot.name} armor.legs with minecraft:golden_leggings")
    s.send(f"item replace entity {t.bot.name} armor.feet with minecraft:leather_boots[dyed_color=11546150]")
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:compass")
    s.send(f"item replace entity {t.bot.name} weapon.offhand with minecraft:totem_of_undying")
    t.bot.ticks(8)
    t.bot.press("KeyE", "e")
    t.bot.ticks(6)
    t.bot.capture("armour-on-character")
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send(f"damage {t.bot.name} 100 minecraft:generic")
    t.bot.ticks(6)
    t.bot.capture("totem-saved")
    d = t.bot.dump()
    t.check(not d["dead"], "the totem kept them alive")
    t.bot.ticks(40)
    s.send(f"clear {t.bot.name}")
    s.send(f"effect clear {t.bot.name}")
    t.bot.ticks(4)


@scenario
def hotbars_can_be_saved_and_loaded_in_creative(t):
    arena(t)
    s = t.server
    s.send(f"gamemode creative {t.bot.name}")
    s.send(f"item replace entity {t.bot.name} hotbar.3 with minecraft:emerald 12")
    t.bot.ticks(8)
    t.bot.key("KeyC", True)
    t.bot.ticks(2)
    t.bot.press("Digit2")
    t.bot.ticks(2)
    t.bot.key("KeyC", False)
    t.bot.ticks(3)
    s.send(f"clear {t.bot.name}")
    t.bot.ticks(6)
    t.bot.key("KeyX", True)
    t.bot.ticks(2)
    t.bot.press("Digit2")
    t.bot.ticks(2)
    t.bot.key("KeyX", False)
    t.bot.ticks(8)
    answer = s.ask(f"execute if items entity {t.bot.name} hotbar.3 minecraft:emerald run say restored", "restored", timeout=3)
    t.check("restored" in answer, "the saved hotbar came back")
    t.bot.press("KeyE", "e")
    t.bot.ticks(5)
    t.bot.click("tab:hotbar")
    t.bot.ticks(4)
    t.check(t.bot.has_widget("saved:1:3"), "and it's listed on its page")
    t.bot.capture("saved-hotbars")
    t.bot.press("Escape")
    t.bot.ticks(4)
    s.send(f"clear {t.bot.name}")
    s.send(f"gamemode survival {t.bot.name}")
    t.bot.ticks(4)


@scenario
def spectators_can_follow_a_mob(t):
    arena(t)
    s = t.server
    s.send(f"summon minecraft:pig 0.5 {GROUND} 2.5 {{NoAI:1b}}")
    s.send(f"gamemode spectator {t.bot.name}")
    t.bot.ticks(10)
    # A pig is short: at 15 degrees the view only just passes over its back.
    t.bot.look(0, 32)
    t.bot.ticks(6)
    t.check(t.bot.dump()["target_entity"] is not None, "crosshair on the pig")
    t.bot.click_mouse(0)
    t.bot.ticks(10)
    answer = s.ask(f"execute as {t.bot.name} on spectating run say following", "following", timeout=2)
    t.check("following" in answer, f"the server has them following it: {answer}")
    s.send(f"spectate")
    s.send(f"gamemode survival {t.bot.name}")
    s.send("kill @e[type=pig]")
    t.bot.ticks(6)


@scenario
def breaking_with_a_tool_is_faster(t):
    arena(t)
    s = t.server
    s.send(f"setblock 0 {GROUND} 2 stone")
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:iron_pickaxe")
    t.bot.ticks(6)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    start = time.time()
    t.bot.button(0, True)
    while time.time() - start < 3:
        if t.bot.dump()["target_block"] != "stone":
            break
        time.sleep(0.05)
    took = time.time() - start
    t.bot.button(0, False)
    t.bot.ticks(5)
    # Stone takes 0.4 s with iron and 7.5 s by hand.
    t.check(took < 1.2, f"took {took:.2f} s")
    answer = s.ask(f"execute if block 0 {GROUND} 2 air run say gone", "gone", timeout=2)
    t.check("gone" in answer, answer)


@scenario
def placing_a_block_in_hand(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:cobblestone 8")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    t.bot.ticks(6)
    answer = s.ask(f"execute if blocks 0 {GROUND - 5} 0 0 {GROUND + 5} 5 0 {GROUND - 5} 0 masked run say x", "", timeout=1) if False else ""
    found = False
    for z in range(0, 5):
        s.drain()
        s.send(f"execute if block 0 {GROUND} {z} cobblestone run say placed {z}")
        try:
            s.wait_line("placed", 0.5)
            found = True
            break
        except Exception:
            pass
    t.check(found, "the block appeared in front of the player")


@scenario
def eating_and_dropping(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:apple 4")
    s.send(f"effect give {t.bot.name} minecraft:hunger 5 200 true")
    t.bot.ticks(40)
    food_before = t.bot.dump()["food"]
    t.check(food_before < 20, f"hungry: {food_before}")
    s.send(f"effect clear {t.bot.name}")
    t.bot.look(0, -40)
    t.bot.ticks(3)
    t.bot.button(1, True)
    t.bot.ticks(40)
    t.bot.button(1, False)
    t.bot.ticks(5)
    d = t.bot.dump()
    t.check(d["food"] > food_before, f"ate: food {food_before} to {d['food']}")
    t.check(inventory_item(d, 36)["count"] == 3, f"apples left: {inventory_item(d, 36)}")
    # Q drops one.
    t.bot.press("KeyQ")
    t.bot.ticks(6)
    d = t.bot.dump()
    t.check(inventory_item(d, 36)["count"] == 2, f"after dropping: {inventory_item(d, 36)}")
    s.send("kill @e[type=item]")


@scenario
def creative_mode_flies_and_breaks_instantly(t):
    arena(t)
    s = t.server
    s.send(f"gamemode creative {t.bot.name}")
    s.send(f"setblock 0 {GROUND} 2 obsidian")
    t.bot.ticks(8)
    t.bot.look(0, 25)
    t.bot.ticks(3)
    t.bot.click_mouse(0)
    t.bot.ticks(6)
    answer = s.ask(f"execute if block 0 {GROUND} 2 air run say gone", "gone", timeout=2)
    t.check("gone" in answer, answer)
    # Double-tap jump to fly.
    t.bot.press("Space")
    t.bot.ticks(1)
    t.bot.press("Space")
    t.bot.ticks(6)
    d = t.bot.dump()
    t.check(d["flying"], "flying")
    t.bot.key("Space", True)
    t.bot.ticks(10)
    t.bot.key("Space", False)
    d = t.bot.dump()
    t.check(d["pos"][1] > GROUND + 1, f"rose to {d['pos'][1]}")
    s.send(f"gamemode survival {t.bot.name}")
    t.bot.ticks(30)


@scenario
def dying_and_respawning(t):
    arena(t)
    s = t.server
    s.send(f"kill {t.bot.name}")
    t.bot.ticks(10)
    t.bot.click("Respawn")
    t.bot.ticks(30)
    d = t.bot.dump()
    t.check(d["health"] > 0, f"health {d['health']}")
    t.check(d["pos"] is not None, "back in the world")


@scenario
def command_key_opens_chat_with_a_slash_on_any_layout(t):
    arena(t)
    # On a Spanish keyboard `/` is Shift+7, which is the physical key Digit7.
    t.bot.key("Digit7", True, "/")
    t.bot.ticks(2)
    t.bot.key("Digit7", False)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check("chat" in d["overlays"], f"overlays {d['overlays']}")
    t.bot.cmd(t="type", text="tp @s 0 %d 0" % GROUND)
    t.bot.ticks(3)
    t.bot.cmd(t="ui_key", key="Enter")
    t.bot.ticks(8)
    d = t.bot.dump()
    t.check("chat" not in d["overlays"], "chat closed")
    t.check(abs(d["pos"][0]) < 0.8, f"teleported to {d['pos']}")


@scenario
def things_make_sounds(t):
    arena(t, 700, 0)
    t.server.send(f"fill 690 {GROUND - 1} -10 710 {GROUND - 1} 40 stone")
    # Sound files arrive in the background, so the first steps may be silent.
    before = t.bot.dump()["sounds_started"]
    d = None
    for direction in ["KeyW", "KeyS", "KeyW", "KeyS"]:
        t.bot.key(direction, True)
        t.bot.ticks(40)
        t.bot.key(direction, False)
        d = t.wait(lambda d: d["sounds_started"] > before, 3)
        if d["sounds_started"] > before:
            break
    t.check(d["sounds_started"] > before, "walking made no footsteps")
    # Files may still be arriving at first; once they're here every step sounds. Walking 8.6
    # blocks a go is 5 steps at least.
    best = 0
    for direction in ["KeyW", "KeyS", "KeyW", "KeyS", "KeyW", "KeyS"]:
        before = t.bot.dump()["sounds_started"]
        t.bot.key(direction, True)
        t.bot.ticks(40)
        t.bot.key(direction, False)
        t.bot.ticks(3)
        best = max(best, t.bot.dump()["sounds_started"] - before)
        if best >= 3:
            break
    d = t.bot.dump()
    t.check(best >= 3, f"walking 2 seconds gave only {best} sounds")
    t.note(f"{best} sounds from walking, {d['sounds_failed']} files failed")
    # Things the server plays.
    # The first time a file may still be arriving, so it can take a second try.
    heard = False
    for _ in range(4):
        before = d["sounds_started"]
        t.server.send(f"execute at {t.bot.name} run playsound minecraft:entity.experience_orb.pickup master {t.bot.name}")
        d = t.wait(lambda d: d["sounds_started"] > before, 4)
        if d["sounds_started"] > before:
            heard = True
            break
    t.check(heard, f"a sound the server sent didn't play: {d['sounds_started']} started, {d['sounds_failed']} failed")
    t.check(d["sounds_failed"] == 0, f"{d['sounds_failed']} sound files failed to load")
    # Swimming and splashing make noise too.
    s = t.server
    s.send(f"fill 696 {GROUND} 0 704 {GROUND + 2} 8 water")
    before = t.bot.dump()["sounds_started"]
    s.send(f"tp {t.bot.name} 700.5 {GROUND + 4} 4.5")
    d = t.wait(lambda d: d["in_water"], 4)
    t.bot.ticks(5)
    d = t.bot.dump()
    t.check(d["sounds_started"] > before, "going into water made no splash")
    s.send(f"fill 696 {GROUND} 0 704 {GROUND + 2} 8 air")
    t.bot.ticks(5)


@scenario
def signs_take_what_you_write(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:oak_sign 4")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "sign", 3)
    t.check(d["modal"] == "sign", f"placing a sign opens its editor, got {d['modal']}")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="Hello sign")
    t.bot.ticks(3)
    t.bot.cmd(t="ui_key", key="Enter")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="second")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["modal_text"][:2] == ["Hello sign", "second"], f"typed {d['modal_text']}")
    # Far too long for a sign: only as much as fits stays.
    t.bot.cmd(t="ui_key", key="Enter")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="W" * 60)
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(len(d["modal_text"][2]) == 0, f"a line too wide is refused, got {d['modal_text'][2]!r}")
    t.bot.click("Done")
    d = t.wait(lambda d: d["modal"] is None, 3)
    t.check(d["modal"] is None, "the editor closed")
    t.check(d["captured"], "the mouse went back to the game")
    t.bot.ticks(5)
    found = False
    for z in range(0, 6):
        answer = s.ask(f"data get block 0 {GROUND} {z} front_text.messages", "", timeout=1.0)
        if "Hello sign" in answer:
            found = True
            break
    t.check(found, f"the server has the words on the sign: {answer}")
    # Holding a sign and using it on the sign again opens it once more.
    d = t.bot.dump()
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "sign", 2)
    if d["modal"] == "sign":
        t.check(d["modal_text"][0] == "Hello sign", f"reopened with {d['modal_text']}")
        t.bot.press("Escape")
        t.bot.ticks(4)
        d = t.bot.dump()
        t.check(d["modal"] is None and not d["paused"], "Esc closes the editor without pausing")


@scenario
def books_are_written_signed_and_read(t):
    arena(t)
    s = t.server
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:writable_book")
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "write_book", 3)
    t.check(d["modal"] == "write_book", f"a book and quill opens for writing, got {d['modal']}")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="The first page.")
    t.bot.ticks(3)
    t.bot.click("Next")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="The second page.")
    t.bot.ticks(3)
    t.bot.click("Next")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["modal_text"] == ["The first page.", "The second page.", ""], f"pages {d['modal_text']}")
    t.bot.click("Done")
    t.bot.ticks(8)
    answer = s.ask(f"data get entity {t.bot.name} SelectedItem", "has the following", 2.0)
    t.check("The second page." in answer, f"saved on the server: {answer}")
    # Reopened, then signed.
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "write_book", 3)
    t.check(d["modal_text"] == ["The first page.", "The second page."], f"reopened with {d['modal_text']}")
    t.bot.click("Sign")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="My book")
    t.bot.ticks(3)
    t.bot.click("Sign and close")
    t.bot.ticks(8)
    answer = s.ask(f"data get entity {t.bot.name} SelectedItem", "has the following", 2.0)
    t.check("written_book" in answer and "My book" in answer, f"signed on the server: {answer}")
    # A signed book opens for reading when the server says so.
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "book", 3)
    t.check(d["modal"] == "book", f"a signed book opens for reading, got {d['modal']}")
    t.check(d["modal_text"] == ["The first page.", "The second page."], f"reads {d['modal_text']}")
    t.bot.press("Escape")
    t.bot.ticks(3)
    t.check(t.bot.dump()["modal"] is None, "Esc closes the book")


@scenario
def boss_bars_scoreboards_and_teams_show(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send('bossbar add lattice:test "Test bar"')
    s.send(f"bossbar set lattice:test players {name}")
    s.send("bossbar set lattice:test value 50")
    d = t.wait(lambda d: any("0.50" in b for b in d["boss_bars"]), 3)
    t.check(any("Test bar 0.50" in b for b in d["boss_bars"]), f"boss bars {d['boss_bars']}")
    s.send('scoreboard objectives add kills dummy "Kills"')
    s.send("scoreboard objectives setdisplay sidebar kills")
    s.send(f"scoreboard players set {name} kills 3")
    s.send("scoreboard players set Other kills 9")
    d = t.wait(lambda d: len(d["sidebar"]) == 3, 3)
    t.check(len(d["sidebar"]) == 3 and d["sidebar"][0] == "Kills" and d["sidebar"][1] == "Other 9", f"sidebar {d['sidebar']}")
    s.send("team add blue")
    s.send('team modify blue prefix "[B] "')
    s.send("team modify blue color blue")
    s.send(f"team join blue {name}")
    d = t.wait(lambda d: any("[B]" in n for n in d["tab"]), 3)
    t.check(any(n.startswith("[B] " + name) for n in d["tab"]), f"tab {d['tab']}")
    t.bot.capture("bars-and-board")
    s.send("worldborder set 1000")
    d = t.wait(lambda d: d["border_size"] == 1000, 3)
    t.check(d["border_size"] == 1000, f"border {d['border_size']}")
    s.send("worldborder set 59999968")
    s.send("tick rate 40")
    d = t.wait(lambda d: d["tick_rate"] == 40, 3)
    t.check(d["tick_rate"] == 40, f"tick rate {d['tick_rate']}")
    s.send("tick rate 20")
    s.send("bossbar remove lattice:test")
    s.send("scoreboard objectives remove kills")
    s.send("team remove blue")
    d = t.wait(lambda d: not d["boss_bars"] and not d["sidebar"], 3)
    t.check(not d["boss_bars"] and not d["sidebar"], f"cleared: {d['boss_bars']} {d['sidebar']}")


def server_pos(t, selector):
    answer = t.server.ask(f"data get entity {selector} Pos", "has the following", 2.0)
    return [float(v) for v in re.findall(r"(-?[\d.]+)d", answer)][:3]


@scenario
def boats_paddle_turn_and_stop(t):
    x, z = 800, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"fill {x - 30} {GROUND - 2} {z - 30} {x + 30} {GROUND - 2} {z + 60} stone")
    s.send(f"fill {x - 30} {GROUND - 1} {z - 30} {x + 30} {GROUND - 1} {z + 60} water")
    s.send(f"summon oak_boat {x + 0.5} {GROUND - 0.5} {z + 0.5} {{Rotation:[0f,0f]}}")
    t.bot.ticks(10)
    s.send(f"ride {name} mount @e[type=oak_boat,limit=1,sort=nearest]")
    d = t.wait(lambda d: d["vehicle"] == "oak_boat", 3)
    t.check(d["vehicle"] == "oak_boat", f"sitting in the boat, in {d['vehicle']}")
    d = t.wait(lambda d: d["driving"] == "Boat", 3)
    t.check(d["driving"] == "Boat", f"steering it, {d['driving']}")
    t.bot.ticks(20)
    d = t.bot.dump()
    start = d["vehicle_pos"]
    # It settles on the water, with the seat just above it.
    t.check(GROUND - 1.1 < start[1] < GROUND - 0.2, f"floats at y {start[1]}")
    t.check(abs(d["pos"][0] - start[0]) < 0.01 and abs(d["pos"][2] - start[2]) < 0.01, f"seated in it: {d['pos']} on {start}")
    t.bot.look(0, 0)
    t.bot.key("KeyW", True)
    t.bot.ticks(40)
    d = t.bot.dump()
    t.check(d["vehicle_pos"][2] > start[2] + 3.0, f"paddling forward went from z {start[2]} to {d['vehicle_pos'][2]}")
    t.check(abs(d["vehicle_pos"][1] - start[1]) < 0.3, f"stayed on the water: y {d['vehicle_pos'][1]}")
    # Speed in a boat on open water settles close to the known 0.4 blocks a tick.
    a = t.bot.dump()["vehicle_pos"][2]
    t.bot.ticks(20)
    b = t.bot.dump()["vehicle_pos"][2]
    t.check(0.25 < (b - a) / 20 < 0.5, f"cruising at {(b - a) / 20:.3f} blocks a tick")
    t.bot.key("KeyW", False)
    t.bot.ticks(5)
    time.sleep(0.3)
    server = server_pos(t, "@e[type=oak_boat,limit=1]")
    d = t.bot.dump()
    t.check(abs(server[2] - d["vehicle_pos"][2]) < 1.5, f"the server has the boat at z {server[2]}, the client at {d['vehicle_pos'][2]}")
    # Turning takes the view with it.
    before = t.bot.dump()["yaw"]
    t.bot.key("KeyA", True)
    t.bot.ticks(20)
    t.bot.key("KeyA", False)
    d = t.bot.dump()
    t.check(d["yaw"] < before - 10, f"turning left turned the view from {before} to {d['yaw']}")
    # Sneaking gets out.
    t.bot.key("ShiftLeft", True)
    d = t.wait(lambda d: d["vehicle"] is None, 3)
    t.bot.key("ShiftLeft", False)
    t.check(d["vehicle"] is None, f"sneaking got out of the boat, still in {d['vehicle']}")
    t.check(d["driving"] is None, "not steering any more")
    t.bot.ticks(10)
    problems = t.bot.problems()
    t.check(not any("moved wrongly" in p or "moved too quickly" in p for p in problems), f"no complaints: {problems[-3:]}")
    s.send(f"tp {name} {x} {GROUND} {z}")
    s.send(f"kill @e[type=oak_boat]")
    t.bot.ticks(5)


@scenario
def saddled_horses_go_where_you_look_and_jump(t):
    x, z = 900, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"fill {x - 30} {GROUND} {z - 30} {x + 30} {GROUND + 8} {z + 60} air")
    s.send(f"fill {x - 30} {GROUND - 1} {z - 30} {x + 30} {GROUND - 1} {z + 60} stone")
    s.send(
        f"summon horse {x + 0.5} {GROUND} {z + 0.5} "
        '{Tame:1b,equipment:{saddle:{id:"minecraft:saddle",count:1}},'
        'attributes:[{id:"minecraft:movement_speed",base:0.3},{id:"minecraft:jump_strength",base:0.7}]}'
    )
    t.bot.ticks(10)
    s.send(f"ride {name} mount @e[type=horse,limit=1,sort=nearest]")
    d = t.wait(lambda d: d["vehicle"] == "horse", 3)
    t.check(d["vehicle"] == "horse", f"on the horse, on {d['vehicle']}")
    d = t.wait(lambda d: d["driving"] == "Horse", 3)
    t.check(d["driving"] == "Horse", f"steering it, {d['driving']}")
    t.bot.ticks(5)
    t.bot.look(0, 0)
    t.bot.ticks(3)
    start = t.bot.dump()["vehicle_pos"]
    t.check(abs(start[1] - GROUND) < 0.1, f"standing on the ground: {start[1]}")
    t.bot.key("KeyW", True)
    t.bot.ticks(40)
    d = t.bot.dump()
    t.check(d["vehicle_pos"][2] > start[2] + 5.0, f"rode from z {start[2]} to {d['vehicle_pos'][2]}")
    a = d["vehicle_pos"][2]
    t.bot.ticks(20)
    b = t.bot.dump()["vehicle_pos"][2]
    # Speed 0.3 at 0.98 input on normal ground settles at 0.3 * 0.98 / (1 - 0.546).
    t.check(0.6 < (b - a) / 20 < 0.68, f"galloping at {(b - a) / 20:.3f} blocks a tick")
    t.bot.look(90, 0)
    t.bot.ticks(20)
    d2 = t.bot.dump()
    t.check(d2["vehicle_pos"][0] < d["vehicle_pos"][0] - 3, f"goes where you look: x {d['vehicle_pos'][0]} to {d2['vehicle_pos'][0]}")
    t.bot.key("KeyW", False)
    t.bot.ticks(10)
    # Holding jump charges up; letting go leaps.
    t.bot.key("Space", True)
    t.bot.ticks(8)
    d = t.bot.dump()
    t.check(d["jump_scale"] > 0.3, f"jump charging: {d['jump_scale']}")
    y0 = d["vehicle_pos"][1]
    t.bot.key("Space", False)
    top = y0
    for _ in range(14):
        t.bot.ticks(1)
        top = max(top, t.bot.dump()["vehicle_pos"][1])
    t.check(top > y0 + 0.5, f"leapt to {top} from {y0}")
    t.bot.ticks(30)
    d = t.bot.dump()
    t.check(abs(d["vehicle_pos"][1] - GROUND) < 0.1, f"landed: y {d['vehicle_pos'][1]}")
    time.sleep(0.3)
    server = server_pos(t, "@e[type=horse,limit=1]")
    t.check(abs(server[0] - d["vehicle_pos"][0]) < 1.5, f"the server has the horse at x {server[0]}, the client at {d['vehicle_pos'][0]}")
    t.bot.capture("riding-horse")
    t.bot.key("ShiftLeft", True)
    d = t.wait(lambda d: d["vehicle"] is None, 3)
    t.bot.key("ShiftLeft", False)
    t.check(d["vehicle"] is None, f"got off, still on {d['vehicle']}")
    s.send("kill @e[type=horse]")
    t.bot.ticks(5)


@scenario
def pigs_follow_the_carrot_and_unsaddled_things_just_carry(t):
    x, z = 1000, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"fill {x - 30} {GROUND} {z - 30} {x + 30} {GROUND + 8} {z + 60} air")
    s.send(f"fill {x - 30} {GROUND - 1} {z - 30} {x + 30} {GROUND - 1} {z + 60} stone")
    s.send(f"summon pig {x + 0.5} {GROUND} {z + 0.5} " + '{equipment:{saddle:{id:"minecraft:saddle",count:1}}}')
    t.bot.ticks(10)
    s.send(f"ride {name} mount @e[type=pig,limit=1,sort=nearest]")
    d = t.wait(lambda d: d["vehicle"] == "pig", 3)
    t.check(d["vehicle"] == "pig" and d["driving"] is None, f"on a pig with no carrot: {d['vehicle']} {d['driving']}")
    s.send(f"item replace entity {name} weapon.mainhand with minecraft:carrot_on_a_stick")
    d = t.wait(lambda d: d["driving"] == "Pig", 3)
    t.check(d["driving"] == "Pig", f"the carrot gives control, {d['driving']}")
    t.bot.look(0, 0)
    t.bot.ticks(3)
    start = t.bot.dump()["vehicle_pos"]
    t.bot.ticks(40)
    d = t.bot.dump()
    t.check(d["vehicle_pos"][2] > start[2] + 2.0, f"the pig walks towards the carrot: z {start[2]} to {d['vehicle_pos'][2]}")
    t.bot.key("ShiftLeft", True)
    d = t.wait(lambda d: d["vehicle"] is None, 3)
    t.bot.key("ShiftLeft", False)
    # A minecart with no one steering carries its rider where it rolls.
    s.send(f"setblock {x} {GROUND} {z + 20} rail")
    s.send(f"summon minecart {x + 0.5} {GROUND} {z + 20.5}")
    t.bot.ticks(10)
    s.send(f"ride {name} mount @e[type=minecart,limit=1,sort=nearest]")
    d = t.wait(lambda d: d["vehicle"] == "minecart", 3)
    t.check(d["vehicle"] == "minecart" and d["driving"] is None, f"in the minecart: {d['vehicle']} {d['driving']}")
    t.bot.ticks(5)
    d = t.bot.dump()
    t.check(abs(d["pos"][2] - d["vehicle_pos"][2]) < 0.2 and abs(d["pos"][1] - d["vehicle_pos"][1]) < 1.0, f"carried along: {d['pos']} in {d['vehicle_pos']}")
    t.bot.key("ShiftLeft", True)
    d = t.wait(lambda d: d["vehicle"] is None, 3)
    t.bot.key("ShiftLeft", False)
    s.send("kill @e[type=pig]")
    s.send("kill @e[type=minecart]")
    t.bot.ticks(5)


@scenario
def tab_completes_commands_with_what_the_server_offers(t):
    arena(t)
    name = t.bot.name
    t.bot.press("KeyT", "t")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="/gi")
    d = t.wait(lambda d: "give" in d["suggestions"], 3)
    t.check("give" in d["suggestions"], f"offers give for /gi: {d['suggestions']}")
    t.bot.capture("tab-completion")
    t.bot.cmd(t="ui_key", key="Tab")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["chat_input"] == "/give", f"Tab filled it in: {d['chat_input']!r}")
    t.bot.cmd(t="type", text=" ")
    d = t.wait(lambda d: name in d["suggestions"], 3)
    t.check(name in d["suggestions"], f"offers the player's name: {d['suggestions'][:8]}")
    t.bot.cmd(t="type", text=name + " minecraft:diam")
    d = t.wait(lambda d: "minecraft:diamond" in d["suggestions"], 3)
    t.check("minecraft:diamond" in d["suggestions"], f"offers items: {d['suggestions'][:8]}")
    # Arrow keys move through the list rather than chat history.
    first = d["suggestions"][0]
    t.bot.cmd(t="ui_key", key="Down")
    t.bot.ticks(2)
    t.bot.cmd(t="ui_key", key="Tab")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["chat_input"] == f"/give {name} minecraft:diamond_axe", f"Down picked the second one: {d['chat_input']!r}")
    # Esc closes the list first, and the box only the next time.
    t.bot.cmd(t="type", text="x")
    d = t.wait(lambda d: d["chat_input"].endswith("x"), 2)
    t.bot.press("Escape")
    t.bot.ticks(3)
    d = t.bot.dump()
    if "chat" in d["overlays"]:
        t.bot.press("Escape")
        t.bot.ticks(3)
        d = t.bot.dump()
    t.check("chat" not in d["overlays"], f"Esc left the chat box: {d['overlays']}")
    t.check(not d["paused"], "closing chat doesn't pause")
    # Plain chat completes player names.
    t.bot.press("KeyT", "t")
    t.bot.ticks(3)
    t.bot.cmd(t="type", text="hello " + name[:2])
    t.bot.ticks(3)
    t.bot.cmd(t="ui_key", key="Tab")
    t.bot.ticks(3)
    d = t.bot.dump()
    t.check(d["chat_input"] == "hello " + name, f"completes names in chat: {d['chat_input']!r}")
    # The list closes first, then the box.
    for _ in range(2):
        if "chat" in t.bot.dump()["overlays"]:
            t.bot.press("Escape")
            t.bot.ticks(3)
    t.check("chat" not in t.bot.dump()["overlays"], "chat closed")


@scenario
def recipe_book_fills_the_grid_and_shows_what_is_missing(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send(f"recipe give {name} *")
    d = t.wait(lambda d: d["recipes"] > 100, 3)
    t.check(d["recipes"] > 100, f"recipes known: {d['recipes']}")
    s.send(f"give {name} minecraft:oak_log 4")
    s.send(f"give {name} minecraft:stick 8")
    s.send(f"setblock 0 {GROUND} 2 crafting_table")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["menu"] == "crafting", 3)
    t.check(d["menu"] == "crafting", f"crafting table open: {d['menu']}")
    t.bot.click("Recipes")
    t.bot.ticks(20)
    t.bot.capture("recipe-book")
    t.bot.click("recipe search")
    t.bot.ticks(2)
    t.bot.cmd(t="type", text="oak planks")
    t.bot.ticks(5)
    t.bot.click("recipe:oak_planks")
    d = t.wait(lambda d: inventory_item(d, 0) is not None, 3)
    result = inventory_item(d, 0)
    t.check(result and result["item"] == "oak_planks", f"the recipe filled the grid: result {result}")
    t.bot.ticks(3)
    t.bot.capture("recipe-placed")
    # Something the player doesn't have shows its outline instead.
    t.bot.click("recipe search")
    t.bot.ticks(2)
    t.bot.cmd(t="ui_key", key="End")
    t.bot.ticks(1)
    for _ in range(12):
        t.bot.cmd(t="ui_key", key="Backspace")
        t.bot.ticks(1)
    t.bot.ticks(2)
    t.bot.cmd(t="type", text="diamond pickaxe")
    t.bot.ticks(5)
    t.bot.capture("recipe-search")
    d = t.bot.dump()
    t.check(d["book_search"] == "diamond pickaxe", f"search {d['book_search']!r}")
    t.bot.click("recipe:diamond_pickaxe")
    t.bot.ticks(8)
    t.bot.capture("recipe-ghost")
    t.bot.press("Escape")
    t.bot.ticks(5)
    s.send(f"clear {name}")
    s.send(f"setblock 0 {GROUND} 2 air")
    t.bot.ticks(5)


@scenario
def stonecutter_and_loom_offer_real_choices(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send(f"give {name} minecraft:stone 8")
    s.send(f"setblock 0 {GROUND} 2 stonecutter")
    t.bot.ticks(6)
    t.bot.look(0, 40)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["menu"] == "stonecutter", 3)
    t.check(d["menu"] == "stonecutter", f"stonecutter open: {d['menu']}")
    t.check(d["cutter_recipes"] > 50, f"knows what can be cut: {d['cutter_recipes']}")
    stone = next(x["slot"] for x in d["slots"] if x["item"] == "stone")
    t.bot.click(f"slot:{stone}")
    t.bot.ticks(2)
    t.bot.click("slot:0")
    d = t.wait(lambda d: d["cut_choices"] > 3, 3)
    t.check(d["cut_choices"] > 3, f"stone can be cut into several things: {d['cut_choices']}")
    t.bot.ticks(3)
    t.bot.capture("stonecutter")
    t.bot.click("choice:stone_slab")
    d = t.wait(lambda d: inventory_item(d, 1) is not None, 3)
    result = inventory_item(d, 1)
    t.check(result and result["item"] == "stone_slab", f"the choice is what comes out: {result}")
    t.bot.press("Escape")
    t.bot.ticks(5)
    s.send(f"clear {name}")
    s.send(f"give {name} minecraft:white_banner 2")
    s.send(f"give {name} minecraft:red_dye 2")
    s.send(f"setblock 0 {GROUND} 2 loom")
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["menu"] == "loom", 3)
    t.check(d["menu"] == "loom", f"loom open: {d['menu']}")
    banner = next(x["slot"] for x in d["slots"] if x["item"] == "white_banner")
    dye = next(x["slot"] for x in d["slots"] if x["item"] == "red_dye")
    t.bot.click(f"slot:{banner}")
    t.bot.ticks(2)
    t.bot.click("slot:0")
    t.bot.ticks(3)
    t.bot.click(f"slot:{dye}")
    t.bot.ticks(2)
    t.bot.click("slot:1")
    t.bot.ticks(6)
    t.bot.capture("loom")
    t.bot.click("choice:stripe_bottom")
    d = t.wait(lambda d: inventory_item(d, 3) is not None, 3)
    result = inventory_item(d, 3)
    t.check(result and result["item"] == "white_banner", f"the banner comes out: {result}")
    t.bot.ticks(3)
    t.bot.capture("loom-result")
    t.bot.press("Escape")
    t.bot.ticks(5)
    s.send(f"clear {name}")
    s.send(f"setblock 0 {GROUND} 2 air")
    t.bot.ticks(5)


@scenario
def maps_fill_in_and_hang_on_the_wall(t):
    x, z = 1100, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"fill {x + 4} {GROUND} {z - 4} {x + 14} {GROUND + 3} {z + 4} stone")
    s.send(f"fill {x - 14} {GROUND - 1} {z - 4} {x - 4} {GROUND - 1} {z + 4} water")
    s.send(f"item replace entity {name} weapon.mainhand with minecraft:map")
    t.bot.ticks(5)
    t.bot.look(0, 20)
    t.bot.ticks(3)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["maps"] > 0, 4)
    t.check(d["maps"] > 0, f"using an empty map made one: {d['maps']}")
    t.bot.ticks(30)
    t.bot.capture("map-both-hands")
    t.bot.look(0, 70)
    t.bot.ticks(5)
    t.bot.capture("map-both-hands-looking-down")
    t.bot.look(0, 0)
    s.send(f"item replace entity {name} weapon.offhand with minecraft:shield")
    t.bot.ticks(10)
    t.bot.capture("map-one-hand")
    answer = s.ask(f"data get entity {name} SelectedItem.components", "has the following", 2.0)
    number = int(re.findall(r'"minecraft:map_id": (\d+)', answer)[0])
    s.send(f"item replace entity {name} weapon.offhand with minecraft:air")
    # In a frame, out in the world.
    s.send(f"summon item_frame {x} {GROUND + 1} {z - 2} " + "{Facing:3b,Item:{id:\"minecraft:filled_map\",count:1,components:{\"minecraft:map_id\":" + str(number) + "}}}")
    t.bot.ticks(5)
    t.bot.look(180, 0)
    t.bot.ticks(5)
    t.bot.capture("map-in-frame", True)
    s.send("kill @e[type=item_frame]")
    t.bot.ticks(5)


@scenario
def things_cover_the_view_when_they_should(t):
    x, z = 1200, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"effect give {name} minecraft:resistance 60 255 true")
    s.send(f"effect give {name} minecraft:saturation 60 255 true")
    s.send(f"time set noon")
    # Far enough away that what's left flowing doesn't reach the other scenes.
    s.send(f"tp {name} {x - 39.5} {GROUND} {z + 0.5}")
    t.bot.ticks(5)
    s.send(f"fill {x - 43} {GROUND} {z - 3} {x - 37} {GROUND + 3} {z + 3} water")
    d = t.wait(lambda d: d["eye_in_water"], 3)
    t.check(d["eye_in_water"], "eyes under the water")
    t.bot.capture("over-underwater")
    s.send(f"fill {x - 43} {GROUND} {z - 3} {x - 37} {GROUND + 3} {z + 3} air")
    # What's been changed shows quickly.
    before = t.bot.dump()["meshes_uploaded"]
    for waited in range(1, 41):
        t.bot.ticks(1)
        if t.bot.dump()["meshes_uploaded"] > before:
            break
    t.note(f"changed blocks reached the screen after {waited} ticks")
    t.check(waited <= 6, f"changed blocks took {waited} ticks to show")
    d = t.wait(lambda d: not d["eye_in_water"], 3)
    s.send(f"weather clear")
    s.send(f"fill {x + 2} {GROUND} {z} {x + 2} {GROUND + 2} {z} lava")
    s.send(f"tp {name} {x + 2.5} {GROUND} {z + 0.5}")
    d = t.wait(lambda d: d["on_fire"], 5)
    t.check(d["on_fire"], f"lava set the player alight at {d['pos']}")
    t.bot.capture("over-in-lava")
    s.send(f"fill {x + 2} {GROUND} {z} {x + 2} {GROUND + 2} {z} air")
    s.send(f"tp {name} {x + 0.5} {GROUND} {z + 0.5}")
    t.bot.ticks(8)
    t.check(t.bot.dump()["on_fire"], "still burning after getting out")
    t.bot.capture("over-on-fire")
    t.bot.ticks(5)
    s.send(f"tp {name} {x + 0.5} {GROUND} {z + 0.5}")
    s.send(f"fill {x - 1} {GROUND} {z - 1} {x + 1} {GROUND + 1} {z + 1} stone")
    t.bot.ticks(5)
    t.bot.capture("over-inside-a-block")
    # Caught at the edge of a block, the game pushes the player out into the open.
    s.send(f"fill {x - 1} {GROUND} {z - 1} {x + 1} {GROUND + 1} {z + 1} air")
    s.send(f"fill {x} {GROUND} {z} {x} {GROUND + 1} {z} stone")
    s.send(f"tp {name} {x + 0.9} {GROUND} {z + 0.5}")
    t.bot.ticks(10)
    d = t.bot.dump()
    t.check(d["pos"][0] >= x + 1.29, f"pushed out of the block: x {d['pos'][0]}")
    s.send(f"fill {x - 1} {GROUND} {z - 1} {x + 1} {GROUND + 1} {z + 1} air")
    t.bot.ticks(4)
    s.send(f"effect give {name} minecraft:blindness 3 0 true")
    t.bot.ticks(10)
    t.bot.capture("over-blind")
    s.send(f"effect clear {name} minecraft:blindness")
    s.send(f"item replace entity {name} armor.head with minecraft:carved_pumpkin")
    t.bot.ticks(6)
    t.bot.capture("over-pumpkin")
    s.send(f"item replace entity {name} armor.head with minecraft:air")
    s.send(f"item replace entity {name} weapon.mainhand with minecraft:spyglass")
    t.bot.ticks(5)
    t.bot.button(1, True)
    t.bot.ticks(15)
    t.bot.capture("over-spyglass")
    t.bot.button(1, False)
    t.bot.ticks(5)
    s.send(f"setblock {x} {GROUND} {z} nether_portal")
    t.bot.ticks(30)
    t.bot.capture("over-portal")
    s.send(f"setblock {x} {GROUND} {z} air")
    t.bot.ticks(5)
    s.send(f"setblock {x} {GROUND} {z} powder_snow")
    s.send(f"setblock {x} {GROUND + 1} {z} powder_snow")
    t.bot.ticks(100)
    t.bot.capture("over-frozen")
    s.send(f"fill {x} {GROUND} {z} {x} {GROUND + 1} {z} air")
    s.send(f"effect clear {name}")
    t.bot.ticks(5)


@scenario
def advancements_statistics_and_server_screens_open(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send(f"advancement grant {name} only minecraft:story/mine_stone")
    d = t.wait(lambda d: d["advancements_done"] > 0, 3)
    # The server only shares what's near what's been done.
    t.check(d["advancements"] >= 2, f"advancements known: {d['advancements']}")
    t.check(d["advancements_done"] > 0, f"one's done: {d['advancements_done']}")
    t.bot.capture("advancement-note")
    t.bot.press("KeyL", "l")
    d = t.wait(lambda d: d["modal"] == "advancements", 3)
    t.check(d["modal"] == "advancements", f"L opens them: {d['modal']}")
    t.bot.ticks(5)
    t.bot.capture("advancements")
    t.bot.click("Done")
    d = t.wait(lambda d: d["modal"] is None, 3)
    t.check(d["captured"], "back in the game")
    t.bot.press("Escape")
    t.bot.ticks(3)
    t.bot.click("Statistics")
    d = t.wait(lambda d: d["stats"] > 0, 3)
    t.check(d["modal"] == "stats" and d["stats"] > 0, f"statistics came: {d['modal']} {d['stats']}")
    t.bot.ticks(5)
    t.bot.capture("statistics")
    t.bot.click("tab:Items and blocks")
    t.bot.ticks(3)
    t.bot.click("Done")
    t.bot.ticks(3)
    # A server's own screen.
    s.send(f"dialog show {name} " + '{type:"minecraft:confirmation",title:"Are you sure?",body:{type:"minecraft:plain_message",contents:"This is a test."},yes:{label:"Yes",action:{type:"minecraft:run_command",command:"say answered yes"}},no:{label:"No"}}')
    d = t.wait(lambda d: d["modal"] == "dialog", 3)
    t.check(d["modal"] == "dialog", f"the server's screen opened: {d['modal']}")
    t.check(d["modal_text"] == ["This is a test."], f"it says {d['modal_text']}")
    t.bot.capture("dialog")
    t.bot.click("Yes")
    s.wait_line("answered yes", 3)
    d = t.wait(lambda d: d["modal"] is None, 3)
    t.check(d["modal"] is None, "it closed")


@scenario
def clicking_in_chat_does_what_it_says(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send(f'tellraw {name} {{"text":"CLICKME","click_event":{{"action":"run_command","command":"say clicked it"}},"hover_event":{{"action":"show_text","value":"a tip"}}}}')
    s.send(f'tellraw {name} {{"text":"LINKME","underlined":true,"click_event":{{"action":"open_url","url":"https://example.org/x"}}}}')
    t.bot.ticks(5)
    t.bot.press("KeyT", "t")
    t.bot.ticks(3)
    # The newest line sits just above the box, the one before it above that.
    bottom = 760 - 64
    t.bot.cmd(t="pointer", x=40, y=bottom - 12)
    t.bot.ticks(3)
    t.bot.cmd(t="click", x=40, y=bottom - 12)
    d = t.wait(lambda d: d["modal"] == "link", 3)
    t.check(d["modal"] == "link", f"a link asks first: {d['modal']} {d['overlays']}")
    t.bot.capture("link-prompt")
    t.bot.click("Cancel")
    t.bot.ticks(3)
    t.bot.press("KeyT", "t")
    t.bot.ticks(3)
    s.drain()
    t.bot.cmd(t="pointer", x=40, y=bottom - 38)
    t.bot.ticks(3)
    t.bot.cmd(t="click", x=40, y=bottom - 38)
    s.wait_line("clicked it", 3)
    t.bot.ticks(3)
    t.check("chat" not in t.bot.dump()["overlays"], "running a command closes chat")


@scenario
def the_world_border_stops_you_and_shows(t):
    x, z = 1400, 0
    arena(t, x, z)
    s = t.server
    name = t.bot.name
    s.send(f"worldborder center {x} {z}")
    s.send("worldborder set 16")
    d = t.wait(lambda d: d["border_size"] == 16, 3)
    t.check(d["border_size"] == 16, f"border {d['border_size']}")
    s.send(f"tp {name} {x} {GROUND} {z + 2} 0 0")
    t.bot.ticks(10)
    t.bot.look(0, 0)
    t.bot.key("KeyW", True)
    t.bot.ticks(40)
    t.bot.key("KeyW", False)
    d = t.bot.dump()
    # The command centres on the middle of the block, and half a player's width stays inside.
    t.check(abs(d["pos"][2] - (z + 8.5 - 0.3)) < 0.01, f"stopped at the border: z {d['pos'][2]}")
    t.bot.capture("world-border")
    t.bot.ticks(5)
    # The server agrees, with no complaints.
    server = server_pos(t, name)
    t.check(abs(server[2] - d["pos"][2]) < 0.3, f"the server has z {server[2]}, the client {d['pos'][2]}")
    s.send("worldborder center 0 0")
    s.send("worldborder set 59999968")
    t.wait(lambda d: d["border_size"] > 1000, 3)


@scenario
def server_dialogs_take_typed_values(t):
    arena(t)
    s = t.server
    name = t.bot.name
    s.send(
        f"dialog show {name} "
        + '{type:"minecraft:multi_action",title:"Form",'
        + 'inputs:[{type:"minecraft:text",key:"word",label:"Word"},'
        + '{type:"minecraft:boolean",key:"flag",label:"Flag",initial:true,on_true:"yes",on_false:"no"},'
        + '{type:"minecraft:single_option",key:"pick",label:"Pick",options:[{id:"first",display:"First"},{id:"second",display:"Second",initial:true}]},'
        + '{type:"minecraft:number_range",key:"count",label:"Count",start:1,end:10,step:1,initial:3}],'
        + 'actions:[{label:"Send it",action:{type:"minecraft:dynamic/run_command",template:"say got $(word) $(flag) $(pick) $(count)"}}]}'
    )
    d = t.wait(lambda d: d["modal"] == "dialog", 3)
    t.check(d["modal"] == "dialog", f"the dialog opened: {d['modal']}")
    t.bot.ticks(3)
    t.bot.click("input:word")
    t.bot.ticks(2)
    t.bot.cmd(t="type", text="hello")
    t.bot.ticks(3)
    t.bot.capture("dialog-inputs")
    since = len(s.lines.all)
    t.bot.click("Send it")
    d = t.wait(lambda d: d["modal"] is None, 3)
    t.check(d["modal"] is None, "pressing the button closes it")
    t.bot.ticks(10)
    said = [l for l in s.lines.all[since:] if "got " in l]
    t.check(any("got hello yes second 3" in l for l in said), f"the command carried the values: {said}")


@scenario
def words_in_a_book_turn_its_pages(t):
    arena(t)
    s = t.server
    pages = "[[{text:\"Go on\",click_event:{action:\"change_page\",page:2}}],[\"Page two\"]]"
    s.send(f"item replace entity {t.bot.name} weapon.mainhand with minecraft:written_book[written_book_content={{title:\"Test\",author:\"Bot\",pages:{pages}}}]")
    t.bot.ticks(6)
    t.bot.click_mouse(1)
    d = t.wait(lambda d: d["modal"] == "book", 3)
    t.check(d["modal"] == "book", f"the book opened: {d['modal']}")
    t.check(d["modal_page"] == 0, f"on the first page: {d['modal_page']}")
    t.bot.ticks(3)
    t.bot.click("book page")
    d = t.wait(lambda d: d["modal_page"] == 1, 3)
    t.check(d["modal_page"] == 1, f"the words turned the page: {d['modal_page']}")
    t.bot.press("Escape")
    t.bot.ticks(3)


@scenario
def server_links_are_offered_on_the_pause_menu(t):
    arena(t)
    d = t.bot.dump()
    t.check(d["links"] >= 1, f"the server's links arrived: {d['links']}")
    t.bot.press("Escape")
    t.bot.ticks(3)
    t.bot.click("Server links")
    d = t.wait(lambda d: d["modal"] == "links", 3)
    t.check(d["modal"] == "links", f"the links page opened: {d['modal']}")
    t.bot.ticks(3)
    t.bot.capture("server-links")
    t.bot.click("Done")
    d = t.wait(lambda d: d["modal"] is None, 3)
    t.check(d["captured"], "back in the game")


@scenario
def other_players_show_on_the_locator_bar_and_player_list(t):
    arena(t)
    name = t.bot.name
    other = rig.Client("Friend")
    try:
        other.wait_playing()
        t.server.send("gamerule locator_bar true")
        t.server.send(f"tp Friend {name}")
        t.server.send(f"execute at {name} run tp Friend ~10 ~ ~")
        t.server.send(f"gamemode survival {name}")
        d = t.wait(lambda d: d["waypoints"] >= 1, 8)
        t.check(d["waypoints"] >= 1, f"a waypoint arrived: {d['waypoints']}")
        t.bot.ticks(5)
        t.bot.capture("locator-bar")
        t.bot.key("Tab", True)
        t.bot.ticks(3)
        t.bot.capture("player-list")
        t.bot.key("Tab", False)
        t.bot.ticks(2)
    finally:
        other.quit()
    t.server.send(f"gamemode creative {name}")
    d = t.wait(lambda d: d["waypoints"] == 0, 8)
    t.check(d["waypoints"] == 0, f"it left with the player: {d['waypoints']}")
