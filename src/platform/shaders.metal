#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 cell;
    float2 pad;
    float2 viewport;
};

struct Instance {
    float col;
    float row;
    float4 fg;
    float4 bg;
    float glyph;
};

struct VOut {
    float4 pos [[position]];
    float4 color;
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
    o.color = in.bg;
    return o;
}

fragment float4 f_main(VOut in [[stage_in]]) {
    return in.color;
}
