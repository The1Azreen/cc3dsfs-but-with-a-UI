// HDR output for cc3dsfs: draws the screens into a linear scRGB (FP16)
// swap chain, where 1.0 = 80 nits. The screen filter matches ui/filter.slint;
// "injection" lifts the picture to an HDR brightness so the filter's
// darkening is made up for, like the HDR modes of the Morph 4K / RetroTINK 4K.

cbuffer Params : register(b0)
{
    float4 top_rect;     // x, y, w, h in output pixels; w = 0 when hidden
    float4 bottom_rect;
    float2 top_size;     // source pixels
    float2 bottom_size;
    // Per screen, since each one may be too small for a filter.
    float top_style;     // 0 off, 1 LCD grid, 2 fine grid, 3 scanlines, 4 thin scanlines
    float bottom_style;
    float edge;          // darkness at a pixel edge, per axis
    float peak;          // display peak in scRGB units
    float top_gain;      // scRGB multiplier for SDR white (brightness and compensation)
    float bottom_gain;
    float top_line;      // fine grid line width, in output pixels
    float bottom_line;
};

Texture2D top_tex : register(t0);
Texture2D bottom_tex : register(t1);
SamplerState point_sampler : register(s0);

struct VsOut
{
    float4 pos : SV_Position;
};

// One triangle covering the whole output.
VsOut vs_main(uint id : SV_VertexID)
{
    float2 uv = float2((id << 1) & 2, id & 2);
    VsOut o;
    o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}

// How dark a point is across one source pixel (t = 0 at the edge, 0.5 in the
// middle): 1 at the edge, 0 in the middle.
float soft_profile(float t)
{
    return 0.5 + 0.5 * cos(6.2831853 * t);
}

// Fine grid: a line `line_width` output pixels wide centred on each source pixel
// boundary, snapped to whole output pixels like ui/filter.slint. `s` is the
// output pixel centre's offset into the screen, `cell` the source pixel size.
float fine_line(float s, float cell, float line_width)
{
    float start = round(round(s / cell) * cell - line_width / 2);
    float px = floor(s);
    return (px >= start && px < start + line_width) ? 1.0 : 0.0;
}

float filter_weight(float style, float2 source_pos, float2 offset, float2 cell, float line_width)
{
    float2 t = frac(source_pos);
    float dx = 0;
    float dy = 0;
    if (style < 0.5)
        return 1.0;
    if (style < 1.5) {
        dx = soft_profile(t.x);
        dy = soft_profile(t.y);
    } else if (style < 2.5) {
        dx = fine_line(offset.x, cell.x, line_width);
        dy = fine_line(offset.y, cell.y, line_width);
    } else if (style < 3.5) {
        dy = soft_profile(t.y);
    } else {
        dy = fine_line(offset.y, cell.y, line_width);
    }
    return (1 - edge * dx) * (1 - edge * dy);
}

bool inside(float2 p, float4 rect)
{
    return rect.z > 0 && p.x >= rect.x && p.y >= rect.y && p.x < rect.x + rect.z && p.y < rect.y + rect.w;
}

float3 screen_color(Texture2D tex, float4 rect, float2 size, float style, float gain, float line_width, float2 p)
{
    float2 offset = p - rect.xy;
    float2 uv = offset / rect.zw;
    // The texture is sRGB, so this sample is already linear.
    float3 c = tex.Sample(point_sampler, uv).rgb;
    // The filter darkens in gamma space, like the Slint filter and the LCD
    // grid in Game Sack's footage, so in linear light it is weight^2.2.
    float w = filter_weight(style, uv * size, offset, rect.zw / size, line_width);
    return c * pow(w, 2.2) * gain;
}

// Keep highlights below the display peak with a soft knee instead of clipping.
float3 roll_off(float3 c)
{
    float knee = peak * 0.75;
    float range = peak - knee;
    float3 over = max(c - knee, 0);
    float3 compressed = knee + range * (1 - exp(-over / range));
    return c <= knee ? c : compressed;
}

float4 ps_main(VsOut i) : SV_Target
{
    float2 p = i.pos.xy;
    float3 c = 0;
    if (inside(p, top_rect))
        c = screen_color(top_tex, top_rect, top_size, top_style, top_gain, top_line, p);
    else if (inside(p, bottom_rect))
        c = screen_color(bottom_tex, bottom_rect, bottom_size, bottom_style, bottom_gain, bottom_line, p);
    return float4(roll_off(c), 1);
}
