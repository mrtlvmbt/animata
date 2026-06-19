use super::*;

#[test]
fn splitmix_is_pure_and_avalanches() {
    // Pure: same input → same output; adjacent inputs → very different outputs.
    assert_eq!(splitmix64(42), splitmix64(42));
    let (a, b) = (splitmix64(1), splitmix64(2));
    assert_ne!(a, b);
    assert!((a ^ b).count_ones() > 10, "adjacent seeds barely differ — weak mix");
}

#[test]
fn seed_fold_is_non_commutative() {
    // The whole point: swapping two fields must change the seed (else id^tick collisions).
    assert_ne!(seed_fold(7, &[3, 5]), seed_fold(7, &[5, 3]));
    assert_eq!(seed_fold(7, &[3, 5]), seed_fold(7, &[3, 5])); // but deterministic
}

#[test]
fn rng_stream_is_deterministic() {
    let mut a = Rng::new(123);
    let mut b = Rng::new(123);
    for _ in 0..100 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
    let mut r = Rng::new(9);
    for _ in 0..1000 {
        let u = r.unit();
        assert!((0.0..1.0).contains(&u));
    }
}
