#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 cell;
    float2 pad;
    float2 viewport;
    float2 cell_uv;
};

struct Instance {
    float col;
    float row;
    packed_float4 fg;
    packed_float4 bg;
    packed_float2 uv;
};

struct VOut {
    float4 pos [[position]];
    float4 fg;
    float4 bg;
    float2 uv;
};

vertex VOut v_main(uint vid [[vertex_id]],
                   uint iid [[instance_id]],
                   constant Instance *inst [[buffer(0)]],
                   constant Uniforms &u [[buffer(1)]]) {
    float2 corner = float2(vid & 1, (vid >> 1) & 1);
    Instance in = inst[iid];
    float2 origin = u.pad + float2(in.col, in.row) * u.cell;
    float2 px = origin + corner * u.cell;
    float2 ndc = (px / u.viewport) * 2.0 - 1.0;
    ndc.y = -ndc.y;

    VOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    o.fg = in.fg;
    o.bg = in.bg;
    o.uv = in.uv + corner * u.cell_uv;
    return o;
}

fragment float4 f_main(VOut in [[stage_in]],
                       texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler s(coord::normalized, filter::nearest, address::clamp_to_edge);
    float coverage = atlas.sample(s, in.uv).r;
    float3 rgb = mix(in.bg.rgb, in.fg.rgb, coverage);
    return float4(rgb, 1.0);
}
