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
    float2 px = float2(in.x, in.y) + corner * u.cell;
    float2 ndc = (px / u.viewport) * 2.0 - 1.0;
    ndc.y = -ndc.y;

    VOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    o.fg = in.fg;
    o.bg = in.bg;
    o.uv = in.uv + corner * u.cell_uv;
    o.loc = corner;
    o.flags = in.flags;
    return o;
}

fragment float4 f_main(VOut in [[stage_in]],
                       texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler s(coord::normalized, filter::nearest, address::clamp_to_edge);
    float coverage = atlas.sample(s, in.uv).r;
    float3 fg = in.fg.rgb;
    if (in.flags & 4u) fg = mix(in.bg.rgb, fg, 0.55); // dim toward background
    float3 rgb = mix(in.bg.rgb, fg, coverage);
    if ((in.flags & 1u) && in.loc.y > 0.90 && in.loc.y < 0.96) rgb = fg; // underline
    if ((in.flags & 2u) && in.loc.y > 0.48 && in.loc.y < 0.54) rgb = fg; // strike
    return float4(rgb, 1.0);
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
