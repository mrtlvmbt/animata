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
    0xcc4a495d96af4d0f,     0x5386eeb6ddbda6fe,     0x73be366dcec6db39,     0x8f66ad194b66746b,
    0xdf076975c90cfa29,     0x3f1e8683572f3dd9,     0x1615b8b8e5595010,     0xdcf35a4c54a589be,
    0x0117af7733d39553,     0x8e31d04c783158f9,     0xade1e9782f43aed7,     0xb5cd0a14532ec1b0,
    0xc3de6856cc921533,     0x6d6d5b91fd532068,     0x147ad9f4036e99cf,     0x6a47d5134c77b191,
    0x94fceaae58c51b21,     0x866a3af741d3ae4f,     0xbca61af68ec81855,     0x3a8ba67ec322889b,
    0x53de3ec24f398d07,     0x2033a5c643cb3724,     0xad08c07e7cddcda7,     0x0c72e928ac24f4e5,
    0xaec4fea91866b340,     0x444aa266d2ec16ec,     0xef4b84068d84ded2,     0xf8735bb41d84cc52,
    0x0a2d6d5c60c6db37,     0x6fd2053e3ef4012b,     0x9c41a0ca268cf832,     0x25b59cf3d6b65bf0,
    0xae4ec35921871117,     0x7eb73661a08a18f2,     0xf49f66279b25f22d,     0xa2770637816e02a5,
    0xa202d9dc67cea897,     0x5d5719d5cc0454c3,     0x53ef39f4af533e77,     0x5578c6f858c12ccf,
    0x51da862441d164ba,     0x052a66f394dfe6ec,     0xb578298c48007ca6,     0x14f33849f35d5de7,
    0x83c4560b3c0e4d16,     0xde31f8f86fea1923,     0x70c2d87b06d76100,     0xa998b358b5c8578f,
    0x1746585a85744b8d,     0xd72bf05e6260cf05,     0xe70aee9b744282c2,     0x862823ba3df6492e,
    0x9ce8134d32849f41,     0x7e44d8b3e9253ec6,     0x2759c0b1d9c3e1f7,     0xf76cf9a3c330fe49,
    0xf15887b8f1c72fc6,     0x7ec9b5037726e49c,     0x67d8f16fcb446f41,     0x21a8e1c5d84687a1,
    0x7317d179e6d01065,     0x70d5046d91c77aa3,     0xd5cf75c35b1fdc61,     0x8a5e56ef2e024f65,
    0x12774e1a2ed92d6b,     0x5128fa9a6560e2b4,     0x7caaf202ee59facc,     0x7d224d879ff16204,
    0xfb5d1f121c9e7d1e,     0xecfeb5b83853e1b1,     0x6c6357321e59de2c,     0xc0bedb5c3f290ecd,
    0x1c2be9ef994fc3fc,     0x8de403e703f2d5b5,     0x64073b8b628d5d43,     0xb39f8b71ae4d7263,
    0xa1ec2a3f1ecd5203,     0x7eb4de0bfc37a913,     0x981452847a79e4eb,     0xe452c9c99ba5b751,
    0x7e139985be628caa,     0xc6dd3026da78e77f,     0x929a8a909bad3a31,     0xb5c0bc44ae0c3dc7,
    0x7763e45eb48dde95,     0x1265208f4e3274c5,     0xb6bb93e3df5a506b,     0xc0d2ded8648b0f44,
    0x78b9f027cec417f2,     0x95786a46f8846fa8,     0x260feac4da715c7a,     0x3eb562025aa2e884,
    0x61453aa94f104559,     0x4e1219ecd8803586,     0x618296da0bdb0cac,     0x3ab31d5a76277e97,
    0xc2c4f41f177f3ba0,     0x558ce804dae358e9,     0x068e07f2669eeece,     0xe8df1e6e3e62d0cb,
    0x89d92732a108e1f6,     0xe577f2936e6e49d8,     0x3c066f63c1a97968,     0x989a941ad5c1551c,
    0x2734fade43ade772,     0x77553be2e4ba9ab2,     0x23879b4983b90e3e,     0xfca711159f3d7dc8,
    0x96065f610995d863,     0xd212babf441a469d,     0x51609ef6d2121824,     0xbbd562f6a016a470,
    0x97c399ceea8d13fb,     0xf74c8a9a64339848,     0x387a4071b259df31,     0x9d83692c9cb1e35a,
    0x6ffb3bb1f1b1efba,     0x09b0fd187c14312a,     0xf97e97b7526fe268,     0x9acc6f17b556308b,
    0x5ecdd14fb49d4f6d,     0xbf1074e62a941f3c,     0x7a29110b6414452b,     0x614cc47cd20b0cfb,
    0x677291da2fa7f65e,     0x9269d6472f841bb3,     0xcdc7e9650a0ca2d3,     0x0744a56842f4c94e,
    0x8c984313d7531355,     0x011c4e05516ced8c,     0x82f668d31277420a,     0xac4140879385c719,
    0x862fb2a95996ae5d,     0x7d4cde788caa347b,     0x48a3b02fa0292188,     0xbffe7076e8bf2ff3,
    0x4e5679a8c50733ae,     0x60b0e1fa24c68e7d,     0x5a63b8e39501b822,     0x573b402758a6829e,
    0xd5cff4be34b86c61,     0x1f4f54ed17c05aff,     0x62f5b477565dc5ff,     0x41947ac572028e17,
    0xc8129939d5be623e,     0xd5ae5c02d8f52d01,     0x11e41c261cbbbbe4,     0xf3933e6804afccda,
    0x784c65de2d33c3c4,     0x8018c5ddfd270f62,     0xc1800bb39850fd29,     0x9988ed5d6294041d,
    0x4711c48309b4ba99,     0x07f00cafc72c0882,     0xdae1fac9ec6864a0,     0xa6e951898c34bcd5,
    0x16353a78147789ab,     0xade6fbb064fa11f4,     0x16d12503d64837c1,     0xf9c622c82fd3e046,
    0x13efc561f4318d99,     0x3a92f836a4a0e74c,     0xfd292f208bd76e70,     0xbf77c877f7c85f21,
    0x7d11c94f2ca20745,     0xc915a4421544f2d0,     0x165419855e9e9f80,     0xa55f902b74b71822,
    0x5f85df6804b45997,     0xcbb3f92a915c6e14,     0xa88212b3215a3dcb,     0x6782245ab432ed3d,
    0x5d59724e8ed46324,     0x06da5812b3c8d9a8,     0xb67ccdb36765eca7,     0x715876d4c6c8b966,
    0x04c80586a9ff83b6,     0x942fa050717c74c1,     0x99a9b1a80dd30048,     0xcb623cd55d6763bd,
    0xb58cdf3dde60ea18,     0x4cbc97fc459aa4b0,     0xe926ef1805235c78,     0x3e28f97676e6c525,
    0x17e0cf16fff61a0c,     0x6631700173cccb71,     0x120697200956ac01,     0xa9765f36a9b4ca35,
    0x3cd205ddc18cd712,     0xd6bd60592f0966d3,     0x8aee4c94b4a1c6bf,     0xb6f1ae3f6211c3e3,
    0xbe4be853d851a952,     0xa9307d3f1541a2d9,     0xad7df9c1e6b37564,     0x5d0dc889ab22e3b8,
    0x7ecd3bd389f042bd,     0xf23fd4bc0753c6d0,     0x5b6acd125d814778,     0x52bf0ad8e17a0380,
    0x1a4fe6981e2252ba,     0x61baf56f15597fc4,     0x4b7c2ba93124f77a,     0x6be6aaf1de424de2,
    0xec33609a53c51cea,     0xb90efeedf18f1d62,     0x1881426c08700f6a,     0x56bbaacb0dfb21d7,
    0x7dbe41186266ab2d,     0x470d20b3045b79e4,     0x3da6aa27900875e5,     0x10169a26304d7be5,
    0x382e9ee9b4463e93,     0x7e05236010eaf731,     0xf1261e381010335f,     0x5d57f544d05345da,
    0x8a4347aeaf9f6250,     0x586aef521047bd6a,     0x2038893aa6ec1a14,     0xa73175b05be0bdd7,
    0xd2bece48ab4b6f6e,     0x69cc9e2665502b2b,     0x2b25879e357491aa,     0x3d992f7879879ced,
    0xf881c2f267d4ecb0,     0x510e6003cf556b89,     0xdab40840b4fe798e,     0x71aed043975d5748,
    0x517a7bafbc7195d8,     0x5cd3f3070182a09e,     0x8ce6269e175e9678,     0xbcd4eab52d68c42a,
    0xb9c577c5e5980486,     0xa46837c75afc2906,     0x065b3c8731a6b92c,     0xd9f3e018c2ae3aed,
    0x4394584497afc6a2,     0xa2fa460b674b3469,     0x6f8752f7cbaadc26,     0x3d226d370a052a0c,
    0xf457b8d3347fc136,     0x419425ac6f3b7383,     0x48fc29a4fbefed12,     0xf84e1aa2805f9de2,
    0x74d3de30a69b0310,     0x66538d41f8ad9cf3,     0x9869c6ffb79e6ad2,     0xeb82d4530c073d8e,
    0x67d39bbe820d02a4,     0xf79079401375332a,     0x3b06bcc9206e64d3,     0x9859d12a632dac46,
    0x045820acd3f964c5,     0x83b441fbe5849d70,     0x1bb5fa141490c97b,     0x18750f80339fefa9,
    0x4fbb27129c6f688d,     0x1ddfdbef81159418,     0xd0734a613195ff05,     0x1a50df73946e25a7,
    0x8c6e6a1622f01383,     0x10982edf9d0b16f3,     0x9bc2e55e799c563b,     0x2352c39a8b089aed,
    0x8f3bf90f70c40313,     0xfc479de869f907f2,     0xdf8da7d5f9db5f8f,     0x873f889b304d2044,
    0x49a18de397b9d2ec,     0x7736f55c09c5a979,     0x21d68b38f11be8e7,     0x0dd8507b128394f3,
    0x14e0b6a96483b0d2,     0xe3f664f2329c8bc4,     0x779a39baef75d87f,     0xc92509ec18128471,
    0x048eefe1e981b2a3,     0x3d71944018bfa17d,     0x28856273ee3a44be,     0x2e4f82625e04bc9d,
    0x7018c89f2a591f72,     0xdf1e9df9c3604793,     0xec411406fa6da7f7,     0x30b4cd851895a87d,
    0xa0f71598b8550617,     0x1a5fcc3004e0d9b2,     0xf8a3338c85432a2d,     0xf1a7e91c336c323c,
    0xc45164b32c0f89fb,     0x93dee4865787b86f,     0xbc63fbe31b20b0d9,     0xbb646fec043e97c9,
    0x8c623b9bb83de645,     0xca7cf35c0549ac21,     0x6880fed25418483e,     0x09fc7df66d61f7a4,
    0xc070d81ff8b1c5f1,     0xdb84f7815ab0d289,     0x54e78372b1f08804,     0x353275eee30172df,
    0xd426f3cc7e429e26,     0x910d531aaa50fe38,     0x9cb4b36b04a6af35,     0x1add5b125564cd4f,
    0xc108e2ec5a65cf4d,     0x10095b3399fef483,     0x21f61ba87088af11,     0x3bb95170b74b6542,
    0x96eef7301870404c,     0x320663e94882006e,     0x4007f5c16613158d,     0x65f52ee22a4c808f,
    0x4a2d18d1d20692ac,     0xe3ed81e1bbbb501c,     0xd2bd5a00e6784f1e,     0x99589776e9cea836,
    0x60cd73652f95e562,     0x74c04d3d60d9d5cc,     0xcd8b5626b41d9303,     0x4189d160a6a1ca28,
    0xe6d82c0b6d4f1614,     0xf2d5f0cd54995001,     0x058390eeec6850d2,     0x91e656552c92f4fd,
    0x9c97980fe58b79d1,     0x51c1b16bf2a532b5,     0x55bac9b5cea84e66,     0xe16e564c32720a57,
    0xfba257b7289d5d7c,     0x561246b9e43df0e4,     0x6b0ee1fccb7b5238,     0xf4ad0b26227eafe1,
    0xd4e1ed72238a7898,     0xd5c199cc13ba30bd,     0xee57f3945764e866,     0xdbd5c8e428ffa0fb,
    0xd6c582db509ab6ed,     0x4537acf0016c1724,     0x4223ad9c18b1e9d6,     0xee01b290124fb545,
    0xa2bdf31622312054,     0x046833d302869dc3,     0xa9303a536aac5c8a,     0x4dad6ae4d60ada57,
    0x4f059b82f8805da1,     0x669d44165ebe800e,     0x9fea4a19b7cf4738,     0xa78e5561422e077c,
    0x2aad913b3bba9189,     0xb34cad32a75e4ec4,     0x2ec11a25b172d217,     0x40e649ce3d8ea209,
    0x3cf2fc9c0e966fd5,     0xf64b64694c4ef7eb,     0x3219d20bbe278573,     0x9ba903a8c2b061f0,
    0x4c934dd5d1a88978,     0x323cd0c23c210be9,     0xcc5f864a96fdb01a,     0x12bf7fea637bc440,
    0x6ab3c0b0440c81e6,     0x530dc6734c430717,     0xfb00dc070ed24818,     0xa1cb11b86b96cf2b,
    0x9f629ad42682bf60,     0xb8255a1ab333d887,     0x7393b8c193f52201,     0xd1eebde97c72b1e9,
    0x9afe690023835ba3,     0x90e5e62b771cd245,     0x4030e5df596ebea9,     0x2297defe6b8d3791,
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
    0xa2c2430367fbb6b0,     0x453c076da215f097,     0x98ca44a17a4313f6,     0x187a24d298c21ec4,
    0x6deab1c3828dd23d,     0x5f80733ca12ea524,     0x2ea7b516cc2338b0,     0xfc832b53a1d5734b,
    0x1197b297e0176a23,     0x45c1cc5873db85c3,     0xb19a614af14f81e1,     0x218b9a72e9a7a8d8,
    0xba3ba71c93d58fd7,     0x38bec9901d06d3b5,     0x22cfd130834ef636,     0x426ce97fd8c973cc,
    0x064c1442b69b6ede,     0xe3032ed650d81570,     0x6b64d1f1b0872beb,     0x47c8fe4fb05684c6,
    0x1f857283302c593a,     0x4f8478a837ca48c0,     0x50957cacc03bac1a,     0xd0e234328fa7b1c9,
    0x8d7d611d457cc98f,     0x85ab3dbe0574f735,     0x55fb1e1eaa78af44,     0x257958e63684f062,
    0xe1d45bbb9556627b,     0x370d3c78bef0f516,     0x4fad86264612aabf,     0x4f210564d85d67d9,
    0xfcb350e95e4e7025,     0x5241abc0dae55718,     0xfef34d419f43758e,     0x3c6a18d69c8f3b69,
    0x22e4b6b9dee514db,     0xe2916bf616841ea7,     0xb18092f1bb7a880a,     0x105dab5fc88a4c9e,
    0x95167b7b9ff4be30,     0x709e84bb8dd63fda,     0xe799cf8aea2c7716,     0x4a58c72a6d8c344b,
    0x3b33e8c1039a1516,     0x17852c8a4ac97b49,     0x96c8fe5b1ac5a4db,     0x9c4e18ff9f60c87e,
    0xe0ef57b594b4f7ec,     0xabcb40f509b0b154,     0x7827fa062ff5b0c3,     0xdbf2e63eb2220638,
    0x6faeb4894bc2d2d7,     0xdb65e7982c7a18cd,     0x7a084bcde9e700b2,     0xebdc0d7b1b2b0e91,
    0x0f7e2c5cc4d128fb,     0x1b89c4e5eb113e3f,     0x6e789c9a16968f3d,     0x2f40bfc88b84ce49,
    0x367e8be62b70fc0a,     0xf3eb67179b8d5c80,     0x161bb174c229c902,     0x4d873cc4e55a77bb,
    0xdb4b237f71815c0d,     0xb8c7829198407678,     0x9992ef120a063ca7,     0x2f6d7111f532c3ff,
    0x6e79b52bea8f59b6,     0x01551d6e489cd6a6,     0x2f6d17aab230300a,     0x9a3e3e5c87445cfb,
    0xf363f60c7971ac89,     0xc0da3034f5d794a8,     0xb940aa00434dc58b,     0xfc0aad507ecc4d8a,
    0x69f4eff077c16d70,     0x03c7da2730d460cf,     0x012a33a5e64a2cdb,     0x12f9b615c2a464a5,
    0xcfa9e58eae39e4e7,     0x63d9e202c278ceef,     0x06d6895dae4ecc0e,     0xe992d328faec5fb0,
    0xba0c76ee6e9d1a6f,     0xab48160fb7724a59,     0x3b42059bb2a2bab9,     0x4c3145be39c80816,
    0xeac8d44eca287bba,     0xd5b86da95e1fae14,     0x9891c6b6da02e9ee,     0x44da37fb5a67f22c,
    0xb7294aaa6980b3ca,     0xa3a617e19bfd5fe6,     0x6ccc49e90204f47e,     0x093a4f7bb1d512d9,
    0x49c5099381b54909,     0xffa5b6a81f97ec6c,     0x5301b9d3a5b02d50,     0xe9a1b76a3209fcfb,
    0xe2bb08c992964da3,     0x3a38da92273ed643,     0x68c8325e4ab68111,     0x8ebd9e41573f72c0,
    0xdb13df0d51760392,     0xb4003e030d9637ce,     0x28be4fbc185e9666,     0xdd6a75c63fdd3a27,
    0xf7d5410a9ff21c9e,     0x8a4253ec2b34b9c2,     0x3a04bcbe3a492343,     0x31ac8dc939d63aba,
    0x1086fb3e8fb85d81,     0xf87e9e3f46c0f503,     0x0f04a48b598d723f,     0xa493cc4b5eb4a51d,
    0xafa22d7a8a490d3a,     0x939bba40900d9373,     0xee915e3bdbb257c9,     0xd531658c7e465458,
    0xb4c1253f375c9bbe,     0xed793b2b5e3a1c9f,     0x16425a2a8efefe8a,     0x90ae2253de0c4e01,
    0xbb64fb39d5779974,     0xccbe47ec147b89ae,     0xc819da77ed7c2559,     0xa724efb55630b030,
    0x80edb79756c3bf2d,     0xc627262abad50d6a,     0x4ba96f929ba69b1b,     0x6f037d483dec4d34,
    0x7d842e10bc13dee8,     0x0c8969879b0c1482,     0x1e4e302b4b2eedc8,     0x936534596db47022,
    0x5e41269a9a924916,     0x1dfbc12448623f59,     0xea88cff2b50a238c,     0xeb9582924da4f30c,
    0x965152699c70bed6,     0x0f4a156f9d1bb1c7,     0x8e4d25acf68f656d,     0xbcc7c067ed00e887,
    0xd727625fa61c7efe,     0x6c54e9d8e5ad52e0,     0x7db5b633ffbe4b8c,     0x27c33cbac281c705,
    0x138bc934ed444ab2,     0xbd70add41b1a9d45,     0x489a415aa9056e9f,     0xb1d25a3d4d21d4ab,
    0x0a181d5c946bc471,     0xe4d736f2990720fd,     0xd56e56e9327ccc8e,     0xc41fb17b59f125c1,
    0xad6a7a5330c3ab9f,     0xf37d8a412b736045,     0xd83ac6b48e8eb92d,     0x9e7e92d9efafe332,
    0x6463098b8c6959fc,     0x04f2749b8fb820e2,     0xbb8be0b3fbc6e15b,     0x8e0c56e6b6058b72,
    0x27b592cbac6d6f0c,     0x9aeff66c8df3f039,     0x330df91dac0f51cc,     0xde2b2f3b7a9c7955,
    0x6633552b330d3e69,     0x1aaf637406c13a8b,     0xe40f35f8cdea1cbb,     0x3ab61440b54c6233,
    0x09927f68daadeb74,     0x1a5d64f967a608a4,     0x22541d1b7ccbe2b0,     0xaa00ca7929940af9,
    0xa18daac38f8cb873,     0x05aabb3057ba81c0,     0xdabfe8a7bc9b41da,     0x618a3d717ae72392,
    0x4dcf0cedd38af786,     0xbd8feae8aa9669aa,     0xfdae668a4d80d9f5,     0xe2f81b9fe457cf6b,
    0x2d702a3a8126e17d,     0xe3ce54bf5943ec70,     0x3938d85488799eb5,     0x663a86d39b5ac17f,
    0x76c239cbe3591b37,     0x0752499138dedf5b,     0x96a90ec13f9aacd1,     0x47dad289bd9e84d2,
    0x6787618a7ecec5d7,     0x91039f432279d5f8,     0xc8874973cd18c7ca,     0xde978a8622755cac,
    0x10d4d46fe279abbf,     0xc6173f7309a3061e,     0xaccb81827ae6f90c,     0xb6433d2bab5009f0,
    0xd4cfce8090a1e4bb,     0xe26e7c195c4fd102,     0x7c7005e5b83f732d,     0xa22347ec84ad1b0d,
    0xeb8e304b9075db0d,     0xecaed9378192f30f,     0xe2398be233e17dcd,     0x47bc9caf0258adaa,
    0x8457df0c8208bfe6,     0x95c32911a245849f,     0x7ffe1396a23ce7ef,     0xcf5ee862efccff04,
    0xf62386f03bcd5461,     0x74a7fa176eec242d,     0x2a0add6c835c4517,     0xfb717f61919bbd29,
    0x34163ca894a0656a,     0xffd8b097328afb87,     0xdeadf3de509d21d5,     0xd1e49c70a0e42835,
    0x209fc6f78f1f6f21,     0xe25442fc9a17de45,     0x3e63fc85f8bf1320,     0x9598a573dabd335e,
    0x9a246d5a51bf2349,     0xfcdb901d347ac070,     0x487d9e5bd2f46fdb,     0xb5ab94267cf1e0d8,
    0x521c53d976861068,     0xf8d83f3a91f2f8af,     0x6d1e57b45debf389,     0xfe1d08f30e3221e8,
    0x61f3d7181a0b61de,     0xc22efb6cf7f18705,     0xfcd4fc8bb708822a,     0x8e401de32b5b09d3,
    0x2db501f9dcaeb124,     0x8d09659b8b203622,     0xc06b5c7294af6490,     0x015626adb9936139,
    0x26854fda6b01a4fe,     0x3c94b1e6d14064e1,     0x054fa39e9d91ef5e,     0x2230c9a85acd27b9,
    0x201f523a8114048b,     0x7f3b043c3acb1992,     0x433fade5eaf9412e,     0x8e8c3e2f9efd0142,
    0x63686b32521d7e93,     0x54776f30deb21ae0,     0x6a54115a7918cd5d,     0x86f9d52c5654ad93,
    0x7414e764fdb4acf6,     0x11c0a9a6d41f7ac6,     0xfb8fb631e65c5539,     0xbe1e3777844e4334,
    0x985e48a7c31f1d04,     0x3303a4f7eb705897,     0x11191595ac343de4,     0x4cc8678b697daee8,
    0x70f939146b489f86,     0x25dd2990ca5c265a,     0x7bd95b42a3b9998d,     0xb725d9c83c26ae19,
    0x7afb9cc8a46c0351,     0x78149cc30e805c31,     0xd41ea159c7d15d06,     0x2a499560e3f93745,
    0x6b4fdd7fd5e927ac,     0x44dc739933a5c5d9,     0xc3ed684fc3a1303f,     0x1c8e46975ca9fdf7,
    0xe136b1966e87fa77,     0x281e03c18a987f39,     0x98a573175acffaf5,     0xa02aa441f0c07708,
    0xe6b9f3624e301986,     0x65f5be3ef7bc2a25,     0x6bd29a146d3db723,     0xb458ddac3dae69e2,
    0xbc41cf478abfc8e7,     0x15af4293c200051b,     0x9840b4ba8363c604,     0x6276ddf4c9e21efe,
    0x4c59a3a74f1f4325,     0x3a1a665e4cf3e2b0,     0x6b026d5433e0f689,     0xe8231dfac2fc3c21,
    0xb6e79c6f9bb308b4,     0x6bdb5e34710303af,     0xa4782107116ce9f9,     0x48fef0588a745620,
    0x925f4727876bd420,     0x9b5d50997fc9633c,     0xa90de6cf840090e1,     0xe0dcff40e7167296,
    0x6b8d3b7d47767972,     0x6f66efc4b682de32,     0x3a59f9941aa3b3a4,     0x189c04a8524f2fb2,
    0xb79b8981f0bec6aa,     0x34a65a0516334aa1,     0x7bbe32b5a8ca0d42,     0xd56c4323ec54a2bd,
    0xd90d804aa7f72768,     0xf45432e18cb0f3f9,     0xad8d71b5b7c2f679,     0x9c420ff85e7b88b9,
    0xa4b7fc76086f07f6,     0x2b9b68a56470f8ab,     0x3895889be6d8a3ae,     0xbc659a70d1223e74,
    0x1ff31af84eac47ed,     0x66ba9e48310ea3a5,     0x498f659371f938d5,     0x6235b4e60f12372a,
    0x6e4cf0d504e1e504,     0xb87a0f3e58b25460,     0x90aa101269b75fa3,     0xac87b46a7a9995c7,
    0xbf1dd8f3d8506486,     0x52cf00a601b0137a,     0x7a3ccd6e76521234,     0x3a270c915587e487,
    0x36e48880511c8524,     0x98ee0c95bfdf7c95,     0x39fb52f5406aace4,     0xec2f0cc5f2a8776b,
    0xe0dbbe3fd3f757e2,     0x1283978787640fc3,     0x9831b635ed212c00,     0x2d6d62154405eefd,
    0x9246d578ea004f22,     0xe4967036ac6afadb,     0x9bb00e8c1aefe45e,     0x7b3d191632b633cd,
    0xb9ea01f9bbe3f882,     0xf1821c6804e60807,     0xa701c43b070cb07c,     0x4ccaa9dbd2fe5a79,
    0xe0784a2f14b1612d,     0xc70f9402d9d30b77,     0x548210edd8befff6,     0xf83aa32354d5e224,
    0x1abad71cd511d6a9,     0x4cfe44cffc7f8124,     0x06063bd05444c358,     0xa27061d04385586d,
    0x54b46a68ea451188,     0x3abc92624d3eb963,     0x6eac3328426dbd15,     0xffaaaef41cb16d25,
    0xa91a2e3c2a00c796,     0x2e1d08e99fb07043,     0xc5827632e59bd4ac,     0xbfea76067f436998,
    0xae9ed9803d23edd9,     0xcdd0423b94e44646,     0xf0610e926db20490,     0x536a6d0b981a4024,
    0x94de5968133ab53f,     0x58afc10bddf7d1a3,     0xba9d37ce042873ba,     0xd71b6c82e8d17833,
    0x993071f720ec9429,     0x6e7004ca0fb5405e,     0xd0428d04a3079008,     0x32f32c059a75981c,
    0xf9f6618ccaedf5b0,     0x74c3d4a72d9afe06,     0xf376e177ed7f14b6,     0x41cb331289728b4c,
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
