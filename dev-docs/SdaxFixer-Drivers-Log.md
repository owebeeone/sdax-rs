# F1 driver publication acknowledgements

Scope: Tokio driver, simulator effect processing, the lifecycle-only scripted
body source, and the blocking-cleanup delegating source. No dependency changes.

## RED

Added `publications_are_drained_before_the_next_scheduled_event` in the
simulator module. With two independent joins, the existing implementation
recorded zero publication acknowledgements instead of two. Executed:
`cargo test -p sdax --lib publication_tests --offline`; the assertion failed
with `left: 0, right: 2`. Initial test-authoring compile errors in shutdown/mode
names were corrected before this behavioral RED.

Strengthened the test to include a third join depending on the first: all three
acknowledgements must complete inside one external simulator step, in FIFO order
(first, second, third), including publication produced by an acknowledgement.

## Implementation

Both drivers finish and record each complete effects batch before feeding any
publication result back to the machine. Each publication produces a result in a
separate immediate FIFO; acknowledgements may append further acknowledgements.
The Tokio source call resolves the run key through the driver's origin table.
Control snapshots, spawn gates and child readiness latches are updated only once
the FIFO is empty. The simulator releases parked child-ready awaits only after
its immediate FIFO has drained; its regular clock queue cannot overtake it.

`ScriptedBodies` explicitly acknowledges known join/component declarations as a
lifecycle-only source: it stores no typed values and exports no fabricated value.
Unknown or non-structural keys fail. `BlockingCleanup` delegates publication to
its wrapped source.

## Verification

Protocol integration initially cannot compile because `PublishReady` and
`ReadyPublished` are being added by the engine owner. The shared protocol and engine implementation subsequently landed locally.

GREEN: `cargo test -p sdax --lib publication_tests --offline` passed the FIFO
regression. `cargo test -p sdax-tokio --test fixer_publication_driver --locked
--offline` passed two new failure tests. The first asserts two publications and
an independent pending body occur in the same begin batch, rejects the first
publication, and requires clean termination, zero tracked tasks, and no rejected
engine events. The second rejects a join after its resource is held, verifies no
consumer constructor runs, and verifies exactly one resource release.

Negative control: temporarily replaced FIFO insertion in the Tokio driver with
immediate `self.step(ReadyPublished { .. })`. The first new failure test failed
with `publication failure must not strand a spawned task: Elapsed(())` and exit
101 under paused time. Restored the correct driver in a `finally` block and
reran both tests GREEN. These tests were added after the protocol implementation;
this mutation is negative-control evidence rather than historical baseline RED.

`cargo test -p sdax-tokio --test conformance --locked --offline`: 81 passed,
including scripted-driver comparisons and multi-thread runtime tests. Full
workspace checks remain the coordinator's integration responsibility.

Added a third host-fault fixture: the custom source returns a successful child
body future without storing its declared export, then delegates actual component
publication to the typed bridge. Both `u32` and an explicitly exported `()` must
fail publication, retain one fault, never construct the consumer, and leave no
engine rejections or tracked tasks. Final targeted driver result: 3 passed.
This verifies the real missing-slot error path, including rejection of a unit
fallback for a declared export.
