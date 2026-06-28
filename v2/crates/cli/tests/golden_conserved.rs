//! Pinned per-tick golden for the conserved field hash (A-0 / issue #138).
//!
//! **Purpose**: pre-refactor baseline for slice-A (`CpuFieldStore` layer generalisation). Without
//! this pin, a refactor of the conserved field could silently alter the trajectory and nothing
//! would red: `state_hash` deliberately excludes the conserved field (see `lib.rs`), and R14 is
//! a relative 1-vs-N comparison with no absolute constant. This pin captures the **current
//! scalar (`L=1`) conserved trajectory** so that the A-1 refactor can prove bit-identity at L=1.
//!
//! **Arch**: the per-cell resource caps come from the `NoiseWorld` f64 `sin` heightmap, which is
//! FMA-divergent between x86 and arm64. The conserved trajectory is therefore arch-bound. The test
//! name contains `v2_golden` so the existing CI filter routes it arm64-only automatically:
//! - x86 jobs: `nextest -E 'not test(v2_golden)'` — excluded
//! - arm64 job: full workspace run — included
//! Zero yaml changes needed.
//!
//! **Re-pin** (single-writer, agent A): only on an INTENDED conserved-field change; read the new
//! left/right from `.ci-report/failed.log` (the arm64 job). Never re-pin to silence drift.

use cli::{default_config, l3_config, run_conserved_hashes};

// A-0 pin: conserved-field hash per tick, default SimConfig (seed 0xA11A_2A11, L=1 scalar).
// Captured on arm64 + Rust 1.96.0 (matches the CI `v2-golden-arm64` job arch + toolchain).
const GOLDEN_CONSERVED: [u64; 384] = [
0x25c854ac1e9b3723,     0x63ecea995c7d6807,     0x5ce7ce6ecbaa4b85,     0x7823a28fef1325a0,
    0x0603f02fb04c2f6b,     0x8352082d7968fd82,     0x2fbba161e87b8274,     0x9fd9a4109e30de2c,
    0x3d96a6368a70cc39,     0x898b5bc7354c0867,     0xa6e0251c48c9ef2e,     0xdc400fc20f2dcb2a,
    0xcc4a495d96af4d0f,     0x5386eeb6ddbda6fe,     0x94452c9493c6cb56,     0x1bb323980da64967,
    0x629c17bcfe62e267,     0x63cdeec96a438fdb,     0x7bb856fb6d0c0792,     0x1646a22b81e8ab87,
    0x728f1d1cc35a8971,     0x6ef8bd39d345bd18,     0xbc4de9ae12134696,     0xcbe306f802a24315,
    0x26e5c5c4a9046d51,     0x4c554c8d9075f52f,     0xb8a5032b20b53693,     0x5338706b94d64b05,
    0xea61c0d0f46b5698,     0x095ddf699386d6bc,     0xd17c67c26f532234,     0xbde738e84ee317d4,
    0x02a1b198fd49b359,     0x4bb788aabf88fa7f,     0x6779752a11e087c6,     0x93f5d0d5fb79af40,
    0xfb317cd5e0e88907,     0x1f6501190ade2884,     0xce398674442a123c,     0x42cc449dbd4ed3db,
    0x5933c5f0d0ea3a0a,     0x811edc2db6917498,     0x2814722a3f7bfa0f,     0x50da8187ecd08286,
    0x49328aa695ddde02,     0x6642db3ca506e81e,     0x66d0759c167d1232,     0xa8e39fcf8747f951,
    0xf311079b922ba879,     0x3c70f6d823f09fb2,     0x2893ebd70a13bfe5,     0x7dcecabdbb165ce7,
    0xa8c649a9e6bdefa7,     0x2cd4e5dc00fd1671,     0x48e442f004f8e67c,     0xba3c04ae2f448e0a,
    0x6dc3af4b182c4dde,     0x0b959f3d7ff68dda,     0x91482fd7e2f9abdd,     0x903d8f3d4e7adeec,
    0xca99896fc8a005f4,     0xe8b0c0cba2e21e67,     0x6ad3fad68faa7fb4,     0xe8b37f933ab46b8d,
    0xee6a3f64db81a3d2,     0x9fcf203395b78e89,     0x410d1bee1af04957,     0x70c43c9a1246bc0c,
    0x086a9fa29e1767bd,     0x562d29e477d9a22b,     0x203fa5d25d094098,     0xdc8aa8f647ffeaee,
    0xa05ae36462710a54,     0xbff558c45945e460,     0x265a5a360a71e5f5,     0xa855c095ec7983fa,
    0x5807a5396c5669e7,     0xcbc9fb76461dfcc4,     0x50a77072da00bb29,     0x501ff544d99801fe,
    0xfd471b5738f3b605,     0x6584774cffd6b24e,     0x6e7522502fc1571c,     0xefdfcf65adbaaa81,
    0x461bae373628af19,     0x10ca0fc3bf2e7ba8,     0x8057b4917812f01b,     0x8de4acf605a8bc8d,
    0x75a9f096bbf9e419,     0x6644dc12d1d1f04f,     0x9197b76945e47a94,     0xbfea439919bf68d1,
    0xe0bd80c2f4468ffd,     0xd681e9450c7c2eb9,     0x3c1406d79931038a,     0xb55fd5773941c4de,
    0xa285a24929a02017,     0x00dea02a22388099,     0xe2bc8c3728ce74e1,     0x0d8637c02644c9e2,
    0x229274903e36e19f,     0x4701b3d11e5b2b6c,     0xf53b6ab60ea07f59,     0xd5500f10c509e339,
    0x2a361ceacdfd89b8,     0x51f9a21cc2100655,     0x6cc8f2a8dc8232c5,     0xe307fc6f54ada603,
    0xbef0c3713143c07a,     0x99fd3dcc704e17bf,     0x57adffd8012f6a8d,     0xcad30fced1229df6,
    0xb38b6bf6daca72fe,     0x02a9178e2fcaa09c,     0x8580e4d0d10ed3ee,     0x2613c4a5c232cc92,
    0xd587263d6a046f7a,     0x9411a74b52d1dbd5,     0xed32ed06f6d94f15,     0x7d166029748832a1,
    0x27b59fd1266f6623,     0x4879315880dab9e8,     0x43bcda77bb97e047,     0x95fb2b37539c962a,
    0x70819a2df8d6bee0,     0x80a3460bbb479c5f,     0xb5ab6a6d1f6d6823,     0x1e033137f47d8087,
    0x14e57e6e51b0bcf9,     0x7225ec7fc61b8dcd,     0x3f22befec12512a8,     0x9f441e427c7351b3,
    0x0f6a2182a3d8d526,     0x9351496ad281a6ca,     0xe405c2c49523f5bb,     0x96b686498ed4b9ec,
    0x5eb88b3b42444739,     0xe4fc426553bce254,     0x6b8d6ab01087c653,     0x69cd29705c48cd07,
    0xc18e7b07daa12bd0,     0x488fc3506cc27098,     0xdb700c77803c155c,     0xb551ccc621e9ede7,
    0xe03d4eaa1e3422e5,     0x4837305372ac350e,     0x910c8bea7ed86be4,     0x7508555680ed1a25,
    0x5cc291ca430ef16e,     0xdd475e3d8855aa1e,     0x32846843574ccd23,     0x5e59e805fcecf9b5,
    0x109419c01e4cb5ba,     0x15829725934b8ad7,     0x022ec0f470f34625,     0x1216560265a6accb,
    0x19b000000a27e846,     0x27fd878174398f9a,     0x79f75959d1c8aa80,     0x5cb421f2636114be,
    0x79948c45bde80514,     0xff0f9d6abba77546,     0x250e692e84ae6b01,     0xf724db41e84c749f,
    0xa28771d2eeacec73,     0x0603ea3c44633aca,     0x311804a44aaa0b7f,     0x1c274153866c8c02,
    0x005b8583d9c5a3ed,     0x02ac4845dcf3afb8,     0x38b6d736910f747e,     0x633d1d5fd4d719ee,
    0x8bf12270f3d17dfc,     0xb24ebbbe0825f541,     0x6459b64dcb7b3d33,     0xabb36b74fd719012,
    0xb00858ea4810987a,     0x00ebc7a11adf6028,     0xd785ca6b50692080,     0xed84b0d13fc110bd,
    0x1b2474ebf6139818,     0xe56cc3068822aecd,     0x50bca9c0276693e1,     0xd94abd2eb29a4a39,
    0x3f07c3919b103692,     0x3f943c033d4c166a,     0x0be7483748b3b177,     0x5cd7024574f0dfe2,
    0x01994cb7d84eea87,     0x91a61d38513d4ca5,     0x2e446cb8aa5e32be,     0x74b505936361564f,
    0x13512a96d3596841,     0x2867ec9f96d5c41d,     0x8b15da90a224e1e8,     0x9e6cbba62ae74285,
    0xf16c999dca0f3ae5,     0xb9391075a8528e52,     0xe02eff8e7f6de646,     0xd28df9e48ea3e189,
    0xfd267ed27d7ed648,     0xcef9ec2b0a42f50c,     0x87acadb001d6afee,     0x957e840052fb2b47,
    0x8aa9bce3bba8118d,     0xc1982d200713f397,     0x39a25dc0880cf988,     0xa8105def5ba5f7c5,
    0x0c711c9c72db4304,     0xd1e4b1593af518c4,     0xeb5e714698949728,     0xf56e9b0597a3d8cb,
    0x88e200b72189b034,     0x74682cb05c2e363d,     0x550b604347a7cb06,     0xcefd6faee707096a,
    0x83475269f337dce6,     0xcae6372840563383,     0x3e8bcff738a99977,     0x072d715373723895,
    0x63ed9e2fb6b13e06,     0x3b035824d211bb5e,     0xb17c96aa405a07dc,     0xc9c921bc39503573,
    0xf61f5057f1f37904,     0x39c5dfb372319c11,     0xcbff298d0e4e3489,     0xcd075983a20d87ff,
    0x76c0d0e8df913000,     0xc4a9d74c7d1f4516,     0x160f7cbb168a8abd,     0x5719299ae0070be6,
    0xdd9ea53901ade2fe,     0xdb81c68e32e11af8,     0x9a1615b260858156,     0xe8af42c142d86229,
    0x23a3c12420fff82e,     0xce1f13066669be27,     0x192b993513010a83,     0x38ea8cca3d63048e,
    0xd31a7a791763d0a5,     0x455d450d23066e82,     0x312dd614416c49ae,     0xb83355fe09dc272c,
    0x72fa37f04930cfec,     0x70c7b8a7c74a1c13,     0x908d171520f172dc,     0xde78dc1b7563da1a,
    0x6d8bbbb83e2e7097,     0x697b3f3cfdf4c172,     0xf38c055b93da114e,     0x32d061556494b287,
    0xdc3c8e3432c89278,     0x1ca897788bcdced8,     0x53c6b95292790de5,     0x6cf03896130e4c7a,
    0x93517679458ac71a,     0xda10447d742acf7b,     0xa2ea0e9bc04d8256,     0x2096e63c869c9d22,
    0xfd334c5fd6cd02f0,     0xa732fd4249b5551d,     0x13320109b71666e2,     0xcabccba2626199d9,
    0x8663a7c699fd0b9c,     0x2aaabebf4d9549e6,     0x9ebee3a12db4b66f,     0x9752af75f7c6a213,
    0xa503651128f74694,     0x07d614e52f3fed2a,     0x14fe922c2b3a94d8,     0x765d1b4fc449bf73,
    0x06065068275464e7,     0x03c598985ad187fb,     0xa8bb5ae404001d91,     0x96cda89175ae6669,
    0xeaba6a9096e52703,     0xa3904a64f38222e4,     0x2b37432816b61a52,     0x2803170808b2dfcd,
    0x599cb032145eedf3,     0x5a157c40c54968b4,     0xd98a46c4458d22f3,     0xbcc331c8f7fb3aef,
    0x7c9e10e36a3b8a02,     0x5002b4ad9db185c8,     0xea5dcecd990e9477,     0xa1e82166908f3bf4,
    0x9ee2464a1227b130,     0xf75ae61092f542c1,     0x81994fb6282d9931,     0x26b7e83534f6c210,
    0xcaf38eee42e9cbc4,     0x9a8a9db941d54bd1,     0x9dd46921e2bea197,     0xf0badeefbd6c9d34,
    0x0085d85443017632,     0xa0a5e3784edac25c,     0xf4fae82b90f9f549,     0x8900c2cf393b885b,
    0xfd5ab568e2503f2b,     0x872d1cf5f02c9026,     0x2072629bd485a624,     0xe9a82ba0fe5047b0,
    0xacd2783ca49f6989,     0x70778fd4f7929f0b,     0xf04f67ef2ea9a0b6,     0x295c9166813dd2d3,
    0xdb7c92f09d3ae7e5,     0xe547168bb1ed6430,     0xa1bae74ff4e2af86,     0x0d1a9ad3154627f9,
    0xf41ac60b53b2555b,     0xf1e82246b2280815,     0xd71286e38e94cff2,     0xb1a2a2e6b3e1a386,
    0xf8ad0821e2712dcb,     0xded7e63035f9a2d1,     0x7b6a2cdcb5460b7b,     0xd5db7377c828ad13,
    0xfc19f5ab619e49fa,     0xc0eeff3bdd8f5d92,     0x08157594ae2c090c,     0x493897fbdf0b8975,
    0x2b713634be9e3389,     0xd466088d36685ac5,     0xd6f5ebe747ce5f54,     0x639f81ae1c1e953e,
    0xa5af6aef8f77292e,     0x45d4480f53a85baf,     0xe49d69153dade3f3,     0xf7302d35e302ba77,
    0xfc18702ab1454b0c,     0x65f11c88150adcd0,     0x3bb7a1c79c79b25a,     0x7f5e992a91ceaf9b,
    0x353a464b36871513,     0x91ac8d163d8217e5,     0xd349b5bed5a2e238,     0x452ea52bbba8be7e,
    0x5c7f9f3ddc468090,     0x43b6fd44c68140d5,     0x43a0f16ca8a5e9a7,     0xc97c6b675a37c54f,
    0x1c9008fb443e4317,     0x449d0edcebfe66de,     0x8102a437520ca7d1,     0x77097c1f5838abdd,
    0x9d2d080dda132762,     0xd0823bae6bbfd22d,     0x9c6feac54eceab6b,     0x97ed2ca5af992120,
    0x1184f2b738c30bdb,     0x6184d6b865814191,     0xb446ada1d6233d8c,     0x6ad81ba0bec9b2e1,
    0xe61faaca41a5e99f,     0xea01a2d418422b42,     0x242978cdde034a9b,     0x8d433762d22c9f1f,
    0x3d621b35c1b0287f,     0xabd8a441fdb28afd,     0x4ae3a0dffc5461e9,     0xb524a1d029f60e45,
    0x5d9199c5cae3d0f2,     0x42d1965a0799ce09,     0xcb56a2240d4c702e,     0xe324c313bbb6d9e1,
    0x52d4f59fae1ae106,     0x68045d95e5110d35,     0xe22a53054e81907a,     0xd46c395e0d0172e5,
    0x0151295a3e524a3a,     0x67bc0f77ddb28c09,     0x62841a747beaa22f,     0xe0119f84a5ee47bf,
    0x1bd132f7bb6f728f,     0xc7d87de12db56149,     0x506c169d8e19b655,     0x3e66384b31b54813,
    0xec8148382e480129,     0x88c25b18529248a5,     0x332c911b0119df04,     0xef1c6de043e1e56f,
];

/// Conserved-field golden pin (A-0 / R19). Arm64 + release only (FMA-divergent trajectory).
/// Excluded from x86 jobs automatically via the `v2_golden` name prefix.
#[test]
fn v2_golden_conserved() {
    if cfg!(debug_assertions) {
        return; // pinned for release; float-fusing differs in debug (arm64 release CI job)
    }
    let h = run_conserved_hashes(default_config(0xA11A_2A11), GOLDEN_CONSERVED.len() as u64);
    for t in 0..GOLDEN_CONSERVED.len() {
        assert_eq!(
            h[t], GOLDEN_CONSERVED[t],
            "conserved golden drift at tick {t} (left=run, right=GOLDEN_CONSERVED)"
        );
    }
}

// A-4 pin: L=3 conserved-field hash (all 3 layers folded in layer-major order). Arch-bound: layer 0
// caps come from NoiseWorld f64 sin (FMA-divergent). Captured on arm64 + Rust 1.96.0.
// Re-pin only on an INTENDED L=3 field change; read left: from .ci-report/failed.log (arm64 job).
const GOLDEN_CONSERVED_L3: [u64; 384] = [
0x2f2ba291c16f8348,     0x8a318132aff6e9bc,     0xeb3ef776b94c2e19,     0x6c429f6143851404,
    0x0937c1353a115c39,     0x4256a5b024e48257,     0x4318ba65796a9040,     0x08d1af17426f7f87,
    0xfac8d3e69c93f4a0,     0xb1ccb38b903bfd33,     0xdc73c15c1d500e99,     0x7cffe20ef04c9897,
    0xa2c2430367fbb6b0,     0x453c076da215f097,     0xccce52e8ff3c3d99,     0x5648216667674cc8,
    0xc65d1e5391f6a273,     0xcf0dafabb1ec7a26,     0x405578a594beddb2,     0xb4a8474116f5c9f2,
    0x697ac555a8396501,     0x0a49a64ce566f162,     0x637d5a56a71542e0,     0x1aa5fb7ce645847d,
    0xca64c57060b15fb5,     0xec6c9f14647aafb2,     0xc7bb6d5379c67cea,     0xa599a4bdbe12b558,
    0xa0bec8c5275459a7,     0xf788236acef71183,     0xfeba1c9ec079690a,     0x3d1b758a79d6d2c9,
    0xf95a5616168eb2e4,     0xcde727da1965c91b,     0xf43b98b903bfffbb,     0x8246cd8a59ad05ec,
    0xb3483ee0545fe088,     0x9004608dd24401dd,     0xffb8d292bb39f4aa,     0x770b36c903903c2b,
    0x2ab8afa1891aabc6,     0x29dcb2d98539e6e5,     0x1749d4fab8854b64,     0x17c825f6794c12d7,
    0x1301f21e71e63064,     0xd96adb75deda088c,     0x6554fed9b2042ff3,     0xfc48440c832c1731,
    0x980acdc74b16f6c2,     0x8302d315aea7f96d,     0xf6ab727d980e6ea1,     0x33a950800db524d6,
    0xb69ed4a21e7353b5,     0x2dce310fcd06bfdb,     0x0d540c62e7fb67cc,     0x7b039a53c29f43a2,
    0x4b9016dc2c448060,     0xf1601db611b1b120,     0xc2f85ad69aa424e1,     0xbb36b927301d4bb3,
    0xd0dd6b045b127e25,     0x5efaa76cb3592cab,     0x11feaf0517adac6c,     0xb2973cf1e46638f2,
    0xfd883e0471133666,     0x9ca3590516f68a9b,     0x9132fac9617ffc13,     0x55130f1a0d3e7694,
    0xeeb0f0d591610b72,     0x7017429c8289eca5,     0xc95ef2647eccc292,     0xf5d5f00bf95f43dc,
    0xf6acd2e1bf9176ba,     0xe18ae1524b1a6eb4,     0x9050c268a80ebfa4,     0x8be857619b26abad,
    0xabb69d3b245563b6,     0xd288f500579f4475,     0x5f8ad2a637d44ae0,     0x1059555146392fe7,
    0x10d9ec1af1886db1,     0x6755f772d3acf3bd,     0x7bb161f8b0dc1058,     0x77c43d45f361d2ec,
    0x518a41eb8bb24505,     0xb137ee7596133015,     0x1f68bd06939b92fe,     0x72930d2073d3a579,
    0x4dde64eba7de8e7d,     0xb8aafa25b8ab8c5e,     0x7c54388f6b1a7b84,     0x004411adf6168e82,
    0x17b5cee27b6e2abf,     0xa887e410a9a91dc6,     0x261c0684b330995e,     0xa5cd962a1e50e140,
    0xfeba1d0ee104c127,     0x3299b070157d6bec,     0xb2c6933ca742631f,     0x0c5621ec73297414,
    0x486b4da82ec61aca,     0x886fb756fa496f91,     0xcf73882d1d882310,     0x2a59882dced3606a,
    0xc9f2e412c2199688,     0xe5785e0ad49fb42c,     0xbaa5c4576facdd20,     0xd31ebc2fedc7e105,
    0x7d1513790afc3ec3,     0x2c24b2207442de2a,     0xd65605e3cea861ac,     0x3a9e5b78c2d55291,
    0x710b0320bca6c90d,     0x9330a2c53698a9a0,     0xdef5505ef5226fd3,     0xfa57c4a7feeb2c26,
    0x38e8e9d7581f8c5c,     0xe7489a39ec0fdb57,     0x72ed755dfa6b90ca,     0xbe6ebeea618383f7,
    0x727009723e667db3,     0x817bf05bbd328320,     0x05aa494fec5e4f3d,     0xcdbc60e5e0d363e9,
    0x32c9ce1f8c368705,     0xf47c93b0040cea10,     0xaa8629b708eb7d17,     0xc33fbb9fb767410b,
    0x76f6a09279ba23bf,     0x340a09a6f26637aa,     0x494565b0ffd2e674,     0x231738024a526418,
    0x7b3cfe7246092483,     0x66d53f6ce5ac604f,     0xfd28d7aeee24395c,     0x1dc3dd9a58695c81,
    0x44a579087853c813,     0xa88fd343f36eeed9,     0xb90e933756694a40,     0x46ebcb48e4316772,
    0xda783a17eb8dd943,     0xbd3baa6fe560310e,     0xe0374930437f60de,     0x378ebadd5063813f,
    0x22420b6bbcd1b7f4,     0x573b51bffae2042f,     0x3789a63543957988,     0x142ace9ad5269b59,
    0xc604c36a96e5fb82,     0xbcf86e3b5e4239a8,     0x01f18133b25544e2,     0xeb3386021e833767,
    0xa531ecf451c4e745,     0x2a60492130391ac7,     0x81b1adbfeea38784,     0xbbf3e6ea5d37dbad,
    0x90463277b0a31a3d,     0x7c7ca1d9aa88c6bb,     0xde13760002101988,     0xbdc46612eeb27098,
    0x93832450ec5855df,     0xcac1bb429d4462d5,     0x808fb18d36153927,     0x58d2a4500d4363fa,
    0x5d69244adc3a110a,     0xb6b578b8d6676a50,     0xdea7d4db5af2becb,     0xcb044320bafb987b,
    0x07f5822ac0dc3d25,     0x37d663c9bf1e9afc,     0xc37a0cec57159b30,     0xd0cfc1549ea2e407,
    0xde6117738a4b72a4,     0xe19ecc1e1265e36b,     0x427243a98a11029a,     0x82e00045fa455ec6,
    0x87ea4140608c254f,     0x4ff74b6913d46564,     0x6d1dfd22588d0793,     0x96695bfbd057bc92,
    0x1b036dcbac27998d,     0xdfbec231b92741b9,     0x8702f12324a8b316,     0xbe3e4889ee205bc8,
    0x349f16e5514c17a6,     0x4901102ebc3c3c24,     0x57271ad6f4fd9d2b,     0xf78c8c51337362c4,
    0x768905c047d0402d,     0xbeee4667f76e46fe,     0x88c29040f20ec17b,     0xd5e5eb660f87a365,
    0x5cd552e6d619bc1d,     0x516de3cc172d7de9,     0xaa6e25f4dc604982,     0x81cf518e71ca671f,
    0x03c767548ee161ea,     0x951b8ab3ec4f6711,     0x98aadce326778ba3,     0xa3d9b0172b5fd513,
    0xc755b384d0cf6d18,     0x364773bbdfcb7a12,     0x8fc1bd354f10a7f3,     0xc166778096056f23,
    0xedce4af73fa9b09e,     0xb290aab9224e159c,     0x789e5593c8ad6600,     0x6e3070fb85141b44,
    0xc938a2f1affdeafd,     0x89dd730389defe64,     0x760c158a8897cef5,     0xbfa19ec759cf32d7,
    0x6881af3df24b0738,     0xcafea5b36253ad66,     0xb8f7278e3b899f83,     0x8e137460ac3caae3,
    0x6efc283de14c87cf,     0x0ad687b96e2245e1,     0x3e5ef397cb1b63a5,     0x485cd812c931dedc,
    0xac2fe8e40ebbfaee,     0xb9dd1959343ea5ef,     0x03788d3aa0e62ca4,     0xb29255661b56e001,
    0xd4c83b8130c15928,     0xe7281f71985d1718,     0xd076aba2acbe0fc7,     0xb95485bd40c6c0f4,
    0xb9c53c22f79ac139,     0x8559dd2824354991,     0x2d1ef92327649737,     0x6fa50ce189ce05c7,
    0x75e29da80784e310,     0xf551a2d15b323842,     0x38e0171195a9e6ca,     0x39c4cf12e8f90b80,
    0x2a565b70ea6225ca,     0xc1a45843eb52d7b7,     0x042b186d87ad9cb3,     0x31d1d36439ab083e,
    0xe812059d50dfd69b,     0xa2ae5d0479e55666,     0x3648bf30c00aa48c,     0xb40e60c3354c8173,
    0xbc5226e55c398f8b,     0xb421ccad7aaaed4f,     0x6cee1ec7f2c68358,     0xe9407ac2a2c49fa2,
    0x1e95bb3022329252,     0x5b3fbd9819c4c950,     0x4457e02b5456fa76,     0x8193ffc5f5852eb6,
    0x9efb4ac6e6255cf8,     0xa71e32af8e83e90d,     0x16cae20873829302,     0x20d88b26c5636c32,
    0xfecc2b06b55c9b08,     0x8dbc6bdbdf97460e,     0x828f91dc510e37c7,     0x15fa6751095a1516,
    0xd83695e938c4cdf0,     0x481f7ad0e13ede39,     0x0c398d978dde3457,     0xf1df7a4c9225237a,
    0x158780136954085b,     0x6bd956ad8635fc5d,     0x860d8c7106a937f2,     0x9e13ac0663ed962a,
    0x17080d12ad664b9b,     0xab9264c357a83ba5,     0x86c0559cb8196981,     0x2dfe1e034888d891,
    0x543a762e0d4b1bac,     0xb762e4d6e3d8fa4e,     0x61e82a24a796c1c6,     0xf1f86bd87dea1c0e,
    0x0199b3ab41e5cb07,     0xbdfcf010fb58dd66,     0xd101fcc98060f526,     0xe97f45133cecd385,
    0x92850aedc300ee92,     0x3fb96f706e046827,     0x2af7e340f5c85bed,     0x73dcd3eea601ec45,
    0x12f1af5d8f11d994,     0xb78e769dfb763a92,     0x9427e21cad67a966,     0xf464b5adb5f1047b,
    0x1259dc3e7711937d,     0x68dd83e473f63c25,     0xa9d0bd0ae6b884cc,     0x02fdf3d239c02b1f,
    0x3db7a2b2dfd52e6f,     0x0b11b75d0b19f9b5,     0xcf88a131484ff12e,     0x3d72aa53b4ec0728,
    0xf8d8e240060e276d,     0x36a97ae9b4677e17,     0xe39b511044990593,     0x3f8d0a66c0b55d71,
    0xa290007f5859e9fa,     0x2bfe51c730e87683,     0xceb245ad05e56f17,     0xff2af05e40e49628,
    0xa003b67ab0e292d5,     0x6741ec4c6829580a,     0xf5d3eb4d2e3242e2,     0x3210fe7bf19fe30f,
    0x3f168d9b12523db7,     0x88cc6aa44e6baa33,     0x2151dccfda2456ac,     0x2ca41116280e9692,
    0x157d30091f4e78b2,     0x5b35eacadfabc808,     0x52f57b0e12ffacbb,     0xd4862bc906477a8d,
    0x4b8581cd2838814e,     0xb395221f66fab8fe,     0x0d15bb18ca186c36,     0xa57828dafdb0616d,
    0xd5c6ea09ec17a8a9,     0xf6884708ec3e88de,     0x58ea40d62110258e,     0x5c30284d75edae62,
    0xec92c608aba2dc28,     0x9fa6476fe0748f79,     0xabed83c535308fd0,     0xb5d92f8355ac4a20,
    0x1e99b312d3b4e4e0,     0xe383955776236c89,     0x34fcd7b74ba74f3e,     0xa8bef015a91a2959,
    0x6ade853a639e5752,     0xdef1e65b1a91f592,     0x8a31d99293700be6,     0xa7512e0e6065d349,
    0xf292452990ab32bb,     0xc97d5c290772f4de,     0x63bd986f8e2ec1ed,     0x7f894cd91638b991,
    0xcc86dceac06237e5,     0xae450740de0c7c02,     0x8b69bc25b5374b21,     0x6ed4412db5587f62,
    0x5a2835cc7267c3e2,     0x598de675f38f44ba,     0x924498608bba1c90,     0x5a9338c8066e8df4,
    0x24b8b3128a57a671,     0x5ed0fe17e82d82ab,     0x8fc1da5625de4c04,     0xa45034ad2a7bdb55,
    0x433c76626b4141de,     0x6aee493218fd7282,     0xcc583f32da6eb7c5,     0xf19e888cbf968bbc,
    0x981860ce3bff9e42,     0x00b8c4f85beb9563,     0x7040df12316d51ad,     0x986a6f2ba797d8a2,
    0xe7df3e7d8a65e7e8,     0xa54f5d93220d6f55,     0x41f92daecb8da6b3,     0xdba61e55f9cddd2d,
    0xb8ffc326e558ae44,     0xcf6e383ae3f33ca2,     0x8f0e4e65441bce34,     0x0ce92dfc97c04c5c,
    0xf4969d91661318c6,     0x9d4259e7d3d84fab,     0x171a46e96b737831,     0x5ff15041ba7fc6ae,
    0xcfb310eb277f94ab,     0x94b9f5ce48d0e5c8,     0x9673930c3c50154b,     0x87b1b6fcebdd6441,
    0x3b8d55fbf55ebbfe,     0xda7ace78f29803f9,     0x278f4c658ac8b251,     0x1702129dbb49e99e,
    0xbbcc199593d8c91e,     0x1ca42e72a289adb3,     0x3df1101e965b89a5,     0x3d32bedf23dbf4b6,
];

/// L=3 conserved-field golden pin (A-4). Arm64 + release only. Separate constant from L=1 so the
/// default config guard (`v2_golden_conserved`) is never touched by L=3 changes.
#[test]
fn v2_golden_conserved_l3() {
    if cfg!(debug_assertions) {
        return;
    }
    let h = run_conserved_hashes(l3_config(0xA11A_2A11), GOLDEN_CONSERVED_L3.len() as u64);
    for t in 0..GOLDEN_CONSERVED_L3.len() {
        assert_eq!(
            h[t], GOLDEN_CONSERVED_L3[t],
            "L=3 conserved golden drift at tick {t} (left=run, right=GOLDEN_CONSERVED_L3)"
        );
    }
}
