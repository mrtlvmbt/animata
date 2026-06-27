# Sources & Further Reading

Primary literature, reference implementations, and high-quality write-ups behind each subsystem.
Curated June 2026.

## Plate tectonics
- Cortial, Boniface, Peytavie, Galin — **Procedural Tectonic Planets** (Computer Graphics Forum, 2019). The art-directable "approximate the phenomena" approach. https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.13614
- Viitanen — **PlaTec / pyplatec**: lithosphere collision simulation with rasterized plates (subduction, aggregation). https://github.com/Mindwerks/plate-tectonics , https://github.com/Mindwerks/pyplatec
- **tectonics.js** — in-browser 3D plate tectonics; readable model of crust age + boundary types. http://davidson16807.github.io/tectonics.js/blog/news.html

## Heightmaps & noise
- Iñigo Quílez — **domain warping** and terrain noise articles. https://iquilezles.org/articles/warp/ , https://iquilezles.org/articles/fbm/
- Musgrave et al. — *Texturing & Modeling: A Procedural Approach* (ch. on hybrid/ridged multifractal terrain).
- **OpenSimplex2** — patent-free simplex-style noise. https://github.com/KdotJPG/OpenSimplex2
- Red Blob Games — **Making maps with noise functions** (elevation redistribution, masks). https://www.redblobgames.com/maps/terrain-from-noise/

## Erosion — hydraulic (visual)
- Mei, Decaudin, Hu — **Fast Hydraulic Erosion Simulation and Visualization on GPU** (Pacific Graphics 2007). The pipe-model / shallow-water grid method. https://ieeexplore.ieee.org/document/4392715/
- Hans Theobald Beyer — **Implementation of a method for hydraulic erosion** (droplet model, B.Sc. thesis; the canonical droplet reference).
- Sebastian Lague — **Coding Adventure: Hydraulic Erosion** (video + code; droplet method). https://github.com/SebLague/Hydraulic-Erosion
- bshishov — **UnityTerrainErosionGPU**: hydraulic+thermal via shallow-water compute shaders. https://github.com/bshishov/UnityTerrainErosionGPU
- Jákó — *Fast Hydraulic and Thermal Erosion on the GPU* (CESCG 2011). https://old.cescg.org/CESCG-2011/papers/TUBudapest-Jako-Balazs.pdf

## Erosion — fluvial / landscape evolution (geomorphic)
- **Stream Power Law** background: `E = K·Aᵐ·Sⁿ`, detachment- vs transport-limited. https://esurf.copernicus.org/articles/2/155/2014/
- Braun & Willett (2013) — **implicit O(n) FastScape solver** for the stream-power equation (the standard stable LEM stepper). https://www.sciencedirect.com/science/article/abs/pii/S0169555X12004618
- Yuan, Braun, Guerit et al. (2019) — **SPL with sediment deposition** (transport-limited). https://agupubs.onlinelibrary.wiley.com/doi/full/10.1029/2018JF004867
- **Badlands** — open-source basin-and-landscape LEM (uplift + fluvial + diffusion). https://github.com/badlands-model/badlands
- Cordonnier et al. — **Large-Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion** (EG 2016): couples uplift map to stream-power erosion for game terrain. https://www.researchgate.net/publication/292192025

## Rivers, drainage, lakes
- Barnes, Lehman, Mulla — **Priority-Flood: optimal depression filling & watershed labeling** (2014). The fill/flow-dir algorithm. https://arxiv.org/abs/1511.04463 , code: https://github.com/r-barnes/Barnes2013-Depressions
- Tarboton — **D∞** multiple-flow-direction method (1997).
- Génevaux et al. — **Terrain Generation Using Procedural Models Based on Hydrology** (SIGGRAPH 2013): build terrain *from* the river network. https://hal.science/hal-01339224/document
- Derzapf / Guérin / Galin — **River Networks for Instant Procedural Planets**. https://www.researchgate.net/publication/230363605
- Red Blob Games — **Procedural river drainage basins** (approachable). https://www.redblobgames.com/x/1723-procedural-river-growing/

## Meanders
- Howard & Knutson — **Sufficient Conditions for River Meandering: A Simulation Approach** (Water Resources Research, 1984). The curvature-driven migration model. https://agupubs.onlinelibrary.wiley.com/doi/pdfdirect/10.1029/WR020i011p01659
- Sylvester — **meanderpy**: clean Python implementation of Howard–Knutson migration + cutoffs/oxbows. https://pypi.org/project/meanderpy/
- Peytavie et al. — **Authoring and Simulating Meandering Rivers** (ACM TOG 2023): interactive vector meander authoring with cutoffs/avulsions. https://dl.acm.org/doi/10.1145/3618350

## Voxels & meshing
- Lorensen & Cline — **Marching Cubes** (SIGGRAPH 1987).
- Ju, Losasso, Schaefer, Warren — **Dual Contouring of Hermite Data** (SIGGRAPH 2002): sharp-feature meshing via QEF.
- Gibson — **Constrained Elastic SurfaceNets** (1998); modern write-up: https://cerbion.net/blog/understanding-surface-nets/
- Lengyel — **Transvoxel** algorithm: seamless LOD between voxel chunks. https://transvoxel.org/
- Miguel Cepero — **Procedural World** blog (voxels→polygons, dual contouring in practice). http://procworld.blogspot.com/2010/11/from-voxels-to-polygons.html

## General PCG
- See the sibling `pcg-engineer` skill for noise basics, WFC, scatter, chunk streaming, and seeding.
