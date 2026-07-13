task: #436 R-15a retained GPU buffers for v2 terrain
phase: blocked (parity criterion FAILED - GPU output differs from macroquad path)
blocked_on: Vertex format/layout mismatch causing incorrect rendering (darker, speckled noise). Root cause: VertexAttribute layout not matching macroquad::models::Vertex byte layout. Needs investigation of struct alignment, VertexFormat::Byte4 behavior, shader binding.
next: Investigate vertex struct layout; verify VertexAttribute byte offsets match actual Vertex fields; check if Byte4 needs normalized flag; review shader binding
updated: 2026-07-13 16:30
