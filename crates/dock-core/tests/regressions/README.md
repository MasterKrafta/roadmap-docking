# Proptest regression seeds

The property test in command_sequences.rs writes a minimized failing command-sequence seed to command_sequences.txt when a counterexample is found. Keep any generated seed file in version control after fixing the bug; proptest replays persisted seeds on later runs.

Named, hand-written regressions belong in the Rust test file and run with both the default configuration and the optional serde feature.
