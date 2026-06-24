//! The arch+profile-bound golden (R19). The M3 trajectory hash folds Position+Energy+Genome (incl. the
//! evolved brain weights) + the recurrent `BrainState` (`h_old`/`h_new`) + the motor `BrainOutput`,
//! AND the f32 signal field; both the world heightmap noise (libm `sin`) and the signal field are
//! arch-divergent (FMA-fused in release), so this is specific to **arm64 + release** — the dev/CI
//! matched arch. It runs ONLY on the arm64 CI job (`-E test(v2_golden)`); the x86 job excludes it
//! (`-E 'not test(v2_golden)'`); it self-skips in debug (different float fusing).
//!
//! It is thread-count-independent (Position/Energy/Genome/BrainState are integer; integer inference is
//! associative; the signal merge is canonical-order), so the fixed-N pin is robust. This pin moved
//! from M2 because M3 replaced hard-coded chemotaxis with batched integer brain inference, added the
//! global multi-rate phase (K brain / N metabolism), and folded the brain buffers into the hash — an
//! INTENDED trajectory change (re-pinned on a fresh arm64 release run). The CONSERVED-field 1-vs-N
//! gate (R14) lives in `r14.rs`, outside this namespace, on both arches.
//!
//! Re-pin (single-writer, agent A): on an intended trajectory change, dump a fresh arm64 RELEASE run
//! and replace `GOLDEN`. Never re-pin to silence an unexplained drift.

use cli::{default_config, run};

const GOLDEN: [u64; 384] = [
    0x8f825d723e0cb31f, 0xc9bf434b6c866dfb, 0xadc3904f585f929e, 0x82fcad50e423c79a,
    0x44111334222e4f16, 0x471185b859241fe3, 0xd0f26852910cf726, 0xac3069378bcf0956,
    0x9568af86f7988ac6, 0xed744000dfabaa9c, 0x119c058fd08d3a02, 0x6266aa5791ded22c,
    0x6716dbbf2ce5654a, 0xe8b1e81684927f92, 0x68f64ff2024678ed, 0xcc95e88f42b73ede,
    0xd2cd46c44b5fc140, 0xc6ca9187a9c2c137, 0xa107a7b5ec578a6d, 0x4e3b643b518707dc,
    0xe8e634353b6d1af8, 0xdb6d6a3f59175928, 0x7990f90ab8d9f63d, 0x0d2b0a0512dc886b,
    0x5da6bd1e7b4d2b45, 0xb796fa35884fb06a, 0xd4b57aa0174d06f7, 0xf470e121603d14e8,
    0xa373916f5ccf4910, 0xad800d74c612b5eb, 0x47c1718441cdb6d1, 0x0eb1f43322dceaea,
    0x320cf9ca8eab8aa8, 0xa5a85eb24a06d84c, 0x0cad5fb6f36a866e, 0x6891c31e653f7410,
    0x041a8fb27ea6b5eb, 0x5214900554d73625, 0x3b10ea50d7782d8a, 0x808e789a8b47fd8b,
    0x781d13411152cbb0, 0x1da415f190119159, 0x56d638f583ed5297, 0x51647ce8a0579e78,
    0xe1f2267739725e4f, 0xaf03dc06a83723bf, 0x54a27bfd350c70ce, 0x60a43c267214d1e3,
    0x99f1202f7ee4b5a4, 0x7b44826774bea28c, 0x62d9f76ceeeea1c2, 0xe0fee8709497193a,
    0xcf9411cbc450d7a2, 0x3b8c81674510c96d, 0x633af4d53e2b6f20, 0x255e054ff2197d61,
    0x8d20c841b3f81b49, 0x17581d1b72dd0a5f, 0xc8ba9388668b4dfe, 0x6686e680190d16d5,
    0x173d30bac12ad2b7, 0x52d48c42e271365b, 0x63232ea7ccfb2cd0, 0x7164db383be6d4e4,
    0xe8072849df1fdcf2, 0xa7a20d2e3ffef8e4, 0x9c33cee47a376d8f, 0x392fe90a08e12555,
    0xbfcfd0f450516a91, 0xb338f77c10aa62fd, 0xce87511d12625d1a, 0xbb57479ad91963fd,
    0xcc6181f5bf93ad5a, 0xec7a86cfbcee4cf5, 0xd3348270e01e3123, 0x88db93b73e9807d6,
    0xa67927f29581aebf, 0x9e3004e5f8e6de54, 0x451ad2dcdbab793a, 0x30e92cdb47e196a5,
    0x3f179060d2884359, 0x4791ca47ce815c6c, 0x58b31dcc79a13cd5, 0x9d3e46084513f372,
    0x709633b74a406e77, 0xcb0797cbca756888, 0x1e0aed5679916c6d, 0x24f814f3b9d58bfe,
    0x5359651c385e9a76, 0x29a1a3b7a0135521, 0x3879a63793949b8f, 0x9fa8c1494e4181dd,
    0x4477f283b88f2d50, 0x72f210b943686d76, 0x77735f703c31054d, 0x2594fdd38eb7d01d,
    0x2b1f3b145eccea08, 0xae950bcd50233598, 0xa11a718c0ed6d99f, 0xb6abb7c786f63a61,
    0x0620a27559f1d59e, 0x707982dd8b5e0a7f, 0x5d56320debc5af33, 0xa8972a541698ffcc,
    0x8eb5c228febc39ee, 0x9c02a864125bb1fc, 0x4aa2b0b3e5b6c1d9, 0x6e818a5d11b6d907,
    0x88aaf2304be2e9fa, 0xced3df75fd80a8ce, 0xd4e3f8300ac7c796, 0x9c960b88ac748bda,
    0xbd8386abf027af87, 0x51bf810354eee5fe, 0x441245afd9f1fd1c, 0xd1af30204bf2c3cc,
    0x98709dde2181c5e9, 0xd6382bcfefeeb831, 0x541aef95dee83281, 0x294cc40c10d2e69c,
    0xbeff0ecdda483860, 0x13932c236ec3fec2, 0x53d2a16917c60183, 0x503e710d4cec0464,
    0x2a4cf191fa2190cd, 0x94ad3d6d26b5fdc4, 0xfb201a57a668a7c4, 0x79c8721f2b4f1650,
    0x810b2aab73cbcef0, 0xe3c965992f5de050, 0xf092efd26cbfc31a, 0x77f80e6e7850581b,
    0x8e00cb93bdf8f11e, 0x2097734546bb78ff, 0x473f76c7fa6d7a30, 0xe5ef4b4f752ce1b4,
    0xe8df75173b63151d, 0xac543c1a2fb0ec08, 0x602d8ef7f66320a1, 0xacfa5af51227e398,
    0x98fec5318ae66741, 0x4cc65bc26b3d060b, 0x90eac9a801dbd344, 0x7ec99f97c14c0d22,
    0x43889f541b3abee5, 0x59343727d9019d14, 0x61607379de619543, 0x2a0b1c579eaa00d8,
    0x4b0e610fdc496f1f, 0x1cb4bd4578a6b7a0, 0x3cbbcfc32b11c30d, 0x881044af5699eee2,
    0x2ac52659551d3f64, 0xb8e19cb0eb9e9675, 0x136aae7e03548cbe, 0x8bb71039ad266616,
    0x6f2e24a74f5e12ce, 0xc138a172358b9779, 0x70db0055c7665814, 0xc20c070433a3c425,
    0x887ad3ed2e01dbb8, 0x3d7360a483961922, 0x602a540f4a085fc2, 0x2e2e83a49e8e4833,
    0x77469caf35b369bb, 0x54523c9367e76a02, 0xca8c9ca107a334f4, 0xe067a83aeac636e0,
    0xc28ad573b9314525, 0xfefc61a13847419a, 0xcfeb97a45499851a, 0x4319712139f53afe,
    0xf95af6adf9fdfb13, 0x547a65a71dcef248, 0x70f5be7505fc76fd, 0x3c05a001c029efd8,
    0x1f662a8f85bb15dc, 0x7ff23f435c39b692, 0x90ccdd97351b0072, 0xd0ece2d368cbbd2f,
    0x30a9c1e01820c78b, 0x6cb12caf65c181c1, 0x428cc675e66f9362, 0x569f6d442ff1fd20,
    0x0ec12b10216f133e, 0x0fb131b52eb24346, 0x47f2c14c6ae20766, 0xbb5ab89c77ed4090,
    0xe86cdcc9388f0ce8, 0x24f5c3c80cfb9504, 0x9e132e3c9cf68218, 0x03bb70c557d07c5c,
    0x47a9b6a1e0dcf4f5, 0xf3adfa08565f7949, 0xc68aacc8cbe11976, 0x1b58c4089a29d3f1,
    0x030b40cc25890032, 0xd47d152de2393e1c, 0x479fb0c17ee88b6c, 0xff024fbf88ebbd9c,
    0x89cb6979d2a4f16d, 0x674177431ee0ac47, 0xf61cafc7c1a52a66, 0x846b0147ad64f15a,
    0xbf2ce1906b88242b, 0x0b7ded2ec01984b3, 0x7533377642025d3b, 0xd4db5471bdd80539,
    0xa192e2d54a5271eb, 0xe33116dec9e01820, 0xad45c98a1b19637f, 0x31b28feeaaf6bd1f,
    0x21e62b00d1869df5, 0x1917891a3f73c078, 0x08959918ac11dd55, 0x3404ed1a1948927f,
    0x65072adea570c896, 0xed7fb21a22a14d23, 0xc7fafe17f72e06c2, 0x4570a5b9eb1309ec,
    0xa69d326a1b1cb010, 0xf40c489521598141, 0x9084413eca5d5702, 0x321f058744f4a481,
    0xaae5c39eee3bb303, 0x653639a9be38cd81, 0x1faf714365da85ae, 0x4f9cb0eb1f98456a,
    0x9ebb3765e0b0fa01, 0x8f3c69feb40a9961, 0xac523653783a7730, 0x65659361d44e3d5b,
    0xa3b93ae80d809d96, 0x03ceaa7c487572f4, 0x6abf4264f9987276, 0x63b8816829d7fc7c,
    0x554ef0a03bc024c8, 0x23b70d96d9892ab9, 0x3c1c9e4198fd809c, 0x0e334e328b0ccccd,
    0x674dd09fbf3b1c80, 0x551f9cc6c7c31001, 0x478eb2015840df55, 0x91b1a89126ddf733,
    0x173c69b2985a945a, 0x71db4f1be23f7b7a, 0x154667da0006eae4, 0x7272cfe41d57ee96,
    0x487da0fde16a9c25, 0x9357ed71e55f38d2, 0xd64cfedc2aa668d3, 0xaaef23911dd42f9b,
    0xd062026fa1bd80de, 0xf8b9645f98d85bd9, 0x4be1b6333ba2f62f, 0xb63276b6b70d2bf8,
    0xe00d6108e4e22c2c, 0x5e40bcf4d49ed6ff, 0x391e42c60b4aaeac, 0xd319505b41ce598a,
    0x1b87878a7c0adb56, 0xf1539f45f03afce8, 0xe1c62775fda5082d, 0x74edd160edc69e81,
    0xa55ace07da3970c9, 0x41b408e95cbc5b4c, 0x2c0947c39629be4b, 0x766c25a24ea26d06,
    0x7e0f414e7cb9f255, 0xb38afce973d234a0, 0x24603b63d9a3e38d, 0x7f215c35d51068e2,
    0xe03be8f8059fac0f, 0x81ce435167379b17, 0x9746e0e44bd53178, 0x2a44773dc432a43e,
    0xbe4808bb5939bcfc, 0xdb7b1d947c1cbde2, 0x9ea976b85c4e0224, 0x9fca039c4347647b,
    0x4cf4344f63b9c044, 0xd6ce51d16b67edbc, 0xb942a7fe07c5302e, 0xf553ffaf1c255028,
    0x92b54800b146e7b6, 0x90f72b4b2ec6fa66, 0xd2bd7ed63655c7d1, 0x7110a0e8c322a941,
    0xaa8745faef7b6972, 0x647ca57276addeea, 0x8774e19504403968, 0xc3e2eba6e4b644b7,
    0x7c70618121380242, 0x9e798b90dfbdf1a8, 0x6ae41048a091f2a1, 0x46399bd59428bf8a,
    0x78aa1b9a9eae4ebb, 0xc5e2c680076b99ba, 0x53247dc4cca9ae20, 0x234ef5e9c66c9603,
    0x80d21f630fd90af0, 0x12695f5d60c8af6a, 0xe392e0272c349789, 0x0995528c1b363ef3,
    0x7f8b11f3238a342b, 0x6897464e23d248c1, 0x9c82f25d6c4df4b3, 0x973d0b3d28c35aa6,
    0xa2c2d6688819952b, 0x98ece474aa9bb3d2, 0xe41e50bdda1fb653, 0xd2129ebf53a5eef5,
    0xea8424e20c985341, 0xefd56f4081808f73, 0xadd861f57bdc0ab8, 0x8bc893c943f8e0d1,
    0x07838c1057ebd452, 0x16596126608b75a1, 0x8d7f1affc8cda58b, 0xaff54b08fe456d90,
    0xe9e532c55b69f9b1, 0x1fc780874eec8d59, 0x3b251fa4dff04ba5, 0x0b1f3fe20a0c0a3f,
    0x7f4296f158b390e3, 0xb52cb113722c3d24, 0x7f6a105096fb198c, 0x9f71b8ea29bf0c4e,
    0x1a840b48b2942745, 0xa7e0e0c795d133e9, 0x54969fe1e81f8222, 0x157dfd4359174f5c,
    0xc6534aa00641dd1a, 0x626ec7f9b7b6109e, 0x2826cda35db0b251, 0x7a7bab9b31b6d153,
    0x66709598811b3cd9, 0xa11d628016ca0eec, 0xbd5ff5a35b1fbb0f, 0x408fffa0ee24e758,
    0x6b17d1daf23a15c8, 0x66d56bf6df99145c, 0xf65ec0d573e586fe, 0x05c371997fb68d30,
    0x37e8f1b684e72953, 0x7bbde94f1b74d0cb, 0x5260b0ee53cfb084, 0x4adb4bd8af742625,
    0x630bbafc7ed8bced, 0x912cb65b332a605f, 0x8ce8aa92bb0933c6, 0xd91394885c7811df,
    0xbcb299ee51bd0fb3, 0x1d8dbd0ac249eb39, 0x02c4c4746b5179a5, 0x9bb4aa5d3f4d1b24,
    0xa2626f9105b7d5a4, 0x01b69d242bf4d3fd, 0x9919fa25019d7905, 0x50ea0af9f601edc1,
    0x3be95703bdf9713a, 0x5bff1e53b101dcb8, 0xd7c725101a13b9d7, 0x0354aa5389e85108,
    0x6b4d2b93ba0d0aff, 0x55e3724bd1fd4ebd, 0x3f031ff87fd1283c, 0xd55aa1902b158017,
    0x381fd51ecf39c605, 0x3c483d9b8b7e87cb, 0x160d8c296410c8b9, 0x66e93eb76557ffff,
    0x4e0c00c14fd78d97, 0xcd316814bf1834a8, 0xb7f33d998d15da31, 0x31dcafe3758aba3b,
    0x821c516c71783792, 0x94c0bd6cb127913f, 0x3b69e9a2f088aa09, 0xae86baf724ebea23,
    0xae234c0706306c71, 0xa9c80543312877ee, 0x7c41d92b7c53c578, 0x568b76bdc0144e07,
];

/// Drift-vs-golden, bit-for-bit (R19). Skipped in debug; the arm64 CI job runs it via
/// `cargo nextest --release -E test(v2_golden)`.
#[test]
fn v2_golden_drift() {
    if cfg!(debug_assertions) {
        return; // golden pinned for release; debug float-fusing differs (run via the arm64 release job)
    }
    let h = run(default_config(0xA11A_2A11), GOLDEN.len() as u64);
    for t in 0..GOLDEN.len() {
        assert_eq!(h[t], GOLDEN[t], "golden drift at tick {t} (left=run, right=GOLDEN)");
    }
}
