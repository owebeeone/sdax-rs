# Dynamic readiness profiling and verification

The pre-change measurement fails the original baseline allocation budget: 10 live children allocate 4,479 times against a 4,361 budget. [Red evidence](../../artifacts/dynamic-profile-20260908/allocation-red.log) records the executed assertion failure. Native sampling independently identifies readiness resolution as a hot path.

Implemented keyed readiness lookup with a once-answered flag, keeping notifications outside the registry lock. Added semantic regression guards for ordering, repeated readiness, end-before-ready outcomes, and reentrant wakeups; these are not claims that the old behavior failed. They pass.

The measured runtime target improves, but the original allocation assertion remains red at 4,384 calls. Do not relabel it green: full allocation regression removal remains unfinished. All eleven shared Mac correctness/build/documentation gates pass, Windows fixture verification passes, and native Rust 1.75 builds pass for both libraries.

The initial shared-gate run failed an obsolete automatic-deletion expectation under the owner's retention policy. The consumer checker/test now preserve read-only objects by default; the full inventory then passed. Both gate logs are retained.

See [complete findings](../../artifacts/dynamic-profile-20260908/DynamicReadinessResults.md).
