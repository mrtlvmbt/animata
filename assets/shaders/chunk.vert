#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
uniform mat4 mvp;
varying lowp vec4 color;
varying highp float vy;
varying lowp float rim;
void main() {
    gl_Position = mvp * vec4(position, 1.0);
    // texcoord.y flags a contour-overlay vert (1.0 = the dark edge strips); the fragment
    // shader hides them when the outline toggle is off. Terrain/tree faces leave it 0.0.
    rim = texcoord.y;
    // texcoord.x is a per-vertex depth nudge flag (-1/0/+1): a face's TOP edge is pushed a
    // hair toward the far plane (+1) so the column's own top face deterministically wins
    // their shared rim (otherwise they z-fight into dark corner speckle, whatever the
    // depth precision); a tree's BOTTOM edge is nudged forward (-1) so the trunk wins the
    // tie against the ground it stands on. The nudge is far below one voxel.
    gl_Position.z += texcoord.x * 0.00012 * gl_Position.w;
    color = color0 / 255.0;
    vy = position.y;
}
