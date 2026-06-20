#version 100
varying lowp vec4 color;
varying highp float vy;
varying lowp float rim;
uniform highp vec4 dbg;
void main() {
    // dbg.z = outline on. A contour-overlay frag (rim) is discarded when it's off, baring the
    // face behind it (the strip is a nudged overlay, so there is always geometry under it).
    if (rim > 0.5 && dbg.z < 0.5) discard;
    if (dbg.y > 0.5) {
        // WATER/LAND mask debug (key J): every opaque column flat grey = "land". The water
        // pass paints flat blue over the columns generation flagged as water, so a dry cell
        // that SHOULD be flooded shows through as a grey hole inside the blue.
        gl_FragColor = vec4(0.62, 0.60, 0.54, 1.0);
    } else if (dbg.x > 0.5) {
        // Quantise to integer levels and colour each by height, with STRONG per-level
        // brightness alternation + a dark line every 5 levels — so every cube step reads
        // as its own band (a topographic-map look). Waterline is at level 6.
        highp float lv = floor(vy);
        highp float t = clamp(lv / 40.0, 0.0, 1.0);
        highp vec3 c = mix(vec3(0.03, 0.08, 0.35), vec3(0.10, 0.65, 0.85), smoothstep(0.0, 0.15, t)); // depth -> shallow
        c = mix(c, vec3(0.92, 0.86, 0.55), smoothstep(0.15, 0.20, t)); // shore
        c = mix(c, vec3(0.28, 0.66, 0.28), smoothstep(0.20, 0.42, t)); // lowland
        c = mix(c, vec3(0.82, 0.74, 0.30), smoothstep(0.42, 0.60, t)); // hills
        c = mix(c, vec3(0.58, 0.40, 0.28), smoothstep(0.60, 0.80, t)); // mountain
        c = mix(c, vec3(1.0, 1.0, 1.0), smoothstep(0.80, 1.0, t)); // peak
        // MONOTONIC by height (no per-level brightness flip — that read as false
        // alternating ridges on a smooth slope). Only a thin dark contour line every 4
        // levels for scale, like a bathymetric map: a bowl reads as a smooth ramp.
        highp float contour = (mod(lv, 4.0) < 0.5) ? 0.62 : 1.0;
        gl_FragColor = vec4(c * contour, 1.0);
    } else {
        gl_FragColor = color;
    }
}
