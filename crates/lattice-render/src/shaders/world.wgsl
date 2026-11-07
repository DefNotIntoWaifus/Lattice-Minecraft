// Lattice world shaders: terrain, sky, and coloured geometry (outlines, entity boxes).

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    // xyz: direction towards the sun, w: daylight 0..1
    sun: vec4<f32>,
    // x: fog start, y: fog end, z: minimum brightness, w: gamma setting, 0 moody to 1 bright
    fog: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

// Light level (0..1) to brightness. Steeper than linear so caves read as dark.
fn light_curve(l: f32) -> f32 {
    return l * l * (3.0 - 2.0 * l) * 0.85 + l * 0.15;
}

// The surface is not sRGB (egui prefers that), so encode gamma on output.
fn encode(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn fog_factor(dist: f32) -> f32 {
    return smoothstep(g.fog.x, g.fog.y, dist);
}

// ---------------------------------------------------------------- terrain

struct TerrainIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) light: vec4<f32>,
    @location(4) offset: vec3<f32>,
};

struct TerrainOut {
    // Invariant so coplanar quads from different draws get identical depth.
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) light: vec2<f32>,
    @location(3) dist: f32,
};

@vertex
fn vs_terrain(v: TerrainIn) -> TerrainOut {
    let world = v.pos + v.offset;
    var out: TerrainOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.uv = v.uv;
    out.color = srgb_to_linear(v.color.rgb);
    out.light = v.light.xy;
    out.dist = length(world.xz);
    return out;
}

fn shade(in: TerrainOut, albedo: vec3<f32>) -> vec3<f32> {
    let sky = light_curve(in.light.x) * mix(0.16, 1.0, g.sun.w);
    let block = light_curve(in.light.y) * vec3<f32>(1.0, 0.86, 0.66);
    // A negative minimum means the darkness effect is pressing all light down.
    let lit = (max(vec3<f32>(sky), block) + vec3<f32>(max(g.fog.z, 0.0))) * (1.0 + min(g.fog.z, 0.0));
    // The brightness setting lifts dark areas more than light ones.
    let bright = pow(min(lit, vec3<f32>(1.0)), vec3<f32>(mix(1.25, 0.6, g.fog.w)));
    let color = albedo * in.color * bright;
    return mix(color, g.sky_horizon.rgb, fog_factor(in.dist));
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    if (tex.a < 0.5) {
        discard;
    }
    return vec4<f32>(encode(shade(in, tex.rgb)), 1.0);
}

@fragment
fn fs_translucent(in: TerrainOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    if (tex.a < 0.01) {
        discard;
    }
    let fade = 1.0 - fog_factor(in.dist);
    return vec4<f32>(encode(shade(in, tex.rgb)), tex.a * mix(0.4, 1.0, fade));
}

// ---------------------------------------------------------------- mobs and moving things

struct EntityIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // x sky, y block, z red flash, w white flash
    @location(3) light: vec4<f32>,
};

struct EntityOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) light: vec4<f32>,
    @location(3) dist: f32,
};

@vertex
fn vs_entity(v: EntityIn) -> EntityOut {
    var out: EntityOut;
    out.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = vec4<f32>(srgb_to_linear(v.color.rgb), v.color.a);
    out.light = v.light;
    out.dist = length(v.pos.xz);
    return out;
}

fn shade_entity(in: EntityOut, albedo: vec3<f32>) -> vec3<f32> {
    var base = albedo * in.color.rgb;
    // Flashes mix as the eye sees them, not in linear light, where red would swamp dark colours.
    if (in.light.z > 0.0 || in.light.w > 0.0) {
        var seen = encode(base);
        seen = mix(seen, vec3<f32>(1.0, 0.0, 0.0), in.light.z * 0.3);
        seen = mix(seen, vec3<f32>(1.0, 1.0, 1.0), in.light.w);
        base = srgb_to_linear(seen);
    }
    let sky = light_curve(in.light.x) * mix(0.16, 1.0, g.sun.w);
    let block = light_curve(in.light.y) * vec3<f32>(1.0, 0.86, 0.66);
    // A negative minimum means the darkness effect is pressing all light down.
    let lit = (max(vec3<f32>(sky), block) + vec3<f32>(max(g.fog.z, 0.0))) * (1.0 + min(g.fog.z, 0.0));
    let bright = pow(min(lit, vec3<f32>(1.0)), vec3<f32>(mix(1.25, 0.6, g.fog.w)));
    return mix(base * bright, g.sky_horizon.rgb, fog_factor(in.dist));
}

@fragment
fn fs_entity(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    if (tex.a * in.color.a < 0.1) {
        discard;
    }
    return vec4<f32>(encode(shade_entity(in, tex.rgb)), 1.0);
}

@fragment
fn fs_entity_translucent(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = tex.a * in.color.a;
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(encode(shade_entity(in, tex.rgb)), a);
}

@fragment
fn fs_entity_glow(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = tex.a * in.color.a * (1.0 - fog_factor(in.dist));
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(encode(tex.rgb * in.color.rgb * a), 1.0);
}

@fragment
fn fs_clouds(in: EntityOut) -> @location(0) vec4<f32> {
    // Clouds fade out at the edge of their own range rather than with the terrain's fog.
    let a = in.color.a * (1.0 - smoothstep(240.0, 384.0, in.dist));
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(encode(in.color.rgb), a);
}

@fragment
fn fs_weather(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = tex.a * in.color.a;
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(encode(shade_entity(in, tex.rgb)), a);
}

@vertex
fn vs_sky_quad(v: EntityIn) -> EntityOut {
    var out: EntityOut;
    let p = g.view_proj * vec4<f32>(v.pos, 1.0);
    // Pinned to the far plane, so anything in the world goes in front.
    out.clip = vec4<f32>(p.xy, p.w * 0.99999, p.w);
    out.uv = v.uv;
    out.color = vec4<f32>(srgb_to_linear(v.color.rgb), v.color.a);
    out.light = v.light;
    out.dist = 0.0;
    return out;
}

@fragment
fn fs_celestial(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = in.color.a * tex.a;
    if (a < 0.004) {
        discard;
    }
    return vec4<f32>(encode(tex.rgb * in.color.rgb), a);
}

// ---------------------------------------------------------------- sky

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let dir = normalize((g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 0.0)).xyz);
    let up = clamp(dir.y, -1.0, 1.0);
    var color = mix(g.sky_horizon.rgb, g.sky_zenith.rgb, smoothstep(0.0, 0.45, up));
    // Below the horizon fade into a darker band so the void isn't bright.
    color = mix(color, g.sky_horizon.rgb * 0.55, smoothstep(0.0, -0.3, up));
    let sun_dir = normalize(g.sun.xyz);
    let s = dot(dir, sun_dir);
    let glow = pow(max(s, 0.0), 64.0) * 0.25 * g.sun.w;
    color = color + vec3<f32>(1.0, 0.93, 0.78) * glow;
    // At dawn and dusk a band of warm light fans out along the horizon on the sun's side.
    if (abs(sun_dir.y) < 0.4) {
        let phase = sun_dir.y / 0.4 * 0.5 + 0.5;
        let strength = pow(1.0 - (1.0 - sin(phase * 3.1415927)) * 0.99, 2.0);
        let band = vec3<f32>(phase * 0.3 + 0.7, phase * phase * 0.7 + 0.2, 0.2);
        let toward = dir.x * sign(sun_dir.x);
        if (toward > 0.0) {
            let u = dir.z / toward / 1.2;
            let v = dir.y / toward / max(0.4 * strength, 0.02);
            let a = strength * clamp(1.0 - sqrt(u * u + v * v), 0.0, 1.0);
            color = srgb_to_linear(mix(encode(color), band, a));
        }
    }
    return vec4<f32>(encode(color), 1.0);
}

// ---------------------------------------------------------------- coloured geometry

struct ColorIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct ColorOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_color(v: ColorIn) -> ColorOut {
    var out: ColorOut;
    out.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    out.color = vec4<f32>(srgb_to_linear(v.color.rgb), v.color.a);
    return out;
}

@fragment
fn fs_color(in: ColorOut) -> @location(0) vec4<f32> {
    return vec4<f32>(encode(in.color.rgb), in.color.a);
}

// Energy swirling over charged creepers and withers: added on top, like the shine.
@fragment
fn fs_energy(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = tex.a * (1.0 - fog_factor(in.dist));
    return vec4<f32>(encode(tex.rgb * in.color.rgb) * a, 1.0);
}

// The surface of End portals and gateways: layers of stars fixed to the screen,
// each one turned, scaled and drifting by its own amount, so the surface looks
// like a window rather than a picture on the block. The vertex's u carries time.
struct PortalOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) screen: vec4<f32>,
    @location(1) time: f32,
    @location(2) dist: f32,
};

@vertex
fn vs_portal(v: EntityIn) -> PortalOut {
    var out: PortalOut;
    out.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    out.screen = out.clip;
    out.time = v.uv.x;
    out.dist = length(v.pos.xz);
    return out;
}

@fragment
fn fs_portal(in: PortalOut) -> @location(0) vec4<f32> {
    let p = in.screen.xy / in.screen.w * 0.5 + vec2<f32>(0.5);
    var color = vec3<f32>(0.012, 0.045, 0.06);
    for (var i = 1; i <= 15; i += 1) {
        let layer = f32(i);
        let t = (layer - 1.0) / 14.0;
        let angle = layer * layer * 2.399963;
        let cosine = cos(angle);
        let sine = sin(angle);
        let scale = (4.5 - layer / 4.0) * 2.0;
        let turned = vec2<f32>(p.x * cosine - p.y * sine, p.x * sine + p.y * cosine) * scale;
        let drift = vec2<f32>(17.0 / layer, (2.0 + layer / 1.5) * in.time * 1.5);
        let stars = textureSampleLevel(atlas, atlas_sampler, turned + drift, 0.0);
        // Dim green-blue far away, bright blue up close, with a little variety.
        let tint = mix(vec3<f32>(0.02, 0.10, 0.10), vec3<f32>(0.09, 0.32, 0.62), t * t) * (0.85 + 0.3 * fract(layer * 0.618));
        color += encode(stars.rgb) * tint;
    }
    let fogged = mix(srgb_to_linear(color), g.sky_horizon.rgb, fog_factor(in.dist));
    return vec4<f32>(encode(fogged), 1.0);
}

// Enchantment shine: added on top of what's already been drawn, where it's already been drawn.
@fragment
fn fs_glint(in: EntityOut) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas, atlas_sampler, in.uv);
    let a = in.color.a * (1.0 - fog_factor(in.dist));
    return vec4<f32>(encode(tex.rgb * tex.rgb * a), 1.0);
}
