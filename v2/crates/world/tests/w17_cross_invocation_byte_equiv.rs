//! W-17 G1 cross-invocation byte-equivalence test.
//! Invokes the w17_byte_equiv example as separate processes with different
//! RAYON_NUM_THREADS settings and verifies all outputs are byte-identical.

#[test]
fn w17_byte_equiv_cross_invocation() {
    use std::process::{Command, Stdio};

    // Use CARGO_BIN_EXE_w17_byte_equiv macro to get the built binary path.
    // This is resolved by cargo at compile time and is the canonical robust way
    // to reference a [[bin]] target from an integration test.
    let binary_path = env!("CARGO_BIN_EXE_w17_byte_equiv");

    // Note: cargo automatically builds bins needed by integration tests,
    // so no explicit build step is required.

    let mut checksums = Vec::new();

    // Run with RAYON_NUM_THREADS=1
    println!("Running with RAYON_NUM_THREADS=1...");
    let output1 = Command::new(binary_path)
        .env("RAYON_NUM_THREADS", "1")
        .stdout(Stdio::piped())
        .output()
        .expect("Failed to run w17_byte_equiv");
    assert!(output1.status.success(), "Binary failed with RAYON_NUM_THREADS=1:\n{}", String::from_utf8_lossy(&output1.stderr));
    checksums.push((1, String::from_utf8_lossy(&output1.stdout).to_string()));

    // Run with RAYON_NUM_THREADS=2
    println!("Running with RAYON_NUM_THREADS=2...");
    let output2 = Command::new(binary_path)
        .env("RAYON_NUM_THREADS", "2")
        .stdout(Stdio::piped())
        .output()
        .expect("Failed to run w17_byte_equiv");
    assert!(output2.status.success(), "Binary failed with RAYON_NUM_THREADS=2:\n{}", String::from_utf8_lossy(&output2.stderr));
    checksums.push((2, String::from_utf8_lossy(&output2.stdout).to_string()));

    // Run with default (unset RAYON_NUM_THREADS)
    println!("Running with default thread pool...");
    let mut cmd = Command::new(binary_path);
    cmd.env_remove("RAYON_NUM_THREADS");
    let output3 = cmd
        .stdout(Stdio::piped())
        .output()
        .expect("Failed to run w17_byte_equiv");
    assert!(output3.status.success(), "Binary failed with default threads:\n{}", String::from_utf8_lossy(&output3.stderr));
    checksums.push((0, String::from_utf8_lossy(&output3.stdout).to_string()));

    // Extract and compare checksums from outputs
    fn extract_checksums(output: &str) -> (u64, u64) {
        let all_on = output.lines()
            .find(|line| line.contains("ALL-ON:"))
            .and_then(|line| line.split("0x").nth(1))
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .expect("Failed to parse ALL-ON checksum");
        let default = output.lines()
            .find(|line| line.contains("DEFAULT:"))
            .and_then(|line| line.split("0x").nth(1))
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .expect("Failed to parse DEFAULT checksum");
        (all_on, default)
    }

    let (all_on_1, default_1) = extract_checksums(&checksums[0].1);
    let (all_on_2, default_2) = extract_checksums(&checksums[1].1);
    let (all_on_3, default_3) = extract_checksums(&checksums[2].1);

    println!("\nResults:");
    println!("  RAYON_NUM_THREADS=1:  ALL-ON=0x{:016x}, DEFAULT=0x{:016x}", all_on_1, default_1);
    println!("  RAYON_NUM_THREADS=2:  ALL-ON=0x{:016x}, DEFAULT=0x{:016x}", all_on_2, default_2);
    println!("  Default thread pool:  ALL-ON=0x{:016x}, DEFAULT=0x{:016x}", all_on_3, default_3);

    assert_eq!(all_on_1, all_on_2, "ALL-ON checksums differ between thread configs");
    assert_eq!(all_on_2, all_on_3, "ALL-ON checksums differ between thread configs");
    assert_eq!(default_1, default_2, "DEFAULT checksums differ between thread configs");
    assert_eq!(default_2, default_3, "DEFAULT checksums differ between thread configs");

    println!("\n✓ G1 test PASSED: all invocations byte-identical");
}
