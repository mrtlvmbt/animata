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
    0x2d6d646f941ad7bb, 0xecc79cf26fed8ea6, 0x92610ee23107c479, 0x108621f3276ef542,
    0xdc509ffff8874b69, 0x82e3d67984da72b7, 0xadc658e74a821a67, 0x573e8ca21041cb11,
    0x74cc0dae554a30b0, 0x18af9f191bf01514, 0x7b76aea324716f63, 0xf76b356b3247ee1d,
    0x2a4da52892c694f2, 0x880dfd778f330559, 0xf943b68e95294088, 0xff896b146f6de1d6,
    0x9a06cf384eeaca14, 0x05cb9ec6a58ea442, 0xbc7ea51fbb3e42d1, 0x72becef6b0f2eb73,
    0x79f17ba87155b2c7, 0x829b5b81e01b3f5d, 0x143db92fadc47235, 0x953feb86eead8dbd,
    0xff3ff638b4d4fc2e, 0x0d0487b46af18dbf, 0x12c5a0e66f84178a, 0xa006ec7cc1fb6f9c,
    0x6aa2b0a42c9a4961, 0xcbb6c0c54b991bf6, 0x9a8b82752ed07fe5, 0x0d5d24aa3f7ed8fd,
    0x43e90c3be6cfae0e, 0x1c78efa617a53def, 0x14ce2712aaab0cf8, 0x304df73c013f2406,
    0x78e806322e992365, 0x47506c1ad1b28b29, 0x3517f809b5ca561a, 0x74fbd7f512c1b759,
    0x71ed3348e3af1aef, 0x72d8134fbcf2d993, 0xb7eadc2b22474993, 0x562f24c49e5f40a5,
    0x4e024a00303b2f67, 0xd53dbefbbde28626, 0xab5516082a9c2d1c, 0xa5a6ae64ffe2df3a,
    0xc27880fec0f03c08, 0x3976182c9630711c, 0xabda90bf9b26fee1, 0xcc87db2bb525898b,
    0xd02e45f5d6fe4d6c, 0x27bd2ad0baec5e97, 0xb51bedf6f14bff0b, 0x6e18bc6f06a27f5c,
    0xae9c844a9fcf70bf, 0x43066e243b692ca3, 0xb78a89577c68fd21, 0x3d8158c92ce706fc,
    0x59e55c68deaa608c, 0x738bdd262357bf8d, 0x32b7b36e4798abae, 0x058eb176bf917bdf,
    0xb2d88c2eabb43072, 0xa1d1b7a5d02ef4d2, 0x86e0773944944800, 0xabcca897a3f1ae9e,
    0xbf01e4df1b7382c7, 0x792d6915b82f1e48, 0x4c6fb3c90ba90eb1, 0x49ba6581cbdb7d23,
    0x0b6ca6cb21781b34, 0xcd2d89c6e1acc46e, 0x4926356800c6c189, 0xe0e97320a5325823,
    0x034bb64c4250ea22, 0x285979e1dfe753d9, 0xf2ab0ad9bec198df, 0xaa37d175a86109f5,
    0x257bca4287a52c38, 0x79dd292b9940faaa, 0x6e862b45f20d7a9c, 0x1a222df76af7f282,
    0xfae63b81a0e9dcd5, 0x35e7efa4c4afebd7, 0x9db8f197c536bdda, 0x8ca6ef521598ad4e,
    0xb25026a662908ca4, 0x3e2bb416a2ffdbf6, 0x4fb712581178cd4e, 0x8b73426b26a8caa2,
    0x90c3c7009fb441a4, 0xe10aeda3bb4f89b1, 0xee4825b961278148, 0x407e17dc53cf56e1,
    0x31661bf7c9e7a1dc, 0x4126f926fdf0ae3f, 0x9aadc4f893b2502f, 0x8b20786a6951c5c5,
    0x5a86bd7323b55baf, 0xb60a314df526596c, 0x9d5fc19b921567e3, 0xc3075cf9b604f16f,
    0xb40fc533343f6d0d, 0x69e6898c67622f64, 0x192498bc263ae870, 0x945c49df97edcf95,
    0x28e61a86e8881564, 0x5bf9fd24fa490781, 0xd919f9dd4c8da0ec, 0xa30cdbe584ff62cc,
    0x6f01b2d38e92e51d, 0xbf40c022685a6b43, 0xba86cf6e8373a8c4, 0x41b81cdc5ebc0a5e,
    0x6e1e4ff5ca59a4f4, 0xd6ca3f4db48660e3, 0x2d53ead6b41788f3, 0xdd81151857926206,
    0xea86564dc3dd5b3f, 0xa05a8f590aa7743e, 0xccc0260884d3ad7d, 0x724ca4d70efc87c2,
    0x2a34ed08f67017a8, 0xe3a43a77b8e676ba, 0x85feb6e72d47b258, 0x02c5ccc2ccb8b930,
    0x77c1e90d8ad96b77, 0xebeea7c9481f23e8, 0x171ead3a5e5c4be7, 0x6289171da5164c59,
    0x593258b76441c20b, 0x3edccdc5b05891f5, 0x1b1d0f8dbeff3051, 0x1606716ec1a06609,
    0x4a2618230bdf43e7, 0x2fed2acbd854b662, 0x92eb14c7bbb7c9a0, 0xa0dbd0d075dade71,
    0x1ab64c5f8c06c409, 0x60451558b5d63bc4, 0x0f0b5b9b0ec35675, 0xc18ba3e7625877f8,
    0x4948fa6ec40fc4a5, 0x4b97ed034acb950d, 0xdac8c404938c9f49, 0x32f246d614d4ef4d,
    0x3d9ad2475977fc54, 0xb8c8eb64e7c37d68, 0x127afaae86865d6c, 0x65407aff76e6db7e,
    0xe4ed9632f46ec256, 0x7db54cfebb2c1790, 0xd0ca4c93f94c3185, 0xa2326b4db927653f,
    0x3fc4dca4ed293633, 0xb99916ecfda7e1bd, 0xff6a6bdbc2a55ac0, 0xe7449748a42495c9,
    0x6aff0a36b08cf711, 0xcc5b34ca5efd97ae, 0xfd1481c069ac3bf0, 0xd9b97ebf5727050b,
    0x7fc01b4e268f3ce8, 0xdc42dbec99cfebb0, 0x3ea2df419c325862, 0xacdb334e08437321,
    0x5052be843562cf56, 0xafce1450563d1fc0, 0xa5c8cc7bd3ad95bd, 0x028e272ad52e4bcc,
    0x9dd7d70fb545ae7b, 0xa909494ff97b5a62, 0x4d3345114f370a42, 0xdacb60aada2646af,
    0xa3cdb98135d3dc6c, 0x3bc8893eefe1c541, 0x5aae2cf9f12e2532, 0x0807ca8bb31a8e89,
    0x6793d016d1bef106, 0x86a3b8a166db1838, 0x7f340f3b2a24bbb8, 0x9dab8fbd5ab17da3,
    0xd4c1251b30038c8d, 0xdf83bee24de2f1a4, 0x2f9aca119d9d7765, 0xed3076f492d8bdcc,
    0x3577169da7601989, 0xc18ef0e44bc13a69, 0xb467e2946f0b3f7e, 0xe89abd4aaddb0626,
    0x57de40f17c72bf25, 0x6d3c17033636bcc7, 0xe9a83e528fab9393, 0x97fd90a8c04f8d07,
    0x28f633297c21dc38, 0x55630f8a4b85e58d, 0xdd3918a5d6c6c59d, 0x426f65d4483b7d7d,
    0x91be3b59abfff097, 0xfc2ce0a8f70a6d83, 0xb32bc5a24b4b4443, 0xf8ad8a161fca59e2,
    0x02609deb9dcb2ad1, 0x043e9cf97496461d, 0x0524ee7f54a6c564, 0xd445f5528ed09e34,
    0x33e0b11ea8e42136, 0x2a9cc1ea822616a3, 0x7e45fe6f8b893fb6, 0x82d029ed4d173b60,
    0x88fc4340811abf99, 0x30e54c82c99d96d1, 0x219cb253a0bcec0b, 0xc63fb68200c6a051,
    0x1a32ee722b5e08b7, 0xc257cd086ffbdac4, 0xd21314ae0f1ab582, 0x0370eed75212a425,
    0x6ae124444188298f, 0x1b2084455f23fc99, 0x72088f7b351a123c, 0x971fcacc08944e76,
    0x19f1aca5897e99e7, 0x3bd4025284b29173, 0x9e7c87c84d0a90b8, 0xf4fe6e196782f6ea,
    0x475a34b4bd68ffdb, 0x9f0e3440f251096d, 0xaf9c72f61ca985e9, 0xf2f6a8cdfddfac48,
    0xb3cebe8d9ad507fa, 0xe9b042b8cc929075, 0x1cfdd31bac2c4cc1, 0xb9d27988423fb0f8,
    0xaae9855b0763b906, 0xf0bcd57054323a18, 0x07d3209015f74361, 0x311d9a7dab6dceb3,
    0xe06de501a2edaf43, 0x9d66dba7249423e6, 0xca820264508c23f3, 0xa1491372d0788399,
    0xbb69c11da2cbfc69, 0x4866cad07c4c8867, 0x420f225f781c8f08, 0x510044395e439cc5,
    0xb8f0ffdb3ccf6c13, 0x2dd628f017eaad89, 0xaf47b1f3b6d93dc9, 0x46b72ff3a5a5dc72,
    0xd5db2ed6ee6661f2, 0x66cee13c9afe6040, 0x0a74f55a702a9881, 0x7c3832db58e4d055,
    0xe255122f23dc1757, 0x218e0c4c6499df66, 0xf5c7b8d94d081327, 0x574eade179ae2f7d,
    0xe84e12d9b4ff60af, 0xb34ed598b40453e9, 0x4c2fa4be5e963aa5, 0xf39a3c41dee2b6d2,
    0xc337f083c642c5ac, 0x326540244d09fc0f, 0x7d9e3ab4d3302c62, 0xb37da1be8924987a,
    0x9c763da6bf540cfe, 0x0f8f970bc314d07c, 0x335656ea2de67039, 0xd482c9ed3fd25b76,
    0x1f2017897dcf6975, 0x1ced0201b0cdde51, 0xf72dc604a7d3feb9, 0xcc7d56b46d58dc83,
    0xbee464699a1090c5, 0xa5ef451d531b2004, 0x4cd20a856cb4ddd7, 0x970169e944bc4a16,
    0x89f90c7d0b2d4186, 0xa4b987e539e527c1, 0x10443ec0d629e05d, 0x9e3e727850b13813,
    0xfb3484079de726e7, 0xf61b9cc77f5dce2a, 0xdabe83d7bbcb67cf, 0xc403f26244ac85c4,
    0x94f9e5ddb673fd53, 0x2539b5bea9e042e4, 0xa0e0792c193c9c55, 0x56a4981adefb260f,
    0xef2cd176865b98bc, 0xf7be9ee230df0144, 0xc467949a5b1f63a2, 0x7c3346d8abccfc08,
    0xff8b51397fe24138, 0xd67da72dcf900004, 0x1ab596828dcacf98, 0x3187ef165bed3bda,
    0xf602ccc3aaea3622, 0xcbf5e8f1d07114a1, 0x2ebd64bca3b6503b, 0x0cb6a1d3fb195cd4,
    0x52ac4001fceffa3d, 0xc67a9e73c18d7939, 0x7e5e4784c3272794, 0x0fa95b45bed542d4,
    0x9d2bfba447bdaf76, 0x9d1eabd3f8079030, 0xb367d4faadeb73fc, 0xa9e7aee5f99c0481,
    0x500c56aa0d93d643, 0xfcf8f7b027710c51, 0x81e114045a221537, 0x07dfb6883ee1e388,
    0x8796ed38cb22992b, 0x168376c39835b847, 0x8f27f381e7c0d576, 0xf5be547c4b757363,
    0x118b832a39989d91, 0x2f3c31d66a405915, 0x7ce828125a8ac130, 0xb31d01dad5cf71ee,
    0x23f5e7be1a868c21, 0x1b054ed6ddde1729, 0x91ca5a353e8e3ee2, 0xd8b9c92e9c271546,
    0x9b7699fc0a0134de, 0x501e81379bf2c246, 0x7253c2b466d05d44, 0xc8a2b5a4032113b5,
    0xc8aed8bcd2b7280e, 0x5ae1ebf9df3eecc4, 0xe95d718156e506ac, 0x45dde3635a8915fc,
    0x265dbe08b2d4b37e, 0xf818cdef4564a123, 0xff27134a28ced6b7, 0xf1d1c5d0956798b0,
    0xcbf8372697576021, 0xdf5d0f95613e11ee, 0xe26e3b4ed33cd4f5, 0x382b7a0921f56f2a,
    0x15da58e395165fa3, 0x11c37f1d0e83539d, 0x9811d2e189b9e6cf, 0xd62753c939dfb50b,
    0xd49e362fb7f3e21f, 0x18193e6bfdd38324, 0xc5812db6badc087b, 0x1876caec3ec5d511,
    0xc08c815873ef4275, 0x053cac8b97153055, 0xa62323780ee4d19a, 0x0c9a1f73e9647dd4,
    0x6ee3685fad67880f, 0x75b250699db29dfc, 0x4ba2b2dda35cae0c, 0x8ea0c929e5a3c84b,
    0x236d0270129a9ea6, 0xbcbf3790e88e5b1c, 0x2aa8f987a24cc5f1, 0x3828bb05b402a11a,
    0xdb8a8b4a0bfa5bca, 0x6de96f952d001ff4, 0xadf6a220ba5f90fa, 0x304d9b94cab6e7b1,
    0x80f3dc37a2224dc5, 0x986655b1f08a0974, 0x321b9945c9d28c2c, 0x358ddb2ad0d1d229,
    0x2e91102c4991a33f, 0xc5f97b8d6d1595ba, 0xca4d23d74d95b072, 0x33364dfa0d74eefe,
    0x54b5a47293650ff4, 0x72a9be3cce9c3dfe, 0xe3b98031d92f9937, 0x5c7a9f866089b826,
    0x121f1deb3004ee29, 0x28b4c4310808de8a, 0xf3ae5d4457253307, 0xaf2f335ff94c254e,
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
    0x4067fbb883c66246,    0x0c64522bd391bd15,    0x47c66cd82d3d3ce8,    0x44f49ef4a1e362dd,
    0xd474f62260b619d3,    0xc697d17f55186413,    0x80841f0e536c35c6,    0x88e5f7efe1c44bb3,
    0x73b62cdd5e8ddad9,    0x5fbaa1574ef358c8,    0x8abfae3dfd9bd0c6,    0xf720da6f53139d02,
    0xe6b3c91f596a0099,    0xd52686478fbf273b,    0x37b314c596a17b0e,    0x21e9fae6f033b301,
    0xbc6be98127c55524,    0x18d584322cdda856,    0x64b0ed4d024ad909,    0xbdedb96e1002133e,
    0xd4482f41be711d53,    0xbf5f9f50ef69b651,    0xa7f377c4f907ead5,    0xc5af6d686c858dab,
    0xccf2f1ecd08766bc,    0x4383c0b0e6704ab8,    0x68b5a02cde6bf06b,    0x537f928f8abaa6cf,
    0xcce3db0e6f92f0f2,    0x75e68781b570b71b,    0xc28a1900b6567fe1,    0x8b653d2073e8ae5f,
    0xae5fc8d12035d083,    0x030b796a7f337d88,    0xa9697e23e9bf7be7,    0xa9b8e246989687ea,
    0xb6787dbe3b63caae,    0x89095490d0725cb3,    0xbcdbd92eb9546254,    0x37e2b088f964d02c,
    0x25712c4c2846cd05,    0xf72d8c158c5df9b5,    0x90c91196ca1af9b2,    0x6692e2d9e4287a5d,
    0x046919aa16564f16,    0x1375b0222af8a3b5,    0xa2a9c4d2c62f9756,    0xfd6e0482b27ee0f2,
    0x618958e607739a7a,    0x5b045694794607f8,    0x9a12d5a87df00895,    0x32e40790526cfad1,
    0xbc8e94627e2fb8c4,    0x1c7f6be95442078e,    0x0bbdff59e9342554,    0xb954c10f206b8426,
    0x8b041a600eae0ae4,    0x5aed80ab4d963f6f,    0xb621423f67b3a799,    0x019f2597ffb53de5,
    0xff94aef660199432,    0xa56802ef0b235234,    0xc6e19781f99457ef,    0x3a55783662c4ddae,
    0x77b613edf9216d30,    0x627583af3f7e2e03,    0x1895b0b4e35c0352,    0x58bd01bdb7c2cf25,
    0x6cd7af404246f849,    0x26720c0829a70280,    0xdc6a4180f51e0fa3,    0xf135b2950272c401,
    0xd4829e5a70a86a48,    0xaebbb57c381942c4,    0x464e18e0248f602b,    0xef424841a0199eb7,
    0x8df7fd6f49edd9d5,    0x2b7c1db7a7acf375,    0x8d8de79c1d9cd476,    0x6e5faff8f92f47ea,
    0x3af0c6f62359b39e,    0x8afb9e5fdd889bc6,    0x175508bf8b934955,    0xd6998897bb32b18d,
    0x1ec5ffe46d8894b0,    0x5b2b515008dd7e1a,    0x4aa8b26afa7ab501,    0x4cbc378c43763ec9,
    0x6b83daf6344f6903,    0x5028d83dda844052,    0x806d8e3a3b7938a3,    0xbe692d13842d0828,
    0xf7d0811c9813bc98,    0x21c9d3ad2aed0853,    0x272c9874b69660d6,    0xc7994ec12d846dc4,
    0xc88fe415d70b11c2,    0x693eddd3c8dfe00f,    0x15ab25c299953ffc,    0x7eb71e5e01b663cb,
    0x3f62fe3eccdc94f0,    0x454eae2e4cf3b6b7,    0xbf0a7d83a8abb8f0,    0xfb5c5f2b225c2269,
    0x66d32a60c29a223c,    0xf4ef9fb5b8d95305,    0xe4b02afc6933e7f2,    0xad40b8b939676b1f,
    0xac6f95098b275467,    0xb3b44a80122ee268,    0x5defd3f6e2e55e4b,    0x55691cbde675734e,
    0x32f3646c6d791033,    0x25910e00b736782b,    0x14f2dce12022ed52,    0x7ef97417f7eefb63,
    0xf9e9c56b46410e57,    0xd6743d611c1f7198,    0x26601e032d853121,    0xd5c83c388de91f26,
    0xcb02a61d54f00a47,    0x5181a7392eacd3b2,    0x1a35cdd6bafaec7d,    0x4116f52c277eeb3f,
    0x47664a429bddc173,    0x6716b28ad80f1ac0,    0xd9fa3ee7714edfee,    0xc3bf2b39e83a7cfe,
    0x2310fae91275ad4d,    0x2fb2dca75be97260,    0x99ca42513c194e2e,    0x748f57ce38a11aec,
    0x8d08f000886a86be,    0xac57a7f8d7f08503,    0xbf334d98557f81ee,    0xae2092ebde40d7d1,
    0x9007830618e11315,    0x169e5c0db951d63d,    0xedef63616b10bb74,    0xd324c912d0356122,
    0xe0a34f8637e18696,    0x64b11d510abb8371,    0x8a7ea32a28a586f3,    0x8245ec4055f6bf5e,
    0x1dd837b1fdbc3104,    0x328d34cfc6f7bde4,    0xe6445ae8c3892197,    0x943bbd1c2ce46f5c,
    0x311c9a63f9e1eb16,    0x9d8132bcbf00d108,    0x8582fca8cc679bd0,    0x6d8a03dce6f5a25d,
    0xb4b63eb10f9770f8,    0x8f6783e9f2394186,    0xbba0bcbcd79408a0,    0x72a81ea83323eb9d,
    0x72c0c2fd23881c30,    0x14722b30eaf57714,    0xffca774165c204de,    0x74fc43e56f8f1101,
    0x1fda635919194e67,    0xa30f80fbf198021f,    0x083b3263d5574dc2,    0xf1a69337852c19f6,
    0x1a3809ddcaa15c20,    0x32d6b215c5ef875e,    0xd776b2f1ed4104f6,    0x12e3485367094a3d,
    0x80010cc6a31cd24e,    0x8c37924d7e870219,    0x8ec5608b03ca417d,    0x49b80f22e58d0d6a,
    0xdb8fe0c2b03cd71a,    0x8030e32d54f9499a,    0x6b992443c5349da3,    0xdac8c72815c63119,
    0xc6d511e0358a34da,    0x55b5d8e7a9a4024e,    0xadb02e38b15721b9,    0x2f98d02b7a5146e0,
    0x6f54d669367b1b74,    0x71c02ba5d723bf2b,    0xf2c67ab7a39c267c,    0x16a3ab1c172c0a14,
    0xbf08af53d7bf6aed,    0xf64ea5854e726049,    0x362f518c77432560,    0x321a96888d8cb8b0,
    0x9837cb6afa80487a,    0x4b38dd0f89ec0d03,    0xb9fc21af48c62ba8,    0x3f41e1bee12239bd,
    0xa0e4c7ac1cf796db,    0x9a7f7b6bfbff9c7d,    0x482547cc83a02f47,    0x5971ab6bdb2472b4,
    0x4bdcc438bb820ba4,    0x8b57e4e277d5d719,    0xa4dd4f1f0f54f420,    0x183bfd0b30405ac8,
    0x5d784caba8098b04,    0x003a4ddd11c64d63,    0x44095309e0ac6fe2,    0x6ec5e05e33347dd7,
    0x8ecc96ba74d0ab78,    0x69242829f3e92c88,    0x66119e515c580af3,    0x775fea9412d6e85c,
    0x1d1ad150fc513184,    0x15db822db896ca7b,    0xc11dad631389fa8e,    0x4600b5ff83ba3f2a,
    0x6fed37b02b93cc82,    0x175953b949d6bc11,    0xa65fcf82397b0f81,    0x40a34063c6ffb532,
    0xbf47af6436ccd1ae,    0x13df28cc1d0c8350,    0x3ade7312abdf1da6,    0x2a4ce2d8eb66f7c5,
    0x4da74d93e09fd5e3,    0x9d1a659e49df0547,    0x849c6bee107ceeb5,    0x8c8f76ce279145f2,
    0x67696df2b69c88c9,    0x919ef53a062b881c,    0xce158ea37a57c578,    0x451b0a7331cd4c62,
    0xbb47e96b563e729e,    0xffff1119461600d4,    0xc91987b1ed2085bc,    0x5863780a683e48fe,
    0x8ef04e4e47cdfcaf,    0xb33ea59c6cf08fb6,    0xd0cf06a77db00258,    0x587f616db71bb0c9,
    0xb7f323b4d0187859,    0xe57a4f7a09724692,    0xf1b5bfcc5107b782,    0x73fe04d2dd4848d8,
    0x246c34a61126d620,    0x97f773ab4747b7fc,    0xc1fe445ca8285137,    0xb371a02ec760f48c,
    0x9cbb3b983bac8183,    0x4e9acd0fcf8e5c22,    0xc4e4c13f5d7e8e59,    0x231c6de35cd42e86,
    0xd70b4c3ac74224a4,    0xafd93d926e372acd,    0x844a9f67eae2c573,    0x25e5fe5f87c8b1c2,
    0x7bf9523741ea15c7,    0x017ea3a21d0b2a36,    0x623ae20a3c685cba,    0x901569851ce17778,
    0x69b97a1ee5f26bc3,    0xfb703557bbe9ef00,    0x6843268405f7fb53,    0x7f7009c87c9cc6de,
    0x2344e0bf56584bb1,    0xba66f3d8b28e1185,    0x272e52d3497c440e,    0xfee1e2b52dc29cde,
    0xf278730bddcba14d,    0xf56e7ae286f0b234,    0xbb1d4f1f04a33905,    0xa716e4ab5d02cc5b,
    0x27e6e732f11d68d8,    0xaf1affdde0d62527,    0x0f53280148939407,    0x85312e867f61e616,
    0x5e942bb30cc932ee,    0x989b049f01a8f67c,    0x9ee0261e5a9d4fd2,    0x337bbb4d0958ac1e,
    0xab294287ed39dba1,    0xe2ab4ddbc0c8dfe2,    0x8a3b40dad86613ac,    0x8115b6be2b9b4868,
    0x23494e2f940c04c3,    0xe94cafaa5ac5a696,    0xec2c34db4c3b95df,    0x81de695a86c05362,
    0x9edcd2a35c2ded16,    0xfda01493d8bcb502,    0x9a22341fd799ed24,    0x8cea5b97f4da47b4,
    0x2c3169bb80572363,    0xf169119fd50e7111,    0x6bb28eee1182dc47,    0xe76f2ec2fe799537,
    0x8625b95fe7854555,    0x9fd7b7a2209c1d9d,    0xf37341863290b4a7,    0xc20b174557ae4204,
    0x16973246941214ad,    0x75b6212a5862ef76,    0xe6d5e991ddc380cd,    0x82b501f610373be2,
    0xe9642d3af5a78e77,    0x6b3c4d138e42ee4a,    0x7525105362e42f76,    0x3c5398b8276226b9,
    0x4b3e8760e71f7ac0,    0x483fd91d3db36de8,    0x0afc01e97e65286f,    0x82c14233e8e3de5c,
    0xe14325f89ec8b9df,    0xaad46bceff37541c,    0x04a38974f02e85ec,    0x9f9babe21c512392,
    0xb82dfce2011d0e3d,    0x0f354f493a67c27f,    0xd5d52dcf56d7b999,    0x1efcb49c0b123653,
    0x0512fb9e4aaa801f,    0x6e1582b47e2d1ad9,    0xe00279f9d815fe38,    0x3aa2c83e6fa151d5,
    0x5cdb609ae3db4c59,    0x071fd72e7da0880d,    0x55a3afa84413883a,    0xc1b4fd7fdcc36ab8,
    0xf650206e5d967be2,    0xdb2ac52fcf5c6627,    0x63a38d93a05bef47,    0x715b5efc2a1bd5c3,
    0xd543676b95b97171,    0xbaefce908be9f46a,    0x70df3346450daf8e,    0x94f7410c26e78a08,
    0x5c851dffc2d48c7c,    0x193d9cd27f1d6375,    0x787ccf7cd5103300,    0xd9814f8ef4858027,
    0x55d25d75b30cac0e,    0xd017a2932dd13e93,    0xc46ff9f291df33a2,    0x01bccd2282e70ff6,
    0x4a2e3b2359ad4b3c,    0x6a5ca5bed134e4b2,    0x681432bdc1d2dc30,    0xe1beb9ee6749d450,
    0xa7e34ccceed51aa6,    0x87bc6a609afd08da,    0xde626a00f8ff7ae2,    0x408dbad5efa299ee,
    0x356557923ad47624,    0xdf6daa9e4714cd0d,    0x5eebe4488c15dc9a,    0xe83eaac343e8145e,
    0x9fb0c20936953c8c,    0xae94ccd1819e6411,    0x9d0a008aae4ca545,    0xd7c3e3cbae8fd8fd,
    0x506fb57405bb58cb,    0x733f5a625e88e475,    0x943751f4e4f47bc8,    0x6f72e631e92a85ad,
    0x5fa9b9e223d0aa0f,    0x3ef0c9f59506ae3e,    0x5ac0eceb129f0325,    0xbbd5043dcda115a4,
    0x2a718390423d2858,    0xae381318ff95f507,    0x6c014ec5ccea9fcf,    0xa45cacbcb2992b47,
    0x77fa19d41914fd90,    0x5fd9d1283c42c710,    0x37045751397df1ea,    0x971d9afa9781c529,
    0x32248c8e23c0033b,    0xf75326c8e07b11b9,    0x467e8174f289c33c,    0x22c328c8b03012bf,
    0xfdf0bf6f15ebafa3,    0xa742bd67fdc87027,    0xc163504492c1094e,    0x1c9187e929f6b911,
    0x305fd344b8e39bdb,    0x1cb2ef908e74f1b3,    0xdf82670240438e4d,    0x04ab4cfa4b06062b,
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
