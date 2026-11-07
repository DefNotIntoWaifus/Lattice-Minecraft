// Runs inside the official client as a test reference. It starts the game, then takes
// commands on stdin so scripted scenes can be replayed and recorded: movement per tick,
// block states as the client sees them, and screenshots. Lattice's own results are
// compared with these recordings. Never shipped; see tools/reference/run.py.

import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.ConcurrentLinkedQueue;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.Screenshot;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

public final class RefHarness {
    private static final ConcurrentLinkedQueue<String> COMMANDS = new ConcurrentLinkedQueue<>();
    private static volatile Minecraft mc;

    private record Segment(int ticks, List<String> keys, Float yaw, Float pitch) {}

    private static final class Run {
        String name;
        PrintWriter out;
        List<Segment> segments = new ArrayList<>();
        int segment = -1;
        int left = 0;
        int rows = 0;
        int missed = 0;
    }

    private static Run run;
    private static int lastTick = Integer.MIN_VALUE;

    public static void main(String[] args) throws Exception {
        Thread control = new Thread(RefHarness::control, "ref-control");
        control.setDaemon(true);
        control.start();
        net.minecraft.client.main.Main.main(args);
    }

    private static void say(String s) {
        System.out.println("REF " + s);
        System.out.flush();
    }

    private static void control() {
        Thread reader = new Thread(() -> {
            try (BufferedReader in = new BufferedReader(new InputStreamReader(System.in))) {
                for (String line; (line = in.readLine()) != null; ) {
                    COMMANDS.add(line);
                }
            } catch (IOException e) {
                say("stdin error " + e);
            }
        }, "ref-stdin");
        reader.setDaemon(true);
        reader.start();
        try {
            while (true) {
                if (mc == null) {
                    mc = Minecraft.getInstance();
                }
                if (mc != null) {
                    // Tasks queued from here run between ticks on the game thread.
                    mc.execute(RefHarness::step);
                }
                Thread.sleep(1);
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private static void step() {
        try {
            String c;
            while ((c = COMMANDS.poll()) != null) {
                handle(c);
            }
            LocalPlayer p = mc.player;
            if (p == null) {
                return;
            }
            int t = p.tickCount;
            if (t != lastTick) {
                if (run != null && lastTick != Integer.MIN_VALUE && t != lastTick + 1) {
                    run.missed += t - lastTick - 1;
                }
                lastTick = t;
                onTick(p);
            }
        } catch (Throwable e) {
            say("error " + e);
            e.printStackTrace();
        }
    }

    private static KeyMapping key(Options o, String name) {
        return switch (name) {
            case "forward" -> o.keyUp;
            case "back" -> o.keyDown;
            case "left" -> o.keyLeft;
            case "right" -> o.keyRight;
            case "jump" -> o.keyJump;
            case "sneak" -> o.keyShift;
            case "sprint" -> o.keySprint;
            case "attack" -> o.keyAttack;
            case "use" -> o.keyUse;
            default -> throw new IllegalArgumentException("unknown key " + name);
        };
    }

    private static void releaseAll(Options o) {
        for (String k : List.of("forward", "back", "left", "right", "jump", "sneak", "sprint", "attack", "use")) {
            key(o, k).setDown(false);
        }
    }

    private static String row(LocalPlayer p) {
        Vec3 v = p.getDeltaMovement();
        return String.format(Locale.ROOT,
                "{\"pos\":[%s,%s,%s],\"vel\":[%s,%s,%s],\"yaw\":%s,\"pitch\":%s,\"ground\":%b,\"wall\":%b,\"water\":%b,\"under\":%b,\"lava\":%b,\"swim\":%b,\"sprint\":%b,\"crouch\":%b,\"fly\":%b,\"pose\":\"%s\"}",
                p.getX(), p.getY(), p.getZ(), v.x, v.y, v.z, p.getYRot(), p.getXRot(), p.onGround(), p.horizontalCollision,
                p.isInWater(), p.isUnderWater(), p.isInLava(), p.isSwimming(), p.isSprinting(), p.isCrouching(),
                p.getAbilities().flying, p.getPose());
    }

    private static void onTick(LocalPlayer p) {
        Run r = run;
        if (r == null) {
            return;
        }
        if (r.segment >= 0) {
            r.out.println(row(p));
            r.rows++;
        } else {
            r.out.println(row(p));
            r.segment = 0;
            r.left = 0;
        }
        while (r.left == 0) {
            if (r.segment >= r.segments.size()) {
                releaseAll(mc.options);
                r.out.close();
                run = null;
                say("done " + r.name + " rows=" + r.rows + " missed=" + r.missed);
                return;
            }
            Segment s = r.segments.get(r.segment++);
            releaseAll(mc.options);
            for (String k : s.keys()) {
                key(mc.options, k).setDown(true);
            }
            if (s.yaw() != null) {
                p.setYRot(s.yaw());
                p.yRotO = s.yaw();
            }
            if (s.pitch() != null) {
                p.setXRot(s.pitch());
                p.xRotO = s.pitch();
            }
            r.left = s.ticks();
        }
        r.left--;
    }

    private static void handle(String line) throws IOException {
        String[] a = line.trim().split("\\s+");
        switch (a[0]) {
            case "run" -> {
                // run NAME FILE ticks:keys:yaw:pitch ...   (keys "-" for none, "~" keeps the rotation)
                Run r = new Run();
                r.name = a[1];
                Files.createDirectories(Path.of(a[2]).getParent());
                r.out = new PrintWriter(Files.newBufferedWriter(Path.of(a[2])));
                for (int i = 3; i < a.length; i++) {
                    String[] f = a[i].split(":");
                    r.segments.add(new Segment(
                            Integer.parseInt(f[0]),
                            f[1].equals("-") ? List.of() : List.of(f[1].split(",")),
                            f[2].equals("~") ? null : Float.valueOf(f[2]),
                            f[3].equals("~") ? null : Float.valueOf(f[3])));
                }
                run = r;
            }
            case "dump" -> {
                // dump FILE x1 y1 z1 x2 y2 z2: writes "x y z stateId" for every non-air block
                Files.createDirectories(Path.of(a[1]).getParent());
                try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(a[1])))) {
                    for (int x = Integer.parseInt(a[2]); x <= Integer.parseInt(a[5]); x++) {
                        for (int y = Integer.parseInt(a[3]); y <= Integer.parseInt(a[6]); y++) {
                            for (int z = Integer.parseInt(a[4]); z <= Integer.parseInt(a[7]); z++) {
                                BlockState s = mc.level.getBlockState(new BlockPos(x, y, z));
                                if (!s.isAir()) {
                                    out.println(x + " " + y + " " + z + " " + Block.getId(s));
                                }
                            }
                        }
                    }
                }
                say("dumped " + a[1]);
            }
            case "models" -> {
                // models FILE: every model definition as the game itself holds it.
                dumpModels(a[1]);
                say("models done");
            }
            case "creative" -> {
                // creative FILE: writes the creative inventory, which only exists in a running game.
                writeCreativeTabs(a[1]);
                say("creative done");
            }
            case "shot" -> {
                // shot DIR NAME.png
                Screenshot.grab(new File(a[1]), a[2], mc.gameRenderer.mainRenderTarget(), 1, c -> say("shot " + a[2]));
            }
            case "look" -> {
                mc.player.setYRot(Float.parseFloat(a[1]));
                mc.player.yRotO = Float.parseFloat(a[1]);
                mc.player.setXRot(Float.parseFloat(a[2]));
                mc.player.xRotO = Float.parseFloat(a[2]);
                say("looked");
            }
            case "cmd" -> mc.getConnection().sendCommand(line.substring(4));
            case "state" -> say("state " + (mc.player == null ? "null" : row(mc.player)) + " screen=" + mc.gui.screen());
            case "quit" -> mc.stop();
            default -> say("unknown command " + a[0]);
        }
    }

    @SuppressWarnings("unchecked")
    private static void writePart(com.google.gson.stream.JsonWriter w, net.minecraft.client.model.geom.builders.PartDefinition p) throws Exception {
        java.lang.reflect.Field cubesF = p.getClass().getDeclaredField("cubes");
        java.lang.reflect.Field poseF = p.getClass().getDeclaredField("partPose");
        java.lang.reflect.Field childrenF = p.getClass().getDeclaredField("children");
        cubesF.setAccessible(true);
        poseF.setAccessible(true);
        childrenF.setAccessible(true);
        var pose = (net.minecraft.client.model.geom.PartPose) poseF.get(p);
        w.beginObject();
        w.name("offset").beginArray().value(pose.x()).value(pose.y()).value(pose.z()).endArray();
        w.name("rotation").beginArray().value(pose.xRot()).value(pose.yRot()).value(pose.zRot()).endArray();
        w.name("scale").beginArray().value(pose.xScale()).value(pose.yScale()).value(pose.zScale()).endArray();
        w.name("cubes").beginArray();
        for (Object c : (java.util.List<?>) cubesF.get(p)) {
            java.lang.reflect.Field f;
            f = c.getClass().getDeclaredField("texCoord");
            f.setAccessible(true);
            var tex = (net.minecraft.client.model.geom.builders.UVPair) f.get(c);
            f = c.getClass().getDeclaredField("origin");
            f.setAccessible(true);
            var origin = (org.joml.Vector3fc) f.get(c);
            f = c.getClass().getDeclaredField("dimensions");
            f.setAccessible(true);
            var dims = (org.joml.Vector3fc) f.get(c);
            f = c.getClass().getDeclaredField("grow");
            f.setAccessible(true);
            Object grow = f.get(c);
            f = c.getClass().getDeclaredField("mirror");
            f.setAccessible(true);
            boolean mirror = f.getBoolean(c);
            f = c.getClass().getDeclaredField("texScale");
            f.setAccessible(true);
            var scale = (net.minecraft.client.model.geom.builders.UVPair) f.get(c);
            f = c.getClass().getDeclaredField("visibleFaces");
            f.setAccessible(true);
            var faces = (java.util.Set<?>) f.get(c);
            w.beginObject();
            w.name("tex").beginArray().value(tex.u()).value(tex.v()).endArray();
            w.name("origin").beginArray().value(origin.x()).value(origin.y()).value(origin.z()).endArray();
            w.name("size").beginArray().value(dims.x()).value(dims.y()).value(dims.z()).endArray();
            w.name("grow").beginArray();
            for (String g : List.of("growX", "growY", "growZ")) {
                java.lang.reflect.Field gf = grow.getClass().getDeclaredField(g);
                gf.setAccessible(true);
                w.value(gf.getFloat(grow));
            }
            w.endArray();
            w.name("mirror").value(mirror);
            w.name("tex_scale").beginArray().value(scale.u()).value(scale.v()).endArray();
            w.name("faces").beginArray();
            for (String n : List.of("DOWN", "UP", "NORTH", "SOUTH", "WEST", "EAST")) {
                w.value(faces.stream().anyMatch(x -> x.toString().equalsIgnoreCase(n)));
            }
            w.endArray();
            w.endObject();
        }
        w.endArray();
        w.name("children").beginObject();
        var children = new java.util.TreeMap<>((java.util.Map<String, net.minecraft.client.model.geom.builders.PartDefinition>) childrenF.get(p));
        for (var e : children.entrySet()) {
            w.name(e.getKey());
            writePart(w, e.getValue());
        }
        w.endObject();
        w.endObject();
    }

    private static void dumpModels(String file) throws IOException {
        Files.createDirectories(Path.of(file).getParent());
        try (com.google.gson.stream.JsonWriter w = new com.google.gson.stream.JsonWriter(Files.newBufferedWriter(Path.of(file)))) {
            w.beginObject();
            var roots = net.minecraft.client.model.geom.LayerDefinitions.createRoots();
            for (var e : roots.entrySet()) {
                w.name(e.getKey().model().getPath() + "#" + e.getKey().layer());
                var layer = e.getValue();
                java.lang.reflect.Field meshF = layer.getClass().getDeclaredField("mesh");
                java.lang.reflect.Field matF = layer.getClass().getDeclaredField("material");
                meshF.setAccessible(true);
                matF.setAccessible(true);
                var mesh = (net.minecraft.client.model.geom.builders.MeshDefinition) meshF.get(layer);
                Object mat = matF.get(layer);
                w.beginObject();
                for (String sz : List.of("xTexSize", "yTexSize")) {
                    java.lang.reflect.Field mf = mat.getClass().getDeclaredField(sz);
                    mf.setAccessible(true);
                    w.name(sz).value(mf.getInt(mat));
                }
                w.name("root");
                writePart(w, mesh.getRoot());
                w.endObject();
            }
            w.endObject();
        } catch (Exception ex) {
            throw new IOException(ex);
        }
    }

    private static void writeCreativeTabs(String file) throws IOException {
        var access = mc.player.connection.registryAccess();
        net.minecraft.world.item.CreativeModeTabs.tryRebuildTabContents(mc.player.connection.enabledFeatures(), true, access);
        var ops = access.createSerializationContext(com.mojang.serialization.JsonOps.INSTANCE);
        Files.createDirectories(Path.of(file).getParent());
        try (com.google.gson.stream.JsonWriter w = new com.google.gson.stream.JsonWriter(Files.newBufferedWriter(Path.of(file)))) {
            w.beginArray();
            for (var tab : net.minecraft.world.item.CreativeModeTabs.allTabs()) {
                w.beginObject();
                w.name("id").value(net.minecraft.core.registries.BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab).getPath());
                w.name("title").value(tab.getDisplayName().getContents() instanceof net.minecraft.network.chat.contents.TranslatableContents tc ? tc.getKey() : tab.getDisplayName().getString());
                w.name("icon").value(net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(tab.getIconItem().getItem()).getPath());
                w.name("kind").value(tab.getType().name().toLowerCase(Locale.ROOT));
                w.name("row").value(tab.row().name().toLowerCase(Locale.ROOT));
                w.name("column").value(tab.column());
                w.name("items").beginArray();
                var stacks = tab.getType() == net.minecraft.world.item.CreativeModeTab.Type.SEARCH ? tab.getSearchTabDisplayItems() : tab.getDisplayItems();
                for (var stack : stacks) {
                    var patch = stack.getComponentsPatch();
                    w.beginArray();
                    w.value(net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(stack.getItem()).getPath());
                    if (!patch.isEmpty()) {
                        w.value(net.minecraft.core.component.DataComponentPatch.CODEC.encodeStart(ops, patch).getOrThrow().toString());
                    }
                    w.endArray();
                }
                w.endArray();
                w.endObject();
            }
            w.endArray();
        }
    }

    private RefHarness() {}
}
