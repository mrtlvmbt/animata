#version 100
uniform highp vec4 params; // params.x = time
varying highp float vDepth;
varying highp vec2 vWorld;

highp float hash(highp vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}
highp float vnoise(highp vec2 p) {
    highp vec2 i = floor(p);
    highp vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    highp float a = hash(i);
    highp float b = hash(i + vec2(1.0, 0.0));
    highp float c = hash(i + vec2(0.0, 1.0));
    highp float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void main() {
    if (params.y > 0.5) {
        // WATER/LAND mask debug (key J): flat OPAQUE blue over every flagged-water column.
        gl_FragColor = vec4(0.16, 0.42, 0.70, 1.0);
        return;
    }
    highp float t = params.x;
    highp vec2 p = vWorld * 0.20; // ripple scale (world units per noise cell)
    // Domain warp: two slow scrolling noise layers warp a third → organic, living surface.
    highp vec2 q = vec2(vnoise(p + vec2(0.0, t * 0.10)),
                        vnoise(p + vec2(5.2, t * 0.12)));
    highp float n = vnoise(p + 1.3 * q + vec2(t * 0.06, -t * 0.05));

    // Depth colour, QUANTISED into bands (cel look): deeper = darker + more opaque.
    // NB depth is in INTEGER voxel levels, minimum 1 (a submerged column has wl > h), so
    // there is no depth→0 shore ramp — the shallowest water is depth 1.
    highp float absorb = 1.0 - exp(-vDepth * 0.16);
    highp vec3 shallow = vec3(0.24, 0.62, 0.74);
    highp vec3 deep    = vec3(0.02, 0.17, 0.40);
    highp float db = floor(absorb * 4.0) / 4.0;
    highp vec3 col = mix(shallow, deep, db);

    // Toon ripple: quantise the noise into 3 brightness steps.
    highp float lv = floor(n * 3.0) / 3.0;
    col *= mix(0.86, 1.14, lv);

    // Bright contour "crest" lines on the band boundaries (animated, since n moves).
    highp float edge = abs(fract(n * 3.0) - 0.5);
    highp float crest = smoothstep(0.47, 0.5, edge);
    col = mix(col, vec3(0.82, 0.93, 0.99), crest * 0.35);

    // Broken foam: ONLY the shallowest shore band (depth ≈ 1), and only on noise crests, so
    // it reads as scattered flecks at the water's edge — not a white wash over shallow lakes.
    highp float shoreband = smoothstep(2.4, 1.0, vDepth); // 1 at depth 1 → 0 by depth ~2.4
    highp float foamy = shoreband * smoothstep(0.58, 0.82, n);
    col = mix(col, vec3(0.92, 0.96, 1.0), foamy * 0.6);

    highp float alpha = mix(0.62, 0.94, db);
    gl_FragColor = vec4(col, alpha);
}
