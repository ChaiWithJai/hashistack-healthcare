# Bounded generative-fill experiment

Date: 2026-07-12

Runtime: loopback-only llama.cpp OpenAI-compatible server.
Model: Bonsai-4B-Q1_0, SHA-256
`4524b3f997f0f06444e568d1f26e2efd69effa3218c7ad3047432fb171e42168`.
Target: an exported copy of the Rust/Axum `post-op-monitor` scaffold.

The experiment copies the reviewed application into a disposable directory,
asks the local model for one schema-constrained pain-escalation threshold,
validates its type, range, and expected instruction meaning, changes exactly
one Rust constant, then runs formatting and all eight scaffold tests. The
original pack is never modified.

## Failure found

For “route pain scores of 6 or higher to the practice inbox,” the first prompt
returned the existing value `7`. Rust still compiled and its tests passed.
The harness rejected the result before editing because compilation is not
proof of instruction adherence.

## Treatment and result

The prompt now separates the current value from the requested value and tells
the model to extract the requested threshold. The same deterministic semantic
validator remains authoritative. Three distinct requests passed end to end:

| Request | Expected/generated | Rust tests |
| --- | ---: | ---: |
| route pain scores of 6 or higher to the practice inbox | 6 | 8/8 |
| escalate when pain reaches 5 | 5 | 8/8 |
| change the escalation threshold to 8 | 8 | 8/8 |

Run one case with:

```sh
MODEL_URL=http://127.0.0.1:8081 \
  scripts/generative-fill-experiment.sh \
  'route pain scores of 6 or higher to the practice inbox' 6
```

This proves a bounded local fill with deterministic validation. It does not
prove general Rust generation, fine-tuning, safe autonomous edits, or that
Bonsai should replace the rules fallback. Broader fills require reviewed edit
schemas, negative cases, source-diff policy, and sandboxed verification.
