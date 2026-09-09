# Starter templates

Start with a complete plan and replace its operations with your application's
work. These starters use a shared in-memory backend so they run without a
database, network connection or credentials.

| Pattern | Start from | Keep when adapting |
|---|---|---|
| Two mounts of one child with different resources and inputs | [Composition](../crates/sdax-tokio/tests/guide/starter_composition.rs) | Formal port, separate bindings, resource dependency edges and completed outputs |
| Transient acquisition failure followed by cleanup | [Retry and cleanup](../crates/sdax-tokio/tests/guide/starter_retry_cleanup.rs) | Lazy acquisition, bounded declarative retry, separate captures and full cleanup errors |
| An external operation whose outcome is unknown | [Identified recovery](../crates/sdax-tokio/tests/guide/starter_recovery.rs) | Stable identity, body timeout, recovery policy and explicit compensation |
| A service that restarts while retaining its published handle | [Resident service](../crates/sdax-tokio/tests/guide/starter_service.rs) | Separate initialization and serving, restart policy, cooperative stop and work draining |

Each file contains `build`, a fallible `boundary`, and runnable examples of the
normal and failure paths. The accompanying [backend](../crates/sdax-tokio/tests/guide/starter_support.rs)
defines `Environment`, `Lease` and `OperationError`. Copy it with the chosen
starter to try the plan, or substitute your own backend and imports.
The starter imports the backend as `crate::starter_support`; keep that module
name or update the imports to match your application.

Run the starters from the repository with:

```sh
cargo test -p sdax-tokio --test guide --locked --offline starter_
```

## Adapt the operations

1. Replace the backend calls, resource values and completed output calculations.
   Keep external acquisition inside the lazy factory passed to `cx.hold`.
   The backend's acquisition methods already return futures: return those
   futures directly, or await them inside an async factory.
2. Name resources distinctly where the external system requires distinct
   identities. Separate component scopes do not rename your backend's objects.
3. Adjust retry, restart, timeout and shutdown settings deliberately. Retrying
   an external operation requires an honest idempotence decision for that operation.
4. Keep resource imports visible through ports and bindings. Ordinary input
   carries request data; it is not a substitute for a resource lifetime edge.
5. Retain the full `Report` internally. These examples convert failures to text
   only at the `Result<u32, String>` boundary and return an error for missing
   output. Text cannot retain typed error downcasting.

The in-memory backend records lifecycle events and models failures. Its pending
request represents an operation without a known result; a real integration must
reconcile with its external system before claiming the uncertainty is resolved.
`Recovery::Resolved` clears ambiguity but does not erase the original timeout fault.
Its work counter represents work owned by the service, not a detached task.

For the compact API forms and smaller examples, see the
[AI authoring reference](AI-Authoring.md). For the full authoring guide, see
[Authoring](Authoring.md).
