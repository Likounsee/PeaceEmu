# Performance

No performance benchmark exists yet, and no throughput or latency claim is made.

## Measurement policy

Choose a deterministic workload before optimizing. Record CPU, OS, compiler/toolchain, build profile, input corpus, warm-up, sample count and summary statistic. Separate correctness checks from timing runs. Compare complete emulation workloads as well as targeted hot paths.

Track allocations and memory use where relevant. Prefer compiler-visible, simple abstractions; do not add assembly or a JIT without a measured need, a correctness strategy and a fallback. Every optimization must rerun the relevant conformance and regression tests.
