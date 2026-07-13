# Workspace verifier image

This image runs the five platform-owned checks for a reviewed source candidate.
It does not run commands proposed by a model and does not need network access at
runtime.

Build and publish the image through the release workflow. Configure the control
plane with the resulting immutable reference, including its digest:

```text
WORKSPACE_VERIFIER_IMAGE=registry.example/practice-studio-verifier@sha256:...
```

Strict-identity or durable control-plane deployments fail closed for owned
source imports when this image is not configured. The deterministic verifier
supports local synthetic treatment iteration, but it is not authoritative for
an externally supplied repository.

The control plane refuses a tag-only image reference. It also supplies the
runtime limits, read-only root, disabled network, dropped capabilities, and
single writable workspace mount. The image writes one bounded JSON report with
the fixed check IDs. Rust validates their order and computes the evidence
digests.

The control plane admits one verifier container at a time by default. Increase
`WORKSPACE_VERIFIER_MAX_CONCURRENCY` only after measuring host memory and CPU.
Excess work fails immediately and can be retried. The control plane does not
queue an unlimited container backlog.
