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

// Re-pinned after #121 hardening (intended trajectory change):
// F5 — conserved_gradient now uses no-flux idx_nf instead of toroidal rem_euclid at domain edges.
// F6 — Velocity added to state_hash (closes the gap where two states differing only in velocity
//       hashed identically). Both are intentional behavioural corrections, not noise.
const GOLDEN: [u64; 384] = [
0x8922901ed218b8c6,     0xbbe3ce5a10fdbb23,     0xe6f35a48ec334323,     0x5c95eff7c6d3a9c2,
    0xf26bbc7c0865c751,     0xe861f49e238444d3,     0xaf80f7bcb2a116ab,     0xc0be68347208b2cc,
    0x909aee0ec8cb1696,     0x0eb9861a1410ef47,     0x25c1b0c371ddc31f,     0xc04103001b028d9b,
    0xc09c0cf0915181c9,     0x5c3f40f21ecf15a6,     0x52c5e0a582dccea6,     0x4832a91ce36f8acf,
    0xf843af338a41d232,     0x419ac58c34fa3fdb,     0x1c71dddabfcb133f,     0x2ff50d7bda2b5c04,
    0xa92938f8f305c075,     0x2f3719771f9960ee,     0x542a9668eb50132c,     0x06b486fa6df3eacd,
    0xa32fffd5148ea0c8,     0xcfea4000056d1adc,     0xea95930f50b1dcee,     0xb4e7b0ccfffb633e,
    0x0a1842508ed2ae2b,     0x89586d36a91150e8,     0xe3f513c6c75232ae,     0x1571234bff6acc0d,
    0x813998f3007d73cd,     0x6efe91037f103716,     0xa17d6bf2ad4a7b83,     0x7b36d9675750c224,
    0xced9ecc88b021eb4,     0xe3285a44962e6df8,     0xcdf2db9deac8adcc,     0x7257a3b73b384057,
    0x1f80adb4eac2e847,     0x1c925459df06fa6d,     0x381156e1c0c5333e,     0x29ef876edbe177be,
    0xca3faf4389a747e7,     0x70a7fc04c3c544c2,     0x1fe9877d62602776,     0x38e766dadffca30e,
    0xcf35dfb7217cec3c,     0x261fa3cd36b57c58,     0xb52965304ff99aa3,     0x2f8eec0993c3da40,
    0x4ede9742257285c4,     0xedd14a379a098d21,     0x460bd218ddcac0ef,     0xd596c62fcf705087,
    0xf3075cf13a2f32d5,     0x0636c83c5b23cc88,     0xcfa252e8cfb0f2ce,     0xfb66e7b881747378,
    0xd5b3a6cb6b521388,     0xc10aca3d4faa4d23,     0x72f1608ededf564c,     0xe98d014d7a3b9e67,
    0xd86709cc8eebb9b6,     0xd9d3759cfcd73047,     0xd5dfb716da39a3de,     0x2310643cedcb94fc,
    0xbf19742512dbeeac,     0x86f3c2eff7a5f649,     0xe246dbe72a0a1bcd,     0x27f8bd7655c13495,
    0x8a5442ca753374fe,     0x0fcd9a4575d7024a,     0xc7f201b2ed851063,     0xda68c8bb0cb454e9,
    0x3e645fc7ebcec221,     0x4248642470b18643,     0x841e2457334d6d87,     0x391082722ebbc379,
    0x32ef91bbd87eaae9,     0x1171e24b0cfd54d5,     0x9baf2b53688caa37,     0x5004d8ee07ed214d,
    0x4c764e04d48481de,     0x1d0f2ae50e2a2ae8,     0xaf729aba03697114,     0x71ccbbf71b852798,
    0x8bea99609aca72ed,     0x3f431fdc37d77621,     0x4c12026ab3d37caf,     0x480f53c6fbfdea22,
    0x6d095cddc6cce697,     0x1b6ff6ed66ffff08,     0x5afb12a0125b713e,     0xa5d6dfd61b052956,
    0x9694065a5471d7fc,     0xeadbf98d4beb366e,     0x222382a995cb2b8e,     0x5b34a32f794e7751,
    0xd28bd82ceb867741,     0x188632eef93c7378,     0x7d6bd7a9ee878611,     0xd4068fbb4ea8f860,
    0xa7641a0c0606547a,     0xaa90946ab9b9796c,     0x84cb2393dd237d23,     0x9f0a96b3ff9dbe0e,
    0xbe270031e9524778,     0x2b2cc802e8b2d91c,     0xb7feb523c894f36f,     0x929ae1bbe769325a,
    0x2f90c57acb3c1869,     0xfa7fd45672df1d72,     0x6c551b1281f7d684,     0x6c47d3011a51fbd5,
    0x241b2d9ec57704e8,     0xc105a97f35bb22ae,     0xec19312136ab2d50,     0x0e625e95dc642f8d,
    0xccf8d71407e9e573,     0x95205a98572c231e,     0xac1060a506959a4c,     0x265de159dfa7754a,
    0x206cce08657817ea,     0x86d130c98b5ef397,     0x6885b344fbf38e41,     0xf896b3009ea38e65,
    0xa07551e5d32ad4f1,     0xbef7b5d61180b5b3,     0x50c11dd145b5e57a,     0x58844fc7acd73723,
    0x8fd9dac82dd9181a,     0x6468405bf8f841c4,     0xbaee25d8ba8ba220,     0x0aab4a0a08964d58,
    0xdac1e2d962aeeef3,     0x656c01f626c63b53,     0xfb4ef58c9dcd3d2f,     0x56b29d8d8b133099,
    0xe32eb084315935e5,     0x562bb8deb4bde078,     0xaa71b973f2df86c9,     0x419d588f32eeca6d,
    0xe9f7490dfd8677f5,     0xdddadec63377936c,     0xfef6d85ed55e94b1,     0x5e191ed4886a20d3,
    0xf28c8d704c2a1b50,     0x05ab94e9724200f1,     0x44d9a88001a85234,     0x4ca357afb78d8b09,
    0x81bf9dbe0c3405f8,     0x98dc5a415e35c93e,     0xec220713c0a2ecf3,     0x9b6673c4d494e642,
    0x80e7233e44eee6cf,     0x0e0fab98263d6069,     0xfa5fbad295683c1a,     0x92cb1740454bdf14,
    0x8ee8ae0dcce68f32,     0x9dc374beabf3fe8d,     0x9b1eea52409eba3c,     0x8377178f32c554fe,
    0x3527407f9d1f5d51,     0x6970bf429741add1,     0xbb86318b59de01d9,     0xf55b886c8964472e,
    0x5ba59c8291d67ebe,     0xc280b8ce446698de,     0x815ca7da4ae4cd82,     0x1ddacb5297684e59,
    0x4412a2aa576581a2,     0xfc8fb2e782436887,     0x7911f59ca400f41f,     0xd4d931a5b6cd0773,
    0xaed2dd27ba1228d1,     0x6ef28df2a08c0ef9,     0xa204eb34c3e44457,     0xd6740d7ebd10efa3,
    0x1a50390ee10b5482,     0x6b5b7528bf6fddb6,     0x47f61e81d7559909,     0xad1822fbccc67818,
    0x483f3e46df989fbe,     0xc73d50dd4baa9643,     0x8d84d06d865e2767,     0x0ca5b48ba9e77468,
    0xcf386849b12b8f7c,     0xc32bffd3a2207c17,     0xbb6b799fb84014f4,     0x37cbe0541fd733e2,
    0x00b6773fe80346a1,     0x9f4d0cad6fb1d05b,     0xe6ecbe50ead33d99,     0x05862d95b946674c,
    0x414105a3b8345892,     0x8bd0f85fe5e26d9c,     0xa4715e1874fc78df,     0xd4820fddf7ea587a,
    0xc94af29cde3ed713,     0xf1b3393a0659bd1e,     0xb9215ded683f80c7,     0xd4d64b9a1e1f6f6f,
    0xc9ab7182f513f76c,     0x43b473c8931e12e9,     0x630950ccc549ff4f,     0x564aa43674126c3b,
    0xe1b22d4c51a597d6,     0x6f34ad86644a67bb,     0x601bfc14ddf498a8,     0xaa91487241a8d036,
    0x1d796175fff192be,     0x970b66b281c414f6,     0xd60cf8805c6eb227,     0x16c0d07d6d511556,
    0x36fb0823b8f98cc6,     0xcf6a29efb36c99e1,     0xbc5c0c70f5be9cf9,     0x169ef8f3f3613583,
    0xd481d6fad23a0b64,     0x976a7175c3103dc4,     0xcf89567f988b9559,     0x0781c9b5de082eb6,
    0xcf25ea8b8557d51b,     0x864409316bb5cc3d,     0xd07ab8efad380264,     0x7de84ecaa459b961,
    0x4a5af5d9d96e0714,     0x9f1c7fc331ca0394,     0x2eae24d40d37444b,     0xe7b70ebad5c120f7,
    0xa0cf8f8593e71358,     0xf5f4c49e50491590,     0x394f12e513b878a3,     0x0b9857e60a1feaa9,
    0x2289690ae8a2f315,     0x3f341b76a5277e91,     0x27a84a47cba0da82,     0x8c79b377ee868f98,
    0x11d45859a43843d1,     0x8d332f072480eb38,     0xfdc6aca1eb664267,     0x131ce2e79feac3ea,
    0x0757389ff88e6d6e,     0x7e750c4b7e4f025b,     0x2f452eb48c506ae5,     0xebe05451283a6b74,
    0x774935b637828d09,     0x6c58a014dbf54747,     0x12f75b837ca2f9e6,     0x2265711cbfffc289,
    0x2c6a010eeaa36981,     0xebd8a6b34e3d15e7,     0xc850d1e3fe016e39,     0x1cc20586178f9735,
    0x6dd034c5de26fcbe,     0xeb4ad7daee90668d,     0x124211322a3f3bcb,     0x96a21eed6bb4e762,
    0x13f537c7c61d1a09,     0xee7a99443439f627,     0x74985ef5d1116097,     0xadc31d0d7c84292d,
    0x83e75d7f1cc76440,     0x58857cbfa4546ed8,     0x20c42395c375e4ff,     0x6de6dbf3d3cf6706,
    0x295ee9d4850aa5b8,     0x32ca457f99f6c124,     0x32c4b65b5f8dfe8f,     0x007c861e753655c6,
    0xfa8f679202852b8b,     0x4f8645f22dd45353,     0x8bd8b445cb65ae5b,     0x98eddd9855e7c103,
    0x351461f50b7a846f,     0xfda29c9ce8d83920,     0xaf5070c068ad8328,     0x329df7b14266aed0,
    0xfdcffae3d14e796e,     0xa854619b3cfea4c7,     0xb99ae79a8a9d8dde,     0x079f1251f79e26d8,
    0x3c615bd371be61f9,     0x282eaf1c01e694e3,     0x288b2b3fd9e275bf,     0x66bda8809548781f,
    0x5f5f0ac1bfa8fecc,     0x185a2ae5d64b80a5,     0x611cc2a7227e0519,     0x9579bcc71e146f8c,
    0x4c728df10804e00f,     0x427ad462ec37dc58,     0x7f4e9d2f5eb34a33,     0x18d44d4ab4d94e60,
    0x854ab08b14c0b899,     0xb2c39b9af46217c2,     0xdf596e01a40789ab,     0xa126894630a84ed4,
    0xf96128991402459f,     0xfdabdf9ce6416916,     0x05402c97615f4e3c,     0xcc4e91f546359bad,
    0xfacc931df7049478,     0x67973d08de825624,     0xb6d9b47fad3c88cc,     0x706a24ddec9870bf,
    0xb444659490dca2b2,     0xdba563a62302e904,     0xe71ba2d83db56688,     0xe080ea79fa2ea5c9,
    0xd55d0144747201ff,     0x6c1f3c5839bb3d12,     0x18138f7b6fa4e475,     0x27ca987b170fde57,
    0x49494a000f5130a1,     0xd70caabc2a35c100,     0x0ed32f8e5614ef23,     0x6b82941d89f3c86e,
    0x43db14f9181c2eea,     0x115f53f772b42cf2,     0xbbde3b15c390a9ab,     0x29d0f8571c0d3dcd,
    0x5afb8b7e1ea30dc5,     0x1c3981dba9503afd,     0xf39a4fdbccbd2dfc,     0x0581959d492993c1,
    0x6ac55be8040f9397,     0x5dbe68bea00e9ae1,     0xc04761080ed4ce8b,     0x51526054a654c6ad,
    0x24ae7219dca5cf75,     0x90ae42da4b6af9cd,     0xf7bd2fd41a57030b,     0x963fabc4e5534f52,
    0xba28eacc0a4e94be,     0x50d3f3fc62c84553,     0xbdd9e3048b42ba42,     0x3e2935cf8b2d9146,
    0x286c57fab017f915,     0x3dc2e39de292dab0,     0xbe0297435cb4a96d,     0x736637413dc689a0,
    0xd8b3f03fdec451ba,     0x72c2fdcd99cbda02,     0xd055356da51171fa,     0xcd0b59332470d8ce,
    0xb8994fc580e7cd5c,     0x45f14ad2d864433c,     0xf0dd5cae003293dd,     0x2271d7957f7b4796,
    0x978f1be99e1cae28,     0x9375d8cdbc1a154d,     0x15bfecad888ddc0e,     0x90391dda37f0cbc9,
    0xb23cf85cf3dd5018,     0xeb8fb678de3b7980,     0xf3e47a505b07d4ee,     0x68ce7d61b9a63bea,
    0x00d11727aad2c2c3,     0xdcea4bfb158aff64,     0xb9eef48403d888dd,     0xaff2ddb1c6229300,
    0x1128ab9618d47343,     0xafa669ee44d3a468,     0xd13b98cb8044cc9d,     0x01629619d4c83104,
    0x4a1e37ce31ec4c58,     0x20590090e9e995a9,     0x75531d5bd9aad851,     0xea1abcb0a9e049b2,
    0x0f7fc64782ee2a09,     0x572d90f5357c9fea,     0xcdfa8b4237a0f726,     0x90989dd94ea295df,
    0x10583584b11ae13d,     0x324779d9429981bd,     0x3eceae84a3dfa31e,     0x32701b1554a51984,
    0xda350bd1fb728283,     0xb45beeb837950d0f,     0x36e4b3ab7d96445e,     0x4b783277700e297d,
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
