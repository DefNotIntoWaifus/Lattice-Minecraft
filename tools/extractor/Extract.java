// Development-time data extractor.
//
// Runs against the *official* server jar's classes (never shipped, never linked
// into Lattice) and writes the facts the client needs to stay in sync with the
// server: block state IDs and properties, collision/outline boxes, physics
// factors, light values, fluid states, entity hitboxes, player attribute
// defaults, plus test vectors produced by the real game code so the Rust ports
// can be verified against them.
//
// Usage: see tools/gen-data.sh

import com.google.gson.stream.JsonWriter;
import java.io.FileWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.ParameterizedType;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.ai.attributes.AttributeSupplier;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;

public class Extract {
    static final Map<String, Integer> shapeIndex = new HashMap<>();
    static final List<List<double[]>> shapes = new ArrayList<>();

    static int shapeId(VoxelShape shape, Vec3 undoOffset) {
        List<double[]> boxes = new ArrayList<>();
        for (AABB b : shape.toAabbs()) {
            boxes.add(new double[] {
                b.minX - undoOffset.x, b.minY - undoOffset.y, b.minZ - undoOffset.z,
                b.maxX - undoOffset.x, b.maxY - undoOffset.y, b.maxZ - undoOffset.z});
        }
        StringBuilder key = new StringBuilder();
        for (double[] b : boxes) {
            for (double d : b) key.append(d).append(',');
            key.append(';');
        }
        Integer id = shapeIndex.get(key.toString());
        if (id == null) {
            id = shapes.size();
            shapes.add(boxes);
            shapeIndex.put(key.toString(), id);
        }
        return id;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    static String valueName(Property p, Comparable v) {
        return p.getName(v);
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        // Index 0 is always "no boxes", index 1 always the full cube.
        shapeId(net.minecraft.world.phys.shapes.Shapes.empty(), Vec3.ZERO);
        shapeId(net.minecraft.world.phys.shapes.Shapes.block(), Vec3.ZERO);

        Method maxH = BlockBehaviour.class.getDeclaredMethod("getMaxHorizontalOffset");
        Method maxV = BlockBehaviour.class.getDeclaredMethod("getMaxVerticalOffset");
        maxH.setAccessible(true);
        maxV.setAccessible(true);

        var getter = EmptyBlockGetter.INSTANCE;
        var zero = BlockPos.ZERO;
        int stateCount = Block.BLOCK_STATE_REGISTRY.size();
        int mismatches = 0;
        Map<net.minecraft.world.level.block.SoundType, Integer> soundTypes = new LinkedHashMap<>();

        try (JsonWriter w = new JsonWriter(new FileWriter(args[0]))) {
            w.beginObject();
            w.name("version").value(SharedConstants.getCurrentVersion().name());
            w.name("protocol").value(SharedConstants.getProtocolVersion());

            // ---- blocks ----
            w.name("blocks").beginArray();
            List<String> offsetBlocks = new ArrayList<>();
            for (Block block : BuiltInRegistries.BLOCK) {
                var states = block.getStateDefinition().getPossibleStates();
                int first = Block.getId(states.get(0));
                for (BlockState s : states) first = Math.min(first, Block.getId(s));
                BlockState def = block.defaultBlockState();
                w.beginObject();
                w.name("name").value(BuiltInRegistries.BLOCK.getKey(block).getPath());
                w.name("first").value(first);
                w.name("count").value(states.size());
                w.name("default").value(Block.getId(def));
                w.name("friction").value(block.getFriction());
                w.name("speed").value(block.getSpeedFactor());
                w.name("jump").value(block.getJumpFactor());
                w.name("hardness").value(def.getDestroySpeed(getter, zero));
                w.name("resistance").value(block.getExplosionResistance());
                w.name("tool").value(def.requiresCorrectToolForDrops());
                w.name("bounce").value(block.getBounceRestitution());
                w.name("has_entity").value(def.hasBlockEntity());
                if (block.asItem() != net.minecraft.world.item.Items.AIR) {
                    w.name("item").value(BuiltInRegistries.ITEM.getKey(block.asItem()).getPath());
                }
                net.minecraft.world.level.block.SoundType sound = def.getSoundType();
                w.name("sound").value(soundTypes.computeIfAbsent(sound, k -> soundTypes.size()));

                // Properties, in state-definition order; values in definition order.
                List<Property<?>> props = new ArrayList<>(block.getStateDefinition().getProperties());
                w.name("props").beginArray();
                for (Property<?> p : props) {
                    w.beginArray();
                    w.value(p.getName());
                    w.beginArray();
                    for (Object v : p.getPossibleValues()) w.value(valueName((Property) p, (Comparable) v));
                    w.endArray();
                    w.endArray();
                }
                w.endArray();

                // Verify our assumption about how state IDs are laid out:
                // id = first + mixed-radix index over props, last prop varies fastest.
                for (BlockState s : states) {
                    int idx = 0;
                    for (Property<?> p : props) {
                        List<?> vals = new ArrayList<>(p.getPossibleValues());
                        idx = idx * vals.size() + vals.indexOf(s.getValue(p));
                    }
                    if (first + idx != Block.getId(s)) mismatches++;
                }

                if (def.hasOffsetFunction()) {
                    offsetBlocks.add(BuiltInRegistries.BLOCK.getKey(block).getPath());
                    boolean anyY = false;
                    for (int x = -8; x < 8; x++)
                        for (int z = -8; z < 8; z++)
                            for (int y = -2; y < 2; y++)
                                if (def.getOffset(new BlockPos(x, y, z)).y != 0) anyY = true;
                    w.name("offset").beginObject();
                    w.name("y").value(anyY);
                    w.name("max_h").value((float) maxH.invoke(block));
                    w.name("max_v").value((float) maxV.invoke(block));
                    w.endObject();
                }
                w.endObject();
            }
            w.endArray();

            // ---- states, indexed by state id ----
            // [collision shape, outline shape, flags, emission, dampening, fluid kind, fluid amount]
            w.name("states").beginArray();
            for (int id = 0; id < stateCount; id++) {
                BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
                Vec3 off = s.hasOffsetFunction() ? s.getOffset(zero) : Vec3.ZERO;
                int collision = shapeId(s.getCollisionShape(getter, zero), Vec3.ZERO);
                int outline = shapeId(s.getShape(getter, zero), off);
                int flags = 0;
                if (s.isAir()) flags |= 1;
                if (s.canOcclude()) flags |= 1 << 1;
                if (s.isSolidRender()) flags |= 1 << 2;
                if (s.liquid()) flags |= 1 << 3;
                if (s.propagatesSkylightDown()) flags |= 1 << 4;
                if (s.getRenderShape() == RenderShape.INVISIBLE) flags |= 1 << 5;
                if (s.skipRendering(s, Direction.UP)) flags |= 1 << 6;
                if (s.isCollisionShapeFullBlock(getter, zero)) flags |= 1 << 7;
                if (s.canBeReplaced()) flags |= 1 << 8;
                // Most blocks suffocate by a tag that isn't loaded here, which gen-data.sh sees
                // to. Bit 19 marks the ones with their own rule, which can be asked now.
                try {
                    var rule = net.minecraft.world.level.block.state.BlockBehaviour.BlockStateBase.class.getDeclaredField("isSuffocating");
                    rule.setAccessible(true);
                    if (rule.get(s).getClass().getName().contains("Blocks$$")) {
                        flags |= 1 << 19;
                        if (s.isSuffocating(getter, zero)) flags |= 1 << 18;
                    }
                } catch (ReflectiveOperationException e) {
                    throw new RuntimeException(e);
                }
                FluidState f = s.getFluidState();
                if (f.isSource()) flags |= 1 << 9;
                if (s.isSolid()) flags |= 1 << 10;
                if (s.useShapeForLightOcclusion()) flags |= 1 << 11;
                for (Direction d : Direction.values()) {
                    if (Block.isFaceFull(s.getFaceOcclusionShape(d), d)) flags |= 1 << (12 + d.get3DDataValue());
                    if (s.isFaceSturdy(getter, zero, d)) flags |= 1 << (20 + d.get3DDataValue());
                }
                // Most blocks suffocate by a tag that isn't loaded here, which gen-data.sh sees
                // to. Bit 19 marks the ones with their own rule, which can be asked now.
                try {
                    var rule = net.minecraft.world.level.block.state.BlockBehaviour.BlockStateBase.class.getDeclaredField("isSuffocating");
                    rule.setAccessible(true);
                    if (rule.get(s).getClass().getName().contains("Blocks$$")) {
                        flags |= 1 << 19;
                        if (s.isSuffocating(getter, zero)) flags |= 1 << 18;
                    }
                } catch (ReflectiveOperationException e) {
                    throw new RuntimeException(e);
                }
                int fluid = 0;
                if (!f.isEmpty()) {
                    if (f.getType() == Fluids.WATER || f.getType() == Fluids.FLOWING_WATER) fluid = 1;
                    else if (f.getType() == Fluids.LAVA || f.getType() == Fluids.FLOWING_LAVA) fluid = 2;
                    else fluid = 3;
                }
                w.beginArray();
                w.value(collision);
                w.value(outline);
                w.value(flags);
                w.value(s.getLightEmission());
                w.value(s.getLightDampening());
                w.value(fluid);
                w.value(f.getAmount());
                w.endArray();
            }
            w.endArray();

            w.name("shapes").beginArray();
            for (List<double[]> boxes : shapes) {
                w.beginArray();
                for (double[] b : boxes) {
                    w.beginArray();
                    for (double d : b) w.value(d);
                    w.endArray();
                }
                w.endArray();
            }
            w.endArray();

            // ---- status effects and potions ----
            w.name("mob_effects").beginArray();
            for (var e : BuiltInRegistries.MOB_EFFECT) {
                w.beginObject();
                w.name("name").value(BuiltInRegistries.MOB_EFFECT.getKey(e).getPath());
                w.name("color").value(e.getColor());
                w.name("category").value(e.getCategory().name().toLowerCase());
                w.name("key").value(e.getDescriptionId());
                w.endObject();
            }
            w.endArray();
            w.name("potions").beginObject();
            for (var p : BuiltInRegistries.POTION) {
                w.name(BuiltInRegistries.POTION.getKey(p).getPath()).beginArray();
                for (var i : p.getEffects()) {
                    w.beginArray();
                    w.value(BuiltInRegistries.MOB_EFFECT.getId(i.getEffect().value()));
                    w.value(i.getAmplifier());
                    w.value(i.getDuration());
                    w.endArray();
                }
                w.endArray();
            }
            w.endObject();

            // ---- block sound types ----
            w.name("sound_types").beginArray();
            for (net.minecraft.world.level.block.SoundType t : soundTypes.keySet()) {
                w.beginObject();
                w.name("break").value(soundName(t.getBreakSound()));
                w.name("step").value(soundName(t.getStepSound()));
                w.name("place").value(soundName(t.getPlaceSound()));
                w.name("hit").value(soundName(t.getHitSound()));
                w.name("fall").value(soundName(t.getFallSound()));
                w.name("volume").value(t.getVolume());
                w.name("pitch").value(t.getPitch());
                w.endObject();
            }
            w.endArray();

            // ---- entity data fields, by class ----
            Map<EntityType<?>, Class<?>> entityClasses = new HashMap<>();
            for (Field f : net.minecraft.world.entity.EntityTypes.class.getFields()) {
                if (f.getType() == EntityType.class && f.getGenericType() instanceof ParameterizedType pt
                        && pt.getActualTypeArguments()[0] instanceof Class<?> c) {
                    entityClasses.put((EntityType<?>) f.get(null), c);
                }
            }
            Map<Class<?>, Boolean> seenClasses = new LinkedHashMap<>();
            for (Class<?> c : entityClasses.values()) {
                for (Class<?> k = c; k != null && k != Object.class; k = k.getSuperclass()) seenClasses.put(k, true);
            }
            w.name("entity_classes").beginObject();
            for (Class<?> c : seenClasses.keySet()) {
                w.name(c.getSimpleName()).beginObject();
                if (c.getSuperclass() != null && c.getSuperclass() != Object.class) w.name("parent").value(c.getSuperclass().getSimpleName());
                w.name("fields").beginArray();
                for (Field f : c.getDeclaredFields()) {
                    if (Modifier.isStatic(f.getModifiers()) && f.getType() == net.minecraft.network.syncher.EntityDataAccessor.class) {
                        f.setAccessible(true);
                        var acc = (net.minecraft.network.syncher.EntityDataAccessor<?>) f.get(null);
                        w.beginArray();
                        w.value(acc.id());
                        w.value(net.minecraft.network.syncher.EntityDataSerializers.getSerializedId(acc.serializer()));
                        w.value(f.getName());
                        w.endArray();
                    }
                }
                w.endArray();
                w.endObject();
            }
            w.endObject();
            w.name("entity_serializers").beginObject();
            for (Field f : net.minecraft.network.syncher.EntityDataSerializers.class.getFields()) {
                if (Modifier.isStatic(f.getModifiers()) && net.minecraft.network.syncher.EntityDataSerializer.class.isAssignableFrom(f.getType())) {
                    w.name(f.getName()).value(net.minecraft.network.syncher.EntityDataSerializers.getSerializedId((net.minecraft.network.syncher.EntityDataSerializer<?>) f.get(null)));
                }
            }
            w.endObject();

            // ---- entity types ----
            w.name("entities").beginArray();
            for (EntityType<?> t : BuiltInRegistries.ENTITY_TYPE) {
                var dim = t.getDimensions();
                w.beginObject();
                w.name("name").value(BuiltInRegistries.ENTITY_TYPE.getKey(t).getPath());
                w.name("id").value(BuiltInRegistries.ENTITY_TYPE.getId(t));
                w.name("width").value(dim.width());
                w.name("height").value(dim.height());
                w.name("eye").value(dim.eyeHeight());
                Class<?> ec = entityClasses.get(t);
                if (ec != null) w.name("class").value(ec.getSimpleName());
                // Where riders sit on it, and where it sits on what it rides.
                w.name("seats").beginArray();
                for (int seat = 0; seat < 4; seat++) {
                    var p = dim.attachments().getClamped(net.minecraft.world.entity.EntityAttachment.PASSENGER, seat, 0f);
                    w.beginArray().value(p.x).value(p.y).value(p.z).endArray();
                }
                w.endArray();
                var sit = dim.attachments().get(net.minecraft.world.entity.EntityAttachment.VEHICLE, 0, 0f);
                w.name("rides_at").beginArray().value(sit.x).value(sit.y).value(sit.z).endArray();
                // What's known about how it moves, for driving it.
                if (net.minecraft.world.entity.ai.attributes.DefaultAttributes.hasSupplier(t)) {
                    @SuppressWarnings("unchecked")
                    var sup = net.minecraft.world.entity.ai.attributes.DefaultAttributes.getSupplier((EntityType<? extends net.minecraft.world.entity.LivingEntity>) t);
                    w.name("attributes").beginObject();
                    for (var e : java.util.Map.of(
                            "movement_speed", Attributes.MOVEMENT_SPEED,
                            "jump_strength", Attributes.JUMP_STRENGTH,
                            "step_height", Attributes.STEP_HEIGHT,
                            "flying_speed", Attributes.FLYING_SPEED,
                            "gravity", Attributes.GRAVITY,
                            "max_health", Attributes.MAX_HEALTH).entrySet()) {
                        if (sup.hasAttribute(e.getValue())) w.name(e.getKey()).value(sup.getValue(e.getValue()));
                    }
                    w.endObject();
                }
                w.endObject();
            }
            w.endArray();

            // ---- player attribute defaults ----
            AttributeSupplier attrs = Player.createAttributes().build();
            // ---- what each map colour number looks like ----
            w.name("map_colors").beginArray();
            for (int c = 0; c < 64; c++) {
                w.value(net.minecraft.world.level.material.MapColor.byId(c).col);
            }
            w.endArray();

            // ---- how map markers look, in registry order ----
            w.name("map_decorations").beginArray();
            for (var d : BuiltInRegistries.MAP_DECORATION_TYPE) {
                w.beginObject();
                w.name("asset").value(d.assetId().getPath());
                w.name("on_frame").value(d.showOnItemFrame());
                w.endObject();
            }
            w.endArray();

            w.name("player").beginObject();
            w.name("movement_speed").value(attrs.getValue(Attributes.MOVEMENT_SPEED));
            w.name("gravity").value(attrs.getValue(Attributes.GRAVITY));
            w.name("jump_strength").value(attrs.getValue(Attributes.JUMP_STRENGTH));
            w.name("step_height").value(attrs.getValue(Attributes.STEP_HEIGHT));
            w.name("sneaking_speed").value(attrs.getValue(Attributes.SNEAKING_SPEED));
            w.name("block_interaction_range").value(attrs.getValue(Attributes.BLOCK_INTERACTION_RANGE));
            w.name("entity_interaction_range").value(attrs.getValue(Attributes.ENTITY_INTERACTION_RANGE));
            w.name("water_movement_efficiency").value(attrs.getValue(Attributes.WATER_MOVEMENT_EFFICIENCY));
            w.name("movement_efficiency").value(attrs.getValue(Attributes.MOVEMENT_EFFICIENCY));
            var pdim = net.minecraft.world.entity.EntityTypes.PLAYER.getDimensions();
            w.name("width").value(pdim.width());
            w.name("height").value(pdim.height());
            w.name("eye").value(pdim.eyeHeight());
            w.endObject();

            // ---- test vectors from the real implementation ----
            w.name("vectors").beginObject();
            w.name("seed").beginArray();
            int[][] seedPos = {{0,0,0},{1,2,3},{-1,64,-1},{123456,-60,-987654},{30000000,319,-30000000},{-17,5,42}};
            for (int[] p : seedPos) {
                w.beginArray();
                w.value(p[0]); w.value(p[1]); w.value(p[2]);
                w.value(Long.toString(Mth.getSeed(p[0], p[1], p[2])));
                w.endArray();
            }
            w.endArray();
            w.name("offset").beginArray();
            for (String name : offsetBlocks) {
                BlockState def = BuiltInRegistries.BLOCK.getValue(net.minecraft.resources.Identifier.withDefaultNamespace(name)).defaultBlockState();
                for (int[] p : seedPos) {
                    Vec3 o = def.getOffset(new BlockPos(p[0], p[1], p[2]));
                    w.beginArray();
                    w.value(name);
                    w.value(p[0]); w.value(p[1]); w.value(p[2]);
                    w.value(o.x); w.value(o.y); w.value(o.z);
                    w.endArray();
                }
            }
            w.endArray();
            w.endObject();

            w.endObject();
        }
        System.out.println("states: " + stateCount + ", shapes: " + shapes.size() + ", id layout mismatches: " + mismatches);
        if (mismatches != 0) System.exit(2);
    }

    private static String soundName(net.minecraft.sounds.SoundEvent e) {
        return e.location().getPath();
    }
}
