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

use cli::{run_conserved_hashes, DEFAULT_THREADS};
use sim_core::MergeStrategy;

// A-0 pin: conserved-field hash per tick, default SimConfig (seed 0xA11A_2A11, L=1 scalar).
// Captured on arm64 + Rust 1.96.0 (matches the CI `v2-golden-arm64` job arch + toolchain).
const GOLDEN_CONSERVED: [u64; 384] = [
    0xe7708e523f24d7bb, 0xdc5072a7efa18ea6, 0x859969385395c479, 0xa3fbf026ebeaf542,
    0x409b13d915354b69, 0x8c45a252dbec72b7, 0x9a7450c0fa341a67, 0x93f370c0999fcb11,
    0x3320138d1bea30b0, 0xa7e5df3d87c81514, 0xb67125470c2b6f63, 0xfd6412c1cf8dee1d,
    0x3a0912c0bfe294f2, 0x8c66a559c0010559, 0x9428a01910194088, 0x1845650f30c1e1d6,
    0xf8473954d0c2ca14, 0x7c32473b8c0aa442, 0xe799007f751c42d1, 0x8024a30c188ceb73,
    0x60e5a2bdc047b2c7, 0x9751c47b3ae13f5d, 0xa0b931a6f6da7235, 0xf2e47f537cb38dbd,
    0x4684a6edd578fc2e, 0x13133ef347f38dbf, 0xf358f2e20b70178a, 0xa3f2896a74c36f9c,
    0x0c44389f11584961, 0xe7b7df8188ad1bf6, 0xfb5a7be104867fe5, 0x91f840309704d8fd,
    0x9a1191cdb3b3ae0e, 0xdf6dc69cfc473def, 0x676d9754d4bb0cf8, 0x8313a0baa6332406,
    0x35cf0ac6fd4f2365, 0xe891dab38ee08b29, 0xaf5848700c96561a, 0xe467e730df8fb759,
    0x5d2d60c58e511aef, 0xd5a1e243384cd993, 0x21b317bdbda14993, 0xc864c390929540a5,
    0x7640462535ed2f67, 0xd5bcde778e968626, 0x2266f671a2642d1c, 0xcd667862b46edf3a,
    0xfa85d44984e03c08, 0x1117b5eb85f8711c, 0xbc30512254e4fee1, 0xd88b03f8948f898b,
    0xd9f040a766264d6c, 0xac9caacb4a3e5e97, 0x741fad1825b5ff0b, 0xf71ea7bcb9ea7f5c,
    0xd91a655436d170bf, 0x5f0978cc08a32ca3, 0xb821fb3119a6fd21, 0xcf07539b00ef06fc,
    0xbd92fb55b792608c, 0x3492e4dde5bdbf8d, 0x9593e1b0493cabae, 0xdb038253b0537bdf,
    0xac941ee9e1d03072, 0xcea945984d8af4d2, 0xf2fdf72cb4944800, 0x17d2e14ba7b5ae9e,
    0x979ee150ca6582c7, 0x2ec81b0b979f1e48, 0x0789a5283dc70eb1, 0xd54c8130b8157d23,
    0x66a5267171101b34, 0x1a05937251d0c46e, 0x66dbe11c2134c189, 0x1b5731ec5b6c5823,
    0x63606d64ad0cea22, 0x426de6f833b553d9, 0xa11c72cff58398df, 0x48d0f6dde1f709f5,
    0x178a7aa2d3352c38, 0x26f70358deecfaaa, 0xe6f90fc20ed57a9c, 0x0006e06c94f3f282,
    0x957933c1c4bfdcd5, 0x8a6d20a39981ebd7, 0x897d65e76c82bdda, 0xadc4dcf443fcad4e,
    0x0df5a54c97488ca4, 0xaf923ea56013dbf6, 0x0d65bd29ffdccd4e, 0x01c59e779064caa2,
    0x7f93aeccea6c41a4, 0x7d8b3eb0776d89b1, 0x2cd1b004fa978148, 0x2a4758825d8d56e1,
    0xfe51c40df82fa1dc, 0x0d09066b59f2ae3f, 0x221c316633d4502f, 0x69583d7bc347c5c5,
    0x39bbdfa0ecd75baf, 0x5afef9896c4e596c, 0x0117c04f48cf67e3, 0xae8a20e373a6f16f,
    0x37ebe1e0dba56d0d, 0x19c2aeebf69a2f64, 0x8c7c39d39d5ae870, 0x7e2dd7cc7643cf95,
    0xf842e497abc01564, 0xb23605b852c70781, 0xbac72c5274b5a0ec, 0x8c9e0541396762cc,
    0x1fbf83b0bdd8e51d, 0xfdf64e3168546b43, 0x1ef3a2c56feba8c4, 0xfd7845e3cb000a5e,
    0x026ded5a2671a4f4, 0x92f27c8af94060e3, 0x79a2867020b188f3, 0xbd08300180866206,
    0x3941d56a45df5b3f, 0x3b5215bfb32b743e, 0x58e6502ef359ad7d, 0x60c1c48a6e7887c2,
    0xfef5dcd7332017a8, 0x3e8059ce7e7276ba, 0x370efd545c97b258, 0x6a3258544258b930,
    0x152ed60a906b6b77, 0xfc0411dd4c4f23e8, 0x516e9382eb0e4be7, 0xd3354fadc7e44c59,
    0x48bdd48392abc20b, 0x1921fcbad9ee91f5, 0xabf75f90dddd3051, 0xeea6c2d1590e6609,
    0x676970a9a89143e7, 0x3524a7208a90b662, 0xc8237c145877c9a0, 0xead06e1a2878de71,
    0x787eba766774c409, 0x25d90098fc4e3bc4, 0x921a798c6f595675, 0xefd5a5d1366877f8,
    0xa0a57e37b045c4a5, 0xe80204c4a231950d, 0x9a8778a4187a9f49, 0x5720ed3197baef4d,
    0xbb3819ca56cffc54, 0x787a657f78f37d68, 0x4038e4c7f5ae5d6c, 0x1d3d7b95b0eadb7e,
    0x8c37074fb4c2c256, 0xb97a4d99440c1790, 0x833712109bc23185, 0xb27ece612729653f,
    0x6a06f8965f433633, 0x7abe30bae3ade1bd, 0x38cc99d1ad255ac0, 0x795439c0fc1295c9,
    0x04f8c308e1eaf711, 0xd8c082ac88a197ae, 0x243c1dd779cc3bf0, 0x742cab977f91050b,
    0xc784c95d78bf3ce8, 0x56d0ca1a6a6febb0, 0x440ca05a0a6e5862, 0x36f460a7b9817321,
    0x87a62eba5bb6cf56, 0x6a10b6fc36bd1fc0, 0xee50ff8451b395bd, 0xd2ce321c37964bcc,
    0x1dfd967f52cfae7b, 0x657e0da563b75a62, 0x62b2e3f269b30a42, 0xe769c1394d4846af,
    0x403b3d1826fbdc6c, 0xbc79bea7ecdfc541, 0xbfa4e7c4ddca2532, 0x0cc198b9b9888e89,
    0x149329b75cb2f106, 0x5d97b87fda6b1838, 0xa12dfac096b4bbb8, 0x9bf6115405eb7da3,
    0x7d922e9cd8698c8d, 0xeeb46e79389af1a4, 0xcf431f93c4537765, 0xe2cf7d281140bdcc,
    0xe652163317ce1989, 0x13fb61aa0a6f3a69, 0xf3f3644fe10f3f7e, 0xfa1954957e8f0626,
    0x3ccd2a1b33a8bf25, 0x364e4f617128bcc7, 0x432bba8797059393, 0xbf0cd8373ac18d07,
    0xe8ca27b867b1dc38, 0xd70473c2c1ebe58d, 0xd72238bf050cc59d, 0x9e9d1dc316c17d7d,
    0x5191a8341751f097, 0xe1fb32f0d2846d83, 0x6e8a2c4b19454443, 0x7b6ab3b9cb0659e2,
    0xd54939fd87a92ad1, 0x4493044761dc461d, 0x95338c7ab7dec564, 0xd632397e58689e34,
    0x1e51d8d43b782136, 0x62fca7337b6016a3, 0xaa87ff8fa11d3fb6, 0xc127934826573b60,
    0xa058d9d01d68bf99, 0x7dbf4499db7b96d1, 0xf9c938247b26ec0b, 0x5f9178be3fa4a051,
    0x4513f8e9567008b7, 0xf1653505f873dac4, 0xbca292f43316b582, 0x53d5a996bf48a425,
    0xe25dc7f07eea298f, 0x5a6d14ac0171fc99, 0x38cdf86352a2123c, 0x73e01a5ca0a84e76,
    0x2f3e1db27a3099e7, 0x22a94f05a04c9173, 0xf44824df8f9a90b8, 0x9021a56e94aef6ea,
    0xcd6a02cc0832ffdb, 0x7b6f574c30f7096d, 0x9934b711845785e9, 0xfea1e34bc14fac48,
    0x8e6275b09de107fa, 0x42c5b07eb9289075, 0xbaf3f957da2a4cc1, 0xac483def244fb0f8,
    0x3c71088e0257b906, 0x6ce0a6ba94023a18, 0xf92dc06406b54361, 0xaa4b85edac87ceb3,
    0x5130eae81ae7af43, 0x3200fc56d9c823e6, 0x6dd091ff072623f3, 0x24e2b704e4c68399,
    0xf70f66bcdd79fc69, 0xab03d10e4ffe8867, 0x21b66520160c8f08, 0x86e8eb138a399cc5,
    0x977e203053296c13, 0xcc75ed466058ad89, 0xc91e16eebec73dc9, 0x1bb894d583c1dc72,
    0xc0c0a651018261f2, 0x89788a59ba7e6040, 0x7c10ad0a26a89881, 0x382e9937d5bad055,
    0x2e5cd30be1ae1757, 0x5809dcdee2cddf66, 0x8fa6ad48ab3a1327, 0x41191f61e4342f7d,
    0xcdcdef38f42160af, 0xe97e03c77fb253e9, 0xd21b50af5ecc3aa5, 0xdc41233bd83eb6d2,
    0xbe17747a44eac5ac, 0x2e585d47a56bfc0f, 0x2a903c07996c2c62, 0xf01d918e2b30987a,
    0x82d07205d6580cfe, 0x84619999441cd07c, 0x8de74fc318f47039, 0x9d4f97033de65b76,
    0x572d7b5138656975, 0x1af4c84073abde51, 0x5f8ac6ff35e1feb9, 0xca4b897fead2dc83,
    0x598bd74fde0690c5, 0x715f2a4bb1132004, 0x6821d8d15d86ddd7, 0x27bdd99d15904a16,
    0xbb95072cb5214186, 0xdae5077a31e327c1, 0xe8f71a746eefe05d, 0x1e715e03cf0b3813,
    0xd42c4704f49926e7, 0xabed22dd5e09ce2a, 0x5e5d98585cad67cf, 0x3161f260f72485c4,
    0x93ac03930a4dfd53, 0x0d7e0fa4521842e4, 0xf4cf86d2fe129c55, 0x9851c956e35d260f,
    0xa2c2209c56e398bc, 0x97939e0a2c570144, 0xfcc7095512db63a2, 0x26f39409efbcfc08,
    0x49bd3edd21724138, 0xc4c1fc026d880004, 0x60f0bf2ce29acf98, 0xd34ee46a07393bda,
    0x827475e37da63622, 0x57782bdb7eaf14a1, 0x406c03001dc0503b, 0xff0e6af7f7715cd4,
    0xa9c2318771f5fa3d, 0x1f129ae21a9b7939, 0x9f1a8115c9ff2794, 0xb99768eeef2d42d4,
    0xaf6d8c599dd1af76, 0xfd0405f43fa79030, 0x5488b59527f373fc, 0x2c210fdcd81a0481,
    0xbe09b548b78dd643, 0xee3a74418e4f0c51, 0xf4645b612c341537, 0x3ba460c7f3d1e388,
    0x7f7fa714bb4c992b, 0xd7d350721c27b847, 0x688f4e8df1d4d576, 0x76bb8fb62b2f7363,
    0x6f820295ddf69d91, 0x1b0abad575965915, 0xab19683bc02ac130, 0x3e1af4862af371ee,
    0x2fedffed19c48c21, 0x9e60b932830c1729, 0x52e7d1269fca3ee2, 0x27bbf2d6be9b1546,
    0x03329907e14534de, 0x77869179e466c246, 0x8b66ffabaa485d44, 0xd021e9cd493713b5,
    0x692b4fd2ab9b280e, 0xf57c18b843b6ecc4, 0x543b1a3ad38d06ac, 0xb3f4d741909115fc,
    0x26e5f1973cd8b37e, 0x84913399e99ea123, 0xc732a0a8b7e0d6b7, 0x2b859d008c0798b0,
    0x3b5b1269ee956021, 0x587d9d13766211ee, 0x688393d1f6d2d4f5, 0x68f83d473ea16f2a,
    0xfe6a508f7c505fa3, 0x68741da620c9539d, 0x3e491b1dac9be6cf, 0x83282a580249b50b,
    0xa7dad24ebc35e21f, 0xdef0f815058b8324, 0xc6b886dca466087b, 0xf24e938db423d511,
    0x278d57dbfc854275, 0x7279ed5753eb3055, 0x70136537aeb0d19a, 0x4744409923bc7dd4,
    0x58d98fe1edc9880f, 0x18950e70c3ba9dfc, 0x4193e2db2144ae0c, 0x4c858f68e78dc84b,
    0xca7d16c3724e9ea6, 0x8869f02f04565b1c, 0x1ea59aa5c5eac5f1, 0xd8a6dd8df4cea11a,
    0xd427d720ff665bca, 0xd8b486a713181ff4, 0x58695f522b6b90fa, 0xb69826d3cad4e7b1,
    0x015bc4a3ec184dc5, 0xe69470b743a20974, 0x8395f1bd7b7a8c2c, 0x3817c9f97fffd229,
    0x004bfcfc3b93a33f, 0xb3fb1ace74a195ba, 0xca8d3e0f83b1b072, 0xb2d352f66078eefe,
    0x51f3b1c2997d0ff4, 0x6a79e49b03a03dfe, 0xf98fd351a3419937, 0xf475afeacd3db826,
    0x5de35d25a732ee29, 0x93160cd795f4de8a, 0x99057c4485973307, 0x93c1229f37b0254e,
];

/// Conserved-field golden pin (A-0 / R19). Arm64 + release only (FMA-divergent trajectory).
/// Excluded from x86 jobs automatically via the `v2_golden` name prefix.
#[test]
fn v2_golden_conserved() {
    if cfg!(debug_assertions) {
        return; // pinned for release; float-fusing differs in debug (arm64 release CI job)
    }
    let h = run_conserved_hashes(
        0xA11A_2A11,
        DEFAULT_THREADS,
        MergeStrategy::Canonical,
        GOLDEN_CONSERVED.len() as u64,
    );
    for t in 0..GOLDEN_CONSERVED.len() {
        assert_eq!(
            h[t], GOLDEN_CONSERVED[t],
            "conserved golden drift at tick {t} (left=run, right=GOLDEN_CONSERVED)"
        );
    }
}
