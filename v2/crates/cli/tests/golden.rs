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
    0xb4eed043bb753494, 0x3edc83294df4b348, 0x8eff50d8cd4f4922, 0xdb8952212ce18bd6,
    0x2e47ff2268cdb814, 0x9708ed7d8cac986e, 0x1db2db7e23d73402, 0xe07ef8a1b2cb77f3,
    0xdf00b55aefda7f68, 0x43691d9f9e1c580a, 0xbfaf3f44c9d42a4b, 0x81918f569d4e8d68,
    0xf7c23bc0cee76b1c, 0xd1bd8a1ac466e6d6, 0xc609adc6ef7f733d, 0xde4e6ecfeff93cbd,
    0x9cb62987b66db898, 0xcbbb34c1c6f46446, 0xc8374b810efa0fbb, 0x3b7d080c7534e665,
    0x93b44f4a34abfe35, 0x2dc63feb6baa442d, 0xcd081b5df58ae529, 0xca3e6f04267f55e3,
    0xbd9c73e36d57a177, 0x95da89225b1f01d7, 0x283e3d6df406df26, 0xd281a963f4f76848,
    0xcb184053fad33f9f, 0x98f9ca342064f145, 0x90d3cc7fb158e1bd, 0x37549a8282a90962,
    0x620b659256d8023f, 0x695080cf7b1362ad, 0x85c779d6b7054a87, 0x7fbd4d40678f224d,
    0x7a9346102390b35e, 0xde1c307ee8198cda, 0x8fe1b311a535a497, 0x1e46d2a501c20864,
    0x31a913a15881c1c7, 0xa575c0778a0bfe7e, 0x5f877b5819241984, 0x84571cdb61085ff8,
    0x446ab6abc86add22, 0x626e08e9f65efa4d, 0x65197c10b800ede6, 0x31c8763ef3e8401f,
    0x622cf930d7d12439, 0xa6c3637345bb10be, 0xc516878e3af2f165, 0xef6ad2e3e6769e55,
    0xcdfe8c8c348b8e0a, 0x3a72c9cfa0904640, 0x61aacc7dbaa5e02e, 0x181e013895d44bf2,
    0x1ca3fbd63b581fdf, 0x8d3444589dc6f4e9, 0x75c9ec857c58c472, 0x03d090c290067068,
    0xe12f50ef0630f25e, 0xe1a9dbfd841cd1b2, 0x747679b10e22aa8b, 0x4c531c32b27fba64,
    0xd9688da62aabb55c, 0xb38787fa136f64fa, 0x3380c71020014ddf, 0xf1cb6ccb0d065c46,
    0x3ca5d2c35800a1b6, 0x25a41f1c211e895b, 0xe66952c9653c86d9, 0xbb83fd59087d0316,
    0xb7ee62d3efecb8dc, 0xe71c216e9470c33f, 0x577b660f7d1d450f, 0xfbc4ef87eebf3ba8,
    0xf004fc35ed3bbadd, 0xba9162b885af438e, 0x36e759c05d9a7c52, 0x73cdb68278cd2a4e,
    0xe764e7bb8512c8ed, 0x6a42585aa8420c67, 0x5d2fb0e79ae1b04d, 0xc2c38381ebb8454f,
    0xecbb859ba1aad789, 0x84f256eff2d9dd88, 0xbb196c2ac963dbf7, 0x73d2a2ead193d034,
    0xafb0a8ef903bc30e, 0x45dbf380bd7565c1, 0xa0663351d56e2a57, 0x82e844de324a627d,
    0x607ff1ac1c0fe546, 0x586585a4c6bfaf02, 0xec750961ff096c6c, 0xafe4995d86ea8bb8,
    0x8cd41bc35795e81f, 0x9968d2b0a2d1cb58, 0x93b8d7230636aa99, 0x8b8cdefd2c3055e3,
    0xa6a174c5d051381f, 0x8d1ef36146d49ad9, 0xa867fcca65657b29, 0x6fafbe0181c716e5,
    0x2f569bd97ae55e78, 0x7c26eeafdb1ac120, 0x7187a25fdaf70530, 0x034d1c35d022a530,
    0x34cf566c7a4b4910, 0x960b399b8146aec9, 0xcaf70b1701ac2985, 0xff3a042586bb490c,
    0x62e4b33ceab03a38, 0x3cf51a7d6466b265, 0x00b2941b6b491c3e, 0x5318194a083b6b74,
    0x67c80ac50070ebb8, 0xae5b84d8f3925391, 0x27ac035e7ffbf63e, 0x4bb33d0d50e8e848,
    0xdadb5720c07e16b2, 0x56671afb8b7d5937, 0x71aa41b25cae48a8, 0xf4144b013918fb86,
    0x08b0c25e36fa6f2c, 0xbfba0b5a94b6a71e, 0x372b733fc244833e, 0xabd86ed2f8b3d6c5,
    0x631cffb812db5dd1, 0xf753e786ee8f9f4d, 0x246782773bc9c3ff, 0x2087321c54e0e4ec,
    0x91064d1d212028fa, 0x1f0a06a5aba1c3a0, 0x56ba30db9d6c7b64, 0xe00209d4de13ade9,
    0xb593fb428b331856, 0x8ce4a74a69030747, 0x5ef344acd7f0eb3b, 0xe3fb99f4d8cda529,
    0x51ef28ad5ab39774, 0x594b63ecaeaad9d6, 0x7614ad7c83907cec, 0x0a42eb0cee0a592e,
    0x9f15bc88ce31ff6a, 0xd77b575fd630dc6c, 0x2303fa96e1accf74, 0xa7904901e12cadfc,
    0x329f5d01f875b722, 0xbe0750a82252ae62, 0x0539d9bb4442672f, 0xac34da7b07c67097,
    0x56fd18718774e4de, 0x472a7cd2a131d221, 0x4b12f8261ede54eb, 0x59a5f56a258488a4,
    0xb409233424e91039, 0x6b9b3669e1cc60cf, 0x7c18e8eede00fc2a, 0x747904d44b31b281,
    0xc1813521d03097b3, 0x77b96dc266a20848, 0x3bf28584c2e36393, 0x4d159260838b5015,
    0x80709d055e0177a7, 0x15d0754fa4f7ddd5, 0x000b2202f21e9fe4, 0x4fc558ed10d619cb,
    0xbbbb2d179241769b, 0xaefee6d67f9ca1d9, 0xec8afe8d40187b29, 0x9f48028c8ee98b5c,
    0x6dad4a72307c93e1, 0xcf17f3323be5f903, 0xe921e2245f2ecaa8, 0x420cf9b6b2a44c6c,
    0x477988476a37db10, 0x6112dcbba27f095e, 0xf1d0d34f99226444, 0xc86877f7105c357a,
    0x78acb3fad65b1d5c, 0x77511887e82623ce, 0x8e71ccd865db9699, 0x89c9d3a2a625855f,
    0x169eb7cb3f0a74af, 0x385724757de00908, 0x4c76b5450bdd011f, 0xbef81ea6fae901ce,
    0x78991758a8487747, 0x0588ca9165cb67c0, 0x6e6ecd6214ff2c90, 0x13123ed307eb9199,
    0x032523a9682a4d43, 0x22f7b031a18a7f1d, 0x7baef4e8e41a3b16, 0xc81ff92f846ca016,
    0xc28a05599d2ff82c, 0xaa7bc4e2ed788ad5, 0x922d9a0c5a7cba08, 0x8526a886ecfaa0fe,
    0xf8eb6489b7c924ff, 0x7e6d4c7d8a1cb333, 0xb21a944edf8fb18e, 0x40bf0f0d2dcbadc9,
    0x6d50f580bd889575, 0x159713e6857be665, 0x6f17924fa3ce1a81, 0x69dabfb45fc1afde,
    0x600f72d8bdd350b5, 0x6f536c1e0ac54463, 0x8744948d97670eec, 0x49a76d1f12a49676,
    0xfcedf007e738cbc9, 0x78bf46725257d901, 0x530b310f99926418, 0xb97cb447be0b4284,
    0xa25759dee22e3107, 0x9c74747ecca4d0e6, 0xa02c1290e519da76, 0x415adf1f2aa9d149,
    0xfc6901d215cc9079, 0xda6a22c253ee1333, 0x75b0c8a0e9bbeeb0, 0xa604eda1ca0f22c8,
    0x707ff4ccb1acd7e7, 0x7511e6f89a2892a9, 0xe7acd5b9d29b7125, 0x4acf8dbbb23a1c6c,
    0xbaf52b879076bd6c, 0x6f7e22ec07868216, 0xb2f69bdeebeac0ed, 0xcfb705c79ff23201,
    0x484372f43f036a8d, 0xe6503945aa8ccc7a, 0x341026b843b3a168, 0xdb72f9e7da2361d9,
    0x3213e3e4b3d80417, 0xfca552b5fec11b5c, 0x51e5747f3999a89d, 0x080be10207e4d880,
    0x02fb64764f1e9f7a, 0x0b3cdce342df8209, 0xc6544d3a79314052, 0x3f1c05fc2ad33077,
    0x76d84bc4e22d3b84, 0x50cdbec64369605e, 0x98d9f1fe264a873d, 0x5bc878b0c16327bd,
    0x738537dab368b9c1, 0xae43529283ad9c15, 0x2b8badfb2a340e6f, 0x41fd6ed1796b42c9,
    0x58dc155293d5e4b3, 0x762c0eed72e403f5, 0xf1d961d70fa22a65, 0x2785b7c2a6c1aed4,
    0x701817b00b07cc66, 0x2562eeba73420cb4, 0x5cf8ca8a845d0cb2, 0xf02240f7e337c5c0,
    0x8f30d583d3ea3571, 0x38c3d2c0f824493e, 0xfc12ba64646c7f66, 0x0d0d9906925139a5,
    0x49f3100dfc61b60b, 0xaba2c0c3861dd73e, 0xadb962493eeae36b, 0x9cb57f54276f6f7f,
    0xbdbdea1909e8d282, 0x9daf3f75d95469f4, 0x2e1f05b43c0a42a1, 0xf0b1016f5622a812,
    0xfdc393703534eca0, 0x74ae7686b0cedede, 0x92ee563715e5f181, 0x575872646a83c0db,
    0xcbef1b1fc9685919, 0x36026ba3398b22e9, 0x811e749120668361, 0x64c81c982253b3e0,
    0x4b78f17d1c91f959, 0x9945b07467e83464, 0x5e3d7a76bbfe3582, 0x986916e50cc33c82,
    0x6b7976ea270d92d8, 0xca1900bd9418f17f, 0x0cebb96bbf044495, 0xf0cb35cc70848898,
    0x6ab8f8a73de15e8d, 0xa8af2647d212d102, 0x5bd07e60e3e1c0dd, 0xc51bea587532ef22,
    0x7a468f307882b50c, 0xfd5bf7f17440f338, 0x3be2fb000a642dec, 0xc5b385cc84044eeb,
    0x296629b12a92cbfc, 0xb8d07f35b2994933, 0x8402936dbb30e761, 0x731c3e694a979b53,
    0xba5e4cd542d7adca, 0xe7fb622f6db3f869, 0xec2cb76b8bd5636c, 0x113d92c98adde539,
    0x3fbe8213e23f3095, 0x54d96a02b3ac413e, 0xbeb1f9ef28a17949, 0x17a0879b9738a4b1,
    0x0bf5485c715709c2, 0x2226159693d71403, 0x21ef4ff999edca76, 0xe73f241754480afb,
    0xa21904b9481289d8, 0xb2d142aca12d3f8a, 0x98492e0c0caacac1, 0x85e35b56bc8de8c9,
    0x229a9d71180b303e, 0x77501e6cd592e55d, 0x24987db0cc4ec358, 0xa090d19fa9c7defa,
    0x0fa0621c3a772093, 0xabec7fa783c8d934, 0x261f790756615fa6, 0xfaa9c0bc3a56e81e,
    0xf57be92e0abc583d, 0x29ce84d02fff2608, 0xa15b383ce4a4cb6f, 0x8e9903742674d030,
    0x96dc53234eb80d76, 0xabf6810ffed11fe2, 0xd83dcc3c8c9317ac, 0xb4b0e96a5451b6b9,
    0x11e7230b36602806, 0xc9052104313a3530, 0x52e1731280f8e739, 0xedae7f90191b2eef,
    0x71f138d484c9a097, 0xb3b1d2a87e910da5, 0xe153c389d1e90991, 0x012de221737b0f6a,
    0x9f83d331dde16396, 0xb6a382e8a4542ece, 0xcfea90a32573a6c4, 0xdd9557008e545f95,
    0xa9afe1d42075ee9b, 0x432163233fd20dfe, 0x45cb8011b98aacbf, 0x44037c7da10130a9,
    0xa2629f75d12f63ec, 0x39263aac64e45369, 0x743d82c12630c133, 0x8180945a92a6d22c,
    0x031538157ed94d29, 0x734ee9b5c6fe6fc4, 0xbc31efca44391aab, 0x70487547f8e0176c,
    0xe28844721dca9a0f, 0xe97d8b65efb6da51, 0x9689dfc110940da3, 0xab09079f9f2b92cb,
    0x836e6098a4490e3b, 0x81c3e7a21755b404, 0x226e225ad2cb11de, 0x61e11bd32b4aa383,
    0xefea9b724f388fd4, 0x205b848b66362481, 0x90ec4216b3e3513b, 0xbbf656cb68d39975,
    0xe2902a442bce979e, 0xef0cafcf95b7c610, 0x2d77aeb27043a7e0, 0x429398eba3c51f86,
    0xad7aa997b91ebb72, 0x7830cf6447e93762, 0xf469b4cd3aee0946, 0x8aa38a2648c0823d,
    0x9d920b567671dfd4, 0x6e8f233946aed344, 0xa1108c0919d46596, 0x2192c0cdc1be17c9,
    0x5a4d786bbff527c0, 0xf2b1fdcc8c11b9ca, 0xc4f089681227161f, 0xebab10c5c446eb1a,
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
