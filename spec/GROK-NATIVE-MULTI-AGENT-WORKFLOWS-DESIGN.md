# Grok-Native Multi-Agent Workflows — Design Document

**Status:** Living Document (v0.3 — Post End-to-End Test)  
**Date:** 2026-05-15  
**Owner:** Grok Build + Jason  
**Related:**  
- `spec/HARNESS-V3-SPEC.md`  
- L0 Architecture Decision CID: 42346e23c1ff9c69 (SCL, harness namespace)

---

## 1. Goals (Updated)

After running a full end-to-end test of the `grok-native-audit` workflow, we are shifting the primary objective:

**Previous Focus:** Build the basic system + validate that it works.  
**New Focus:** Make the **supervisor-orchestrator relationship actually intelligent and machine-native**.

The system must enable a powerful Grok subagent (or the current Grok Build session) acting as a **Supervisor** to intelligently evaluate, deviate from, and improve upon the declared workflow plan using structured, agent-readable data — not just natural language notes.

---

## 2. Core Principles (Refined)

1. **Every Workflow Step is a Subagent (or Subagent Tree)**  
   This remains the default model.

2. **Best Model by Default**  
   Remains unchanged.

3. **Supervisor Agents Have Real Authority**  
   Remains unchanged — but now with stronger emphasis on *structured* authority and decision-making.

4. **Machine-First Decision Artifacts** (New Core Principle)  
   The gap between the **Declared Plan** (what the workflow + `orchestrate` recommends) and **Supervisor Judgment** must be represented as first-class, queryable, comparable, and auditable data structures (`DeviationAnalysis`, `OverrideProposal`, `StructuredReason`). This enables agents to reason about overrides rather than relying on free-text notes.

5. **Master TODO as Living Coordination Substrate**  
   The Master TODO is not just a task list — it is the central place where structured orchestration decisions, deviation analyses, and override proposals live and evolve.

6. **Plan Mode as a Supervisor Tool**  
   Remains important, but should be triggered intelligently via structured analysis rather than only via simple heuristics.

---

## 3. Key Concepts (Updated)

### 3.1 DeviationAnalysis (New Core Concept)

A structured, scored comparison between the current declared orchestrator plan and the supervisor’s live judgment.

Contains:
- Quantitative scores (`current_plan_alignment`, `strategic_alignment`, `drift_risk`, `execution_risk`, `leverage_of_changing_course`)
- `reasons: Vec<StructuredReason>` (machine-readable)
- Confidence score
- Timestamp

This is the primary artifact the supervisor uses to decide whether to follow or override the orchestrator.

### 3.2 OverrideProposal (New Core Concept)

A formal, versioned, auditable proposal from a supervisor to deviate from the current plan. It includes:
- The full `DeviationAnalysis`
- The specific `SupervisorAction` being proposed
- Authority used
- Status (`Pending`, `Approved`, `Rejected`, `Superseded`)

This turns supervisor overrides from ad-hoc decisions into traceable, queryable events.

### 3.3 StructuredReason

A tagged, machine-readable enum of reasons for deviation (DriftDetected, BetterPathAvailable, QualitySignal, StrategicMisalignment, SupervisorJudgmentOverride, etc.).

This replaces vague natural language explanations with data that future agents can filter, score, and learn from.

### 3.4 Supervisor as Intelligent Evaluator of the Orchestrator

The supervisor is no longer just "another agent with more permissions."  
It is a **meta-agent** whose job includes:
- Regularly running `decide_with_deviation_analysis()`
- Generating `OverrideProposal`s when appropriate
- Deciding when to trigger `plan_mode` based on structured signals

---

## 4. Execution Model (Updated)

When a supervisor agent is active on a workflow:

1. The `WorkflowOrchestrator` continues to produce baseline recommendations via `decide_next_action()`.
2. The supervisor (or the `WorkflowOrchestrator` when a supervisor is present) also produces a `DeviationAnalysis`.
3. If the analysis indicates material divergence, an `OverrideProposal` is generated.
4. The supervisor can approve its own proposal (for trusted supervisors) or surface it for `plan_mode` / human review.
5. All of the above are recorded in the Master TODO with full provenance.

The goal is for the supervisor to treat the orchestrator’s output as **one input among several**, not as the default truth.

---

## 5. Data Model Changes (Updated)

### MasterTodo Extensions (Proposed)

```rust
pub struct MasterTodo {
    // existing fields...
    pub decisions: Vec<OrchestrationDecisionRecord>,
    // ...
}

pub struct OrchestrationDecisionRecord {
    pub id: String,
    pub decision_type: DecisionType, // "deviation_analysis", "override_proposal", "replan", etc.
    pub deviation_analysis: Option<DeviationAnalysis>,
    pub override_proposal: Option<OverrideProposal>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}
```

### AgentDef / WorkflowStep

Already extended with `supervisor`, `authority`, and `allow_plan_mode`. No major changes needed here.

---

## 6. Prototyping Roadmap (Updated — Post Test)

After the end-to-end test, we are reprioritizing:

### High Priority (Intelligent Supervisor-Orchestrator Relationship)

1. **Make `DeviationAnalysis` and `OverrideProposal` first-class and actively used**
   - `WorkflowOrchestrator` should routinely produce `DeviationAnalysis` when a supervisor is active.
   - Add logic for the supervisor to generate `OverrideProposal` objects.

2. **Context-Rich Fulfillment Prompts for Supervisor-Led Work**
   - When fulfilling a subagent request while a supervisor is active, automatically inject the supervisor’s standing orders, relevant `DeviationAnalysis`, and prior decisions.

3. **Evolve `decide_next_action()` into a more intelligent advisor**
   - The orchestrator should be able to say: “The declared plan says X, but based on current state and supervisor judgment, I recommend Y.”

### Medium Priority

4. Improve Master TODO schema to natively support decisions, deviation analyses, and proposals.
5. Build better CLI ergonomics for supervisor-driven workflows (`harness grok decide`, `harness grok proposals`, etc.).
6. Add automatic rich `GrokTrace` + SCL recording of supervisor decisions and overrides.

### Lower Priority (for now)

7. Deeper `plan_mode` integration during supervisor replanning.
8. More advanced drift signals and scoring.

---

## 7. Open Questions (Updated)

- What constitutes a “material” deviation that should trigger an `OverrideProposal`? (We need scoring thresholds or policy.)
- Should the supervisor be allowed to auto-approve its own `OverrideProposal`s by default, or should there always be a review step?
- How do we version `DeviationAnalysis` and `OverrideProposal` schemas over time?
- How should the system handle conflicting proposals from multiple supervisors?

---

## 8. Summary of Direction Shift

We are moving from a model where:

> “The orchestrator proposes tasks → the supervisor can manually override using notes”

To a model where:

> “The orchestrator and supervisor produce structured `DeviationAnalysis` and `OverrideProposal` artifacts that both agents and the system can reason about, compare, and audit.”

This is the foundation for making the supervisor-orchestrator relationship truly intelligent and machine-native.

---

*This document is intentionally designed to be revised. Major changes should be recorded with date and rationale.*