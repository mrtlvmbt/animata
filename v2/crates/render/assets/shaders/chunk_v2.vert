#version 100
// V2 shader - minimal, no discard, no depth nudge
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;
uniform mat4 mvp;
varying lowp vec4 color;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    color = color0 / 255.0;
    // Unused attribute references to prevent compiler optimization
    color.a *= max(1.0, texcoord.x * 0.0 + normal.w * 0.0 + 1.0);
}
