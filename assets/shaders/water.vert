#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
uniform mat4 mvp;
varying highp float vDepth;
varying highp vec2 vWorld;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    vDepth = texcoord.y;     // water depth in voxel levels
    vWorld = position.xz;    // world coords → animation is seamless across chunks
}
