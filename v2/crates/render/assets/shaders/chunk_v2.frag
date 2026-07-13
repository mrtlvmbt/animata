#version 100
varying lowp vec4 color;
void main() {
    // V2 shader test: no special logic, just pass color through
    gl_FragColor = color;
}
