#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 cell;
    float2 viewport;
    float2 cell_uv;
};

struct Instance {
    float x;
    float y;
    packed_float4 fg;
    packed_float4 bg;
    packed_float2 uv;
    uint flags; // bit0 underline, bit1 strike, bit2 dim
    float w;    // proportional quad width (device px); 0 = uniform cell width
    float h;    // proportional quad height (device px); 0 = uniform cell height
};

struct VOut {
    float4 pos [[position]];
    float4 fg;
    float4 bg;
    float2 uv;
    float2 loc; // cell-local 0..1 (for underline/strike rules)
    uint flags [[flat]];
};

vertex VOut v_main(uint vid [[vertex_id]],
                   uint iid [[instance_id]],
                   constant Instance *inst [[buffer(0)]],
                   constant Uniforms &u [[buffer(1)]]) {
    float2 corner = float2(vid & 1, (vid >> 1) & 1);
    Instance in = inst[iid];
    // Proportional glyphs (Plex Sans chrome) carry w > 0: draw a narrower quad
    // and sample only the left w/cell_w slice of the cell. Mono cells (w == 0)
    // fall back to the uniform cell box.
    float qw = in.w > 0.0 ? in.w : u.cell.x;
    float qh = in.h > 0.0 ? in.h : u.cell.y;
    float uvw = in.w > 0.0 ? (in.w / u.cell.x) * u.cell_uv.x : u.cell_uv.x;
    float uvh = in.h > 0.0 ? (in.h / u.cell.y) * u.cell_uv.y : u.cell_uv.y;
    float2 cell_px = float2(qw, qh);
    float2 px = float2(in.x, in.y) + corner * cell_px;
    float2 ndc = (px / u.viewport) * 2.0 - 1.0;
    ndc.y = -ndc.y;

    VOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    o.fg = in.fg;
    o.bg = in.bg;
    o.uv = in.uv + corner * float2(uvw, uvh);
    o.loc = corner;
    o.flags = in.flags;
    return o;
}

static inline float3 srgb_to_linear(float3 c) {
    return select(c / 12.92, pow((c + 0.055) / 1.055, 2.4), c > 0.04045);
}
static inline float3 linear_to_srgb(float3 c) {
    return select(c * 12.92, 1.055 * pow(c, 1.0 / 2.4) - 0.055, c > 0.0031308);
}

fragment float4 f_main(VOut in [[stage_in]],
                       texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler s(coord::normalized, filter::nearest, address::clamp_to_edge);
    // Bar / underline cursors: fill a thin band in the cursor color, discard the
    // rest so the glyph drawn underneath shows through.
    if (in.flags & 8u) { // bar cursor (left edge)
        if (in.loc.x < 0.12) return float4(in.fg.rgb, 1.0);
        discard_fragment();
    }
    if (in.flags & 16u) { // underline cursor (bottom band)
        if (in.loc.y > 0.85) return float4(in.fg.rgb, 1.0);
        discard_fragment();
    }
    float coverage = atlas.sample(s, in.uv).r;
    // Blend glyph coverage in linear light, then re-encode: anti-aliased edges
    // stay the right weight instead of the muddy/thin look of an sRGB-space mix.
    float3 bg_lin = srgb_to_linear(in.bg.rgb);
    float3 fg_lin = srgb_to_linear(in.fg.rgb);
    if (in.flags & 4u) fg_lin = mix(bg_lin, fg_lin, 0.55); // dim toward background
    float3 lin = mix(bg_lin, fg_lin, coverage);
    if ((in.flags & 1u) && in.loc.y > 0.90 && in.loc.y < 0.96) lin = fg_lin; // underline
    if ((in.flags & 2u) && in.loc.y > 0.48 && in.loc.y < 0.54) lin = fg_lin; // strike
    return float4(linear_to_srgb(lin), 1.0);
}

// Solid-color pixel-space quad, used for chrome (title bar, separator).
struct SolidUniforms {
    float4 rect; // x, y, w, h in device pixels
    float4 color;
    float2 viewport;
};

vertex float4 v_solid(uint vid [[vertex_id]],
                      constant SolidUniforms &u [[buffer(0)]]) {
    float2 corner = float2(vid & 1, (vid >> 1) & 1);
    float2 px = u.rect.xy + corner * u.rect.zw;
    float2 ndc = (px / u.viewport) * 2.0 - 1.0;
    ndc.y = -ndc.y;
    return float4(ndc, 0.0, 1.0);
}

fragment float4 f_solid(constant SolidUniforms &u [[buffer(0)]]) {
    return u.color;
}
