#!/usr/bin/env python3
"""MAX forward-pass probe (time-boxed, 2026-10-08).

Question: can MAX 26.5's own Qwen2 pipeline produce all-token logits
for scoring (the forward pass this kernel track deliberately does not
own)?

Findings (all verified by running this probe in the crate's pixi env):

1. The capability exists in the library: `Qwen2Model` is registered,
   takes a `return_logits` constructor argument, `ReturnLogits` has an
   `ALL` member, and `Qwen2Model` inherits `LogProbabilitiesMixin`
   (`compute_log_probabilities`) — the machinery behind MAX's
   OpenAI-compatible logprobs serving.
2. Importing the model class from the conda `max` package required six
   PyPI packages the conda env does not declare: requests, pydantic,
   msgspec, pillow, av, llguidance (the pipelines package eagerly
   imports its whole serving/multimodal surface).
3. Assembly is a factory's job, not a constructor call:
   `TextGenerationPipeline` requires a pre-built pipeline_model,
   weight_adapters, tokenizer, and memory_plan; `PipelineConfig` is a
   msgspec struct. The factory and CLI entrypoints (`max serve`,
   `max.entrypoints`) ship with the pip `modular` distribution, which
   is not what the conda channel installs — `max.entrypoints` does not
   exist in this env.
4. A hand-rolled direct `Qwen2Model` construction additionally needs
   the KV-cache manager and weights plumbing the executor owns.

Conclusion: end-to-end MAX forward scoring was NOT landed in the
probe's time box. The wall is packaging/factory surface, not
architecture support or math. The production-shaped paths are
(a) adopt the pip `modular` distribution and drive the serving stack
(OpenAI logprobs API), or (b) invest in the executor-level assembly.
Neither is needed for this track's gates: the forward pass stays with
the PyTorch sidecar, and the Mojo kernels own the reduction.
"""

import inspect


def main() -> int:
    from max.nn import ReturnLogits

    print("ReturnLogits:", list(ReturnLogits))
    from max.pipelines.architectures.qwen2.model import Qwen2Model

    print("Qwen2Model params:", list(inspect.signature(Qwen2Model.__init__).parameters))
    print("has compute_log_probabilities:", hasattr(Qwen2Model, "compute_log_probabilities"))
    try:
        import max.entrypoints  # noqa: F401

        print("max.entrypoints: present")
    except ModuleNotFoundError:
        print("max.entrypoints: ABSENT in the conda package")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
