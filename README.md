# byteseek

A KV-native agent substrate: an agent's *self* — its code, state, memory, and
execution progress — all live in one addressable, persistent, self-modifiable KV
tree (kvspace, backed by redis). LLM, shell, python, and sub-agents become
first-class citizens of this tree as registered rwir. One process = one
**corebrain**: lay the `.kv` into redis → register rwir → bootstrap a vthread →
drive execution (kvlang mode 2), handling rwir in place.

See `doc/substrate.md` for the architecture and implementation notes.

## Roadmap: three stages

byteseek evolves along one line: the agent progressively takes over the power to
*build its own tools, manage its own resources, and rewrite itself* — from human
hands into this tree.

- **corebrain** — the reasoning core that drives the tree.
- **rwirext** — extension capabilities registered into the tree (`llm` / `shell`
  / `python` / `agent` are the first four).
- **extbrain** — the next-generation reasoning core that corebrain autonomously
  trains and iterates within kvspace, able to replace the current corebrain.

**Stage 1 — corebrain writes its own code, humans assist with extensions (current).**
corebrain generates and executes kv code itself; rwirext extension libraries are
created with human assistance to keep them high-trust; tasks are executed
iteratively in the corebrain loop. The "Verified" list in `doc/substrate.md`
sits at this stage.

**Stage 2 — mature extension libraries, corebrain iterates extbrain autonomously.**
A large body of mature rwirext libraries accumulates; with sufficient
storage/compute, corebrain autonomously trains and infers within the kvspace
architecture, replaces its own corebrain, and then manages, uses, and iterates
extbrain. Humans step back from "assisting with tool-building" to "setting
goals"; the agent begins to improve its own reasoning core.

**Stage 3 — AGI.**
By the common industry definition, AGI is a system that matches or exceeds human
capability across the full breadth of cognitive tasks — rather than a narrow one:
it generalizes to novel tasks, acquires new skills on its own, and transfers
knowledge across domains. byteseek pursues this endpoint through its own
architecture, and each general-intelligence property maps to a concrete mechanism
in the tree:

- **Open-ended skill acquisition** — a new skill is a new piece of rwirext / kv
  code written into the same tree, not a retrained monolith. Capability grows by
  the tree growing.
- **Self-improvement of the reasoning core** — via extbrain, the agent trains,
  verifies, and replaces its own corebrain in-architecture; the improvement loop
  runs on itself, with no fixed ceiling on corebrain / extbrain / rwirext.
- **Cross-domain transfer** — all skills, memory, and history share one
  addressable kvspace, so knowledge learned in one task is directly reachable and
  reusable in another.
- **Tractable oversight** — because the whole self-modification process stays
  inside one addressable, persistent, self-modifiable KV tree (with PC as a
  crash-recoverable KV path), capability growth remains observable, auditable,
  and recoverable rather than an opaque black box. Humans supply only goals,
  values, and constraints.

This is the endpoint the closed loop is aimed at — an open goal, not a solved one.
