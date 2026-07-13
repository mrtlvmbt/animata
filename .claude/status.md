task: #436 R-15a retained GPU buffers for v2 terrain
phase: testing (local app launch gate blocks screenshot/bench, but code is complete)
blocked_on: Local cargo run blocked by sim gate (render crate properly excluded, hook is overly cautious). Code ready for screenshot/bench acceptance tests.
next: PM runs: --screenshot dim=512 retained OFF/ON at iso-default/iso-zoom-close; --bench dim=64/512 both modes; post results to PR
updated: 2026-07-13 14:35
