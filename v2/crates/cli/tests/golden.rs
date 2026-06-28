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
    0xc09c0cf0915181c9,     0x5c3f40f21ecf15a6,     0x2b7d7f5f6e5cae11,     0x4127090071aab00b,
    0xdbacb91f5395d3f4,     0x2c6521a52eedc3c2,     0x635e5b3e1b7b3977,     0xa31e052804fd48fc,
    0xcd6a81fa65bd94ce,     0x592f982140dee201,     0x076b48db22429017,     0x63b5a0b1cedfde86,
    0x888a827f8e21de72,     0x567df1cf2e8a16ff,     0x4e4a24af89b793cb,     0x5e1fbde47592d626,
    0x17b1784ad8610203,     0xa27312add5fd6aed,     0xe60d05ebd3dae423,     0x2b696eaf2edebb3c,
    0xdf832d86be7bc124,     0x0fcd4a98375a8f71,     0xa89a1abeb92163bd,     0xa6997171b6155146,
    0x515ab7f7409cd642,     0xa0797bb00117843b,     0x51427dee61d22c9a,     0x4d51d20822556e81,
    0xec98b4b14246d612,     0x423f5da6eae08f05,     0x7251d053576dd887,     0x192ed8ca87a07ba3,
    0xc4fe6b7c556209a2,     0x2761c4b9adbed29f,     0x705327dea67020ed,     0xb83003b0d3ce41b9,
    0x4f226badd91ff818,     0xa5053a7ac66fe88b,     0x374b6e3fe0c72345,     0xf0f6e983ba5600c6,
    0x123945a3394641ff,     0x89310bbbfb754611,     0xb9abf93167ed9c16,     0x584bb6112334f2f8,
    0xf70de996309a8537,     0xf5615ad9cb0094ed,     0x4963ba78736ee99e,     0x7449a89c128a4e3d,
    0x948602d45c3a0e92,     0x18463452df7f0a30,     0x142bc27d7eee8930,     0xa661478e629d3136,
    0xc793e9e36ab310d2,     0x3aeeefc60f57b568,     0xb03bd25dc4429f36,     0xb4c13bfd15799f5b,
    0x9f5afdd06362cd57,     0x047a6a8c81714a3e,     0xaf8abba5a55ad1aa,     0x08008862a1b7bb11,
    0x49e6f16530b3c422,     0x65139510c6f299ec,     0x758769b3de4c6343,     0xdc9385b35a36990c,
    0x1edad39b72c000d6,     0xd435551ea7c7f749,     0x00a725249cc93e46,     0x277140096fbc3500,
    0x1d50a4f1789bca10,     0x62bc46de19b22647,     0x7e69d5c5f347e035,     0xb353211aa0d927bb,
    0x5d4e83b71341174a,     0x26c22b25348bb62c,     0x9fef323e0e713cce,     0x3b1fcf934bb7a626,
    0xb397fa09bc2edf68,     0x33cc82d7d984a4d3,     0x1fd2bf3a88147366,     0x179a2ce5f04eb042,
    0x93d1e36585cd2711,     0x2d47f28557189047,     0x68eea8545219145f,     0x43cab309ae65a790,
    0x99b1af8a970cdcbc,     0x85729d7dd31288e4,     0x273b42d34ffc04cd,     0x709f9bd5822b45fc,
    0x141d6198ecdd65f8,     0xb09f713cf5e85c52,     0xc9049cb722f90a75,     0x103f1b9b99a38f4a,
    0xd0957082808b7cf4,     0xd6390ac11ea1da0c,     0x8a2ebe4db9c928c6,     0x1fb7ffe160992113,
    0x587576a32536c02f,     0x43eec550999b3c22,     0xa0f6cfc737cd8bcc,     0x47df1cb33277a0a6,
    0x436fe9ddcd88843a,     0x309914a0d756932e,     0xfe79a00327231657,     0xe36fdffa062e5d41,
    0xe0e362d02aa97921,     0xf69a8977e8abb7f1,     0xbb41946762296f63,     0x480911b9d8702e7d,
    0xb1b9c424665f1158,     0x6fba8e6e6ca447f7,     0x63129d0f3f633651,     0x0aa3bfb6bc402ccb,
    0x1ef0ad73e2f4c711,     0xdcfe488f0d6e0ece,     0x7361964700c129c5,     0xdce70c59284dcb92,
    0x9e7053a3589d0246,     0x6f59665decb7f745,     0xeee7689854a8bd36,     0xd5baf6e6a247a1c7,
    0xef8ce8ace27e29de,     0xe2483c7e2b67702b,     0x903d97505c53db1a,     0x2485863b9ae28042,
    0x19c05d9ee6477692,     0x0274b23d493ca199,     0x4f4d5c56abdab360,     0x52c418bbfc055881,
    0x03f3a60103452f3f,     0xe78db700a8dd1cd0,     0x3528e3327a4b41e7,     0x73ff1c32a9c86a3f,
    0x77cd31a5fb3dbf2e,     0x978e129d80782c03,     0x236f4866feb86859,     0x03ff80d3e1bf9a84,
    0x1afcafe48763e4e0,     0x9b37c540b1e4f5ac,     0x55d169033fbe8bb7,     0x4704cf8992eb8a91,
    0xd923995d3b6f1931,     0x66d657e0806ac7b6,     0xc7703dffbaf1fb5c,     0x14e636de08d16c85,
    0x59e109b5603dd934,     0x31c3204eeca1ab09,     0x3bf97804d479ffe0,     0x6578f37b06b42ed5,
    0xab6660b1d3f66a3b,     0xdacded4ca39c795b,     0xe719f779c9e14d0a,     0xa27b7d3d24198c39,
    0x28bb00a4d7d0d925,     0x233284419913d906,     0xf1aae77cb11e9f54,     0xdf22fcc213d38e3d,
    0x0b9895762aaf1030,     0x927f0a4fcbe065ce,     0x783c707f0220b475,     0x598e9c7d5f331f97,
    0xec8240d83bf93145,     0xa453c537abc9818b,     0x81282a3397a6abb4,     0x2f4e7a3a31b5ba41,
    0xe90de59f9bad26aa,     0x6b17ead3f772c7d1,     0xd84cfcbacc3e647f,     0xc030ffcd735eab68,
    0x4e40f85ec7b9da7a,     0x2c9234e385f719f1,     0x6b54cb0afe254344,     0x43552fe3233d7969,
    0xcb29924bf9a17b71,     0x57ce4920b951bf6e,     0x6c38cf2f33892f86,     0xfa0b4c05240e2d12,
    0xd8a9d4c877b32256,     0x710a6c1ef9b02988,     0x0902d082fa455be0,     0xd84eed6b76c980ef,
    0xb71863484391faef,     0x151c15d90c2ac831,     0x78f435e1c07771d4,     0x23d5c34d9278171c,
    0x5537c8650678b9e5,     0xf81a28b86fa8537f,     0x0c993eea186fa7db,     0x4d1a38e100c52a31,
    0x634aedfd929db8c7,     0x669a0722408c495e,     0x670bfc7afcd451bc,     0x7b8c71bd2ebee7ca,
    0x60248e42f74a0cd3,     0x215a53f99c105557,     0x8665ad2c18430e64,     0x50220379ee27e40d,
    0x244811a8defa5682,     0x255060f2b4a0a29b,     0x50d9d2fd9b7e1230,     0x2a20652f60dd79b9,
    0x28f94d407aa43e0a,     0x8de6bb567dd90e10,     0xb6c0e912ec50fcff,     0xf3332f53b7f1e529,
    0xe680a7e678e544ef,     0x053685632bc67efd,     0x34705c5e6fa7d7c4,     0xae8dfd98c26e8f71,
    0xac1e6a1e4be12d30,     0xbf11731b7e878db8,     0x947f31e8cc3dd362,     0x4b1d09fcb387e833,
    0xea73ebcc6138baca,     0x5338412d0c8f05ce,     0xd17a20fd732d1bb6,     0x775b85ad51eec317,
    0xb2756b9a9970ad07,     0x31708a92e52f7b4a,     0xbaece788e18bc46d,     0x72158473fbd4df36,
    0x72c7a9cd90c28f6a,     0x3745fcd547f89d64,     0x5fff4416947051c0,     0xc733d2d775939291,
    0xab88e568161feec9,     0x1e8af65f18460ede,     0x8efb5e8b83ea147b,     0x0663a12cb56c534a,
    0x6e5b798be6d58d85,     0x0b4b283397059ab9,     0xaaf6b4986cfccc98,     0x00d7b30092bacf78,
    0xc53b447c73d08d3d,     0x0f1b2115c15f063a,     0x35ae0d237c33d940,     0x4c17c3b8c76fd79f,
    0x2361faa87c991184,     0xd5f011615719139e,     0x89cb488458fdaa51,     0x2e606576bf9e741a,
    0x44016bd55bed574b,     0x0ee9296754b33ecf,     0x96c2d04fe120fb9b,     0x18ce9f27e2882687,
    0x45ead7fb25d4b4d7,     0xbd5dd1fa6b98b9a9,     0x82bc3ee7380db48c,     0x47e9149692be3c57,
    0x282fbd331150200a,     0x496a62c3f70c83ce,     0x39610015c78b19e4,     0x01d0f561e05257dc,
    0x9745fa33a7130bbc,     0x16497397f73b11ed,     0x97c56950ae27725a,     0x9f3a7790f5020aef,
    0xc5aa2f197ddb84a3,     0xca420d9c349e9d5d,     0xc5e8618c7024891b,     0xef7fee2260b1eb03,
    0xb9ad19db92cde23f,     0x2a3294dac6ed8264,     0x7fe2ed53e4aec429,     0x5623a2fb26a535b6,
    0x4b03fe49289d23c5,     0x24c4af6ae17c7c00,     0x12c16e4dc8635f0f,     0x2288a27743f564e5,
    0x3f39ccee4f1ca396,     0xc977e58bfb174457,     0x4c299734f1af5bc6,     0xeb3be3684aa582fd,
    0x8fa9f112b9f0c614,     0x0771bba7fd42cfc7,     0xcef0de4252b480c3,     0x11d8b7216062f8cd,
    0x630d1ae7dd84fe97,     0x988ba6f7e1c0b632,     0x11c7e91ae31410b9,     0x9caa5036a9fd45fc,
    0x373fea4006c166f7,     0x10be2a19bb1e7157,     0xc3e09c0656bf18d7,     0xf617a0cf5f1fdd8b,
    0x9c80272fd77b09e2,     0xb00821c9a42b1612,     0x4f898da8fecb5cf2,     0x2bd27ed897ed54e7,
    0xa91c27753c317194,     0xf1283dda60526ecd,     0x4f8c02b6337a37da,     0xb8a1955e416bc7f1,
    0x1df0fdc42aa2322b,     0x74802c6f9a68f3ae,     0x9d31673f7e5dd893,     0x251d37b4c030549b,
    0x6b1bf11a7fce186e,     0x2305535eb85da307,     0xd8aec3ff749bb7a3,     0x1b897767b1ec6099,
    0x7a0c1adb5e97d681,     0x4a718806557ca55b,     0xdb6597ebe34ef599,     0x797bb00e5954cec7,
    0xec75e9ddd687b706,     0x7972532516d09976,     0x25dfa8267a5d21a6,     0xb2eb00e1b5371ada,
    0x64a6755b59a82574,     0xa39e22dc8433bb59,     0x33a6b020a1a60a2f,     0x8a21658959944ed7,
    0xa8cb8e97679f5aa6,     0x1b43d56f9719855c,     0xb49f5984b3ed9659,     0xee7dcea62fd986b2,
    0x7d6b8b58add22dbf,     0xdf9a8c83a669e6c6,     0xded3e160f197b5c6,     0xcb9a76b20a766586,
    0xfadb7b3a6b59789f,     0x6734e5931e1456c0,     0x5465d226e86b5305,     0xd943a1348afb22f6,
    0x78923149b0cdbea4,     0xe501fbeb6c840d1c,     0xa741f37e51c2e2f4,     0x2510ad083450b49a,
    0x203056b3a7841cfb,     0x15a0b8bd97feeaba,     0x8ddbebd53ada6380,     0x994b83e95194dfaf,
    0x763af65502567338,     0x70facb92d8bd205f,     0x0884122282328512,     0x56386be2bed2f9ea,
    0xb8c9b465b108c4b8,     0x2facc60b302314b5,     0xd8503fdf89c2a312,     0x72f5b139d1ccffd3,
    0x5bd0884100acfdf3,     0x729df743453970aa,     0x3ef29be678f13730,     0xe8014025409815f8,
    0x100a03fd5af3bfbc,     0x11de57ee7473ae8e,     0x6206b4bd2b793366,     0x0fa0803414a907ac,
    0xda499f94374ed9ed,     0xba3c369a0f17d2a4,     0x216951968094aedc,     0x168d4e912c7e3b3b,
    0x230f2e659e506971,     0xd04a822f705ead80,     0x58d889d36da98ae6,     0x5690356e364d4bb4,
    0x82397adfae56883c,     0x041724b4f6145764,     0xc27b105886fe68d4,     0x14c545c29d06ffc8,
    0x6a44c24798952f16,     0x5fe921ffa6e57cf1,     0xb2dd76f7ace1962d,     0x342a347c6311768a,
    0xada2a87630d9b632,     0x4e7968eedfb68ccd,     0x949219c0fb82cf79,     0xd7aa1c0a4d94c652,
    0x78fabc4381f941f8,     0xe64e1bea9e0e6859,     0x8c3b291ac4a5d4ff,     0xa85b3199c0c4ed4f,
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
