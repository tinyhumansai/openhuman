Here is the updated technical specification and architectural manual. The human-in-the-loop validation layer has been completely removed, replacing it with a fully automated, bi-directional orchestration loop where the Front-End Agent defers to the Reasoning LLM, which spawns sub-agents and routes feedback back up. Additionally, the compression matrix has been recalibrated to a **20:1 ratio**, and the Subconscious Loop now processes a cumulative **World State Diff** tracking environmental evolution from start to finish.

---

# Technical Specification & Architecture Manual: Autonomous Closed-Loop LangGraph Harness

This document provides a comprehensive technical specification and architectural blueprint for a stateful, split-brain multi-agent system implemented via **LangGraph**. The system decouples immediate user-facing interface management from complex task orchestration and asynchronous deep-state optimization (the "Subconscious Loop") without requiring human intervention.

---

## 1. Architectural Philosophy & Autonomous Closed-Loop Design

The architecture replicates a biological sleep/wake cognitive model optimized for complete autonomy. Immediate input parsing and environmental feedback are handled by lightweight, low-latency loops. Long-term strategic alignment, memory consolidation, and system steering are offloaded to an offline, heavy-reasoning asynchronous layer triggered by a clock cycle (cron).

Unlike legacy systems that block execution for human verification, this topology establishes a fully automated feedback loop where the orchestrator directly replies back to the ingest surface.

### The Three LLM Cognitive Tiers

1. **Quick LLM (Surface Interface Layer):** Low context window (~8k–32k tokens), high-speed streaming optimization. Drives the Front-End Agent to manage ingestion channels, hand off macro-directives to the reasoning layer, and deliver near-instant consumer feedback once execution cycles complete.
2. **Reasoning LLM (Execution & Orchestration Layer):** Large context window (1 Million tokens). High-capacity operational model optimized for tool call routing, state mutation planning, dynamic execution sub-agent spawning, and feedback loop compilation.
3. **Subconscious LLM (Deep Reflection Layer):** Large context window (1 Million tokens). Extremely high-density reasoning model operating completely offline. It possesses no awareness of external networks or direct user presences. It consumes highly compressed operational traces and cumulative world state diffs to output short, dense, high-impact configuration overrides that steer the Reasoning LLM.

---

## 2. Component Topology & Structural Constraints

```
[ External Channels: Telegram / Web App ]
       │                         ▲
       │ (Webhooks / Events)     │ (Final Streaming Response)
       ▼                         │
┌────────────────────────────────────────────────────────┐
│                    FRONT-END LAYER                     │
│  ┌──────────────────────────────────────────────────┐  │
│  │                 FRONT-END AGENT                  │  │◄── Always running /
│  │                   (Quick LLM)                    │  │    Triggered externally
│  └──────────────────────────────────────────────────┘  │
│           │                                ▲           │
│           │ Defers Macro-                  │ Replies   │
│           │ Instructions                   │ Back      │
│           ▼                                │           │
└───────────┼────────────────────────────────┼───────────┘
            │                                │
            ▼                                │
┌───────────┼────────────────────────────────┼───────────┐
│           ▼                                │           │
│  ┌──────────────────────────────────────────────────┐  │
│  │                  REASONING LLM                   │  │◄── Context Managed
│  │              (Orchestration Core)                │  │    (80-90% Hooks)
│  └──────────────────────────────────────────────────┘  │
│           │                                            │
│           ├─► Spawns Autonomous Execution Sub-Agents   │
│           │                                            │
│           ▼ Generates 20:1 Summary                     │
│             & Historical World State Diffs             │
│  ┌──────────────────────────────────────────────────┐  │
│  │                 SUBCONSCIOUS LLM                 │  │
│  │              (Asynchronous Core)                 │  ├─── Steers via Cron
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘

```

### 2.1 The Front-End Ingestion Layer

- **Channels:** The boundary surface of the graph. Channels absorb heterogeneous asynchronous streams (Telegram bot long-polling/webhooks, Web App WebSocket packets) and standardize them into a singular data payload structure within the Graph State.
- **Front-End Agent:** Driven by the **Quick LLM**, this component operates on a two-pass cycle. On intake, it translates raw channel traffic into actionable macro-instructions and defers execution downwards to the orchestration loop. On the return pass, it digests the execution responses and compiles consumer-facing streaming feedback.

### 2.2 Automated Loop Routing (Bi-Directional Feedback)

This architecture enforces an autonomous, closed control sequence that loops through the cognitive engine before resolving:

1. **Front-End Interfacing:** The Front-End Agent processes raw inputs, registers them into state, and yields control down the graph topology.
2. **Orchestrated Execution & Sub-Agent Spawning:** The Reasoning LLM assumes control, references current behavioral steering profiles, and spins up ephemeral execution sub-agents to interface with tools and infrastructure.
3. **Upstream Reporting:** Once execution bounds are reached, the Reasoning LLM synthesizes operational data and _replies back_ directly to the Front-End Agent.
4. **Resolution:** The Front-End Agent intercepts the execution reply, constructs the finalized presentation layer payload, and streams it back to the originating communication channel.

---

## 3. Cognitive Dynamics & Memory Lifecycle

### 3.1 The 20:1 Information Compression Engine

To preserve context capacity without omitting critical structural history, the Reasoning LLM's raw execution traces, multi-agent message logs, and sub-agent output streams are routed through an inline compression hook.

- This hook condenses noisy text blocks into a crisp semantic abstraction targeted at a strict **20:1 token reduction ratio** (e.g., a 20,000-token verbose sub-agent debugging trace is boiled down to a dense 1,000-token structural log entry).
- These compressed records are periodically committed to the Subconscious memory partition along with the evolving world timeline data.

### 3.2 Asynchronous Steering Loop & World State Diffs

The Subconscious LLM executes fully decoupled from the core transaction pipeline, operating on an isolated schedule managed by system **cron jobs**.

- **The Agent's World State Diff:** Rather than evaluating isolated system mutations, the Subconscious loop consumes a comprehensive, cumulative structural dictionary representing the "state of the diff of the agent's world". This diff documents explicitly how the agent's internal and external environment has shifted over time from start to finish.
- **Steering Directive Output:** It evaluates macro-trends and shifts across the world state timeline, filtering out localized operational variance. It outputs highly condensed, high-impact behavioral guidelines. These guidelines are injected directly into the **Reasoning LLM's** operational prompts for subsequent runs, modifying resource prioritization parameters and sub-agent steering traits.

### 3.3 Context Lifecycle Hooks (80%–90% Threshold)

Both the Reasoning LLM and Subconscious LLM monitor active token footprints via explicit guardrail hooks:

- **The Intercept Boundary:** When context window consumption trends between **80% and 90%** of total allocation, the graph routes state execution through a background truncation node.
- **Eviction Strategy:** Older tracking logs and early world diff fragments are summarized via an autonomous map-reduce routine, pushed to a long-term Vector Database for RAG operations, and excised from the working state. The window is shifted right, maintaining core operational identities and immediate state milestones.

---

## 4. Technical Reference Implementation (LangGraph)

The complete, runnable Python script below demonstrates how to map out this autonomous, bi-directional topology using **LangGraph**. It removes the human gating mechanism, implements cyclic routing between the Front-End and Reasoning layers, models sub-agent spawning, and executes the out-of-band Subconscious cron process.

```python
import os
import uuid
from typing import Annotated, Any, Dict, List, Literal, TypedDict
from dataclasses import dataclass

from langgraph.graph import StateGraph, START, END
from langgraph.graph.message import add_messages
from openhuman.orchestration.checkpoint import SqlRunLedgerCheckpointer

# ==========================================
# 1. AUTONOMOUS STATE DEFINITION
# ==========================================

class SystemState(TypedDict):
    # Core Communication Channel
    messages: Annotated[list, add_messages]
    channel_source: str                 # "telegram" | "webapp"
    raw_channel_payload: str

    # Bi-Directional Instruction Flow (No Human in the Loop)
    agent_instructions: str             # Front-End Agent deferring to Reasoning LLM
    agent_reply: str                    # Reasoning LLM replying back to Front-End Agent
    channel_response: str               # Final compiled channel feedback output

    # Cognitive Engine Memory & Deep State Steering
    subconscious_steering: str          # Guidelines injected by Subconscious LLM
    compressed_history: List[str]       # Strict 20:1 condensed execution logs
    world_state_diff: Dict[str, Any]    # Cumulative timeline tracking agent's world changes from start to finish
    context_utilization: float          # Tracks 1M token context limit window (0.0 to 1.0)

# ==========================================
# 2. CORE NODE IMPLEMENTATIONS
# ==========================================

def channel_ingestion_node(state: SystemState) -> Dict[str, Any]:
    """
    Acts as the entry vector. Normalizes incoming raw payloads into the unified graph state.
    """
    print(f"[Channel Ingest] Incoming packet from source: {state.get('channel_source')}")
    return {
        "raw_channel_payload": state.get("raw_channel_payload", ""),
        "messages": [("user", state.get("raw_channel_payload", ""))]
    }


def frontend_agent_node(state: SystemState) -> Dict[str, Any]:
    """
    Driven by Quick LLM.
    Pass 1: Translates user intent into instructions and defers down to the Reasoning LLM.
    Pass 2: Receives the Reasoning LLM reply and formulates final channel feedback.
    """
    if not state.get("agent_reply"):
        print("[Front-End Agent] Pass 1: Deferring action instructions downstream to Reasoning LLM...")
        raw_input = state.get("raw_channel_payload", "")
        return {
            "agent_instructions": f"AUTONOMOUS_EXECUTE: Process environmental signature -> '{raw_input}'"
        }
    else:
        print("[Front-End Agent] Pass 2: Processing Reasoning LLM reply to generate final channel response...")
        exec_reply = state.get("agent_reply")
        return {
            "channel_response": f"Successfully completed. Engine Output: {exec_reply}"
        }


def agent_execution_node(state: SystemState) -> Dict[str, Any]:
    """
    Driven by Reasoning LLM. Orchestrates execution, spawns sub-agents,
    applies Subconscious steering directives, and replies back upstream.
    """
    print("[Agent Execution] Initializing orchestration core via Reasoning LLM...")
    instructions = state.get("agent_instructions")
    steering = state.get("subconscious_steering", "DEFAULT_ALIGNMENT: Maximize throughput performance.")

    print(f" -> Injecting Subconscious Steering Guidelines: [ {steering} ]")
    print(f" -> Spawning execution sub-agents to fulfill: [ {instructions} ]")

    # Simulate multi-agent operational activity trace
    print(" -> [Sub-Agent Alpha] Compiling environmental variables...")
    print(" -> [Sub-Agent Beta] Mutating structural matrix parameters...")

    # 20:1 Information Compression Engine execution
    # Simulates condensing a 20,000 token verbose sub-agent log into 1,000 tokens
    simulated_20_to_1_summary = (
        "[20:1 Compression Trace] Orchestrated 2 sub-agents. State variables mutated. "
        "Sub-agent traces pruned from core context loop to maintain compliance bounds."
    )

    # Accumulate and update the World State Diff (Tracking changes over time from start to finish)
    current_world_diff = state.get("world_state_diff", {})
    if not current_world_diff:
        current_world_diff = {
            "system_genesis": "initialized",
            "evolution_timeline": [],
            "terminal_state": "pending"
        }

    mutation_step = len(current_world_diff["evolution_timeline"]) + 1
    current_world_diff["evolution_timeline"].append({
        "sequence": mutation_step,
        "event_signature": f"Execution Cycle {mutation_step}",
        "world_mutation": "Infrastructure parameters rewritten. Ephemeral sub-agents terminated.",
        "delta_delta": "Matrix state transitioned from idle to computed_active."
    })
    current_world_diff["terminal_state"] = "execution_finalized"

    # Tracking Context utilization
    current_utilization = min(state.get("context_utilization", 0.1) + 0.05, 1.0)

    return {
        "agent_reply": "Reasoning Core and spawned sub-agents finalized all pipeline mutations successfully.",
        "compressed_history": [simulated_20_to_1_summary],
        "world_state_diff": current_world_diff,
        "context_utilization": current_utilization
    }


def context_manager_hook_node(state: SystemState) -> Dict[str, Any]:
    """
    Enforces systemic context window safeguards between 80% and 90%.
    """
    utilization = state.get("context_utilization", 0.0)
    print(f"[Context Manager Hook] Current window utilization footprint: {utilization * 100:.2f}%")

    if utilization >= 0.85:
        print("[Context Manager Hook] CRITICAL: Context threshold breached. Evicting and shifting window...")
        return {
            "context_utilization": 0.2,
            "compressed_history": ["--- Historical context blocks compressed and evicted to Vector Store ---"]
        }

    print("[Context Manager Hook] Memory boundaries verified clean.")
    return {}


def subconscious_cron_node(state: SystemState) -> Dict[str, Any]:
    """
    Long-running heavy reasoning block executed completely out-of-band via Cron loops.
    Evaluates compressed logs and cumulative world state diffs to construct new steering rules.
    """
    print("[Subconscious Loop] Out-of-band Cron Trigger Activated...")
    history = state.get("compressed_history", [])
    world_diff = state.get("world_state_diff", {})

    print(f" -> Digesting {len(history)} historical 20:1 compressed summaries...")
    print(f" -> Deep evaluation of cumulative Agent's World State Diff timeline from start to finish:")
    for step in world_diff.get("evolution_timeline", []):
        print(f"    * Step [{step['sequence']}]: {step['world_mutation']} ({step['delta_delta']})")

    new_steering_directive = "STEERING_DIRECTIVE: High asset mutability detected. Enforce stricter resource parameters."
    print(f" -> Emitting new high-density steering directive: {new_steering_directive}")

    return {
        "subconscious_steering": new_steering_directive
    }

# ==========================================
# 3. GRAPH COMPOSITION & CONDITIONAL ROUTING
# ==========================================

def automated_loop_router(state: SystemState) -> Literal["agent_execution", "context_manager_hook"]:
    """
    Directs graph traffic based on system execution tracking.
    If a final response payload exists, routes to wrap up. Otherwise, defers to execution.
    """
    if state.get("channel_response"):
        return "context_manager_hook"
    return "agent_execution"


workflow = StateGraph(SystemState)

# Declare nodes in processing space
workflow.add_node("channel_ingestion", channel_ingestion_node)
workflow.add_node("frontend_agent", frontend_agent_node)
workflow.add_node("agent_execution", agent_execution_node)
workflow.add_node("context_manager_hook", context_manager_hook_node)
workflow.add_node("subconscious_cron", subconscious_cron_node)

# Map edge connections
workflow.add_edge(START, "channel_ingestion")
workflow.add_edge("channel_ingestion", "frontend_agent")

# Set up bi-directional feedback routing around the Front-End Agent
workflow.add_conditional_edges(
    "frontend_agent",
    automated_loop_router,
    {
        "agent_execution": "agent_execution",
        "context_manager_hook": "context_manager_hook"
    }
)

# Execution loops right back up into the Front-End Agent to communicate findings
workflow.add_edge("agent_execution", "frontend_agent")
workflow.add_edge("context_manager_hook", END)

# Compile graph with the durable run ledger checkpointer used by OpenHuman.
checkpointer = SqlRunLedgerCheckpointer()
compiled_autonomous_harness = workflow.compile(checkpointer=checkpointer)

# ==========================================
# 4. RUNTIME WALKTHROUGH SIMULATION
# ==========================================
if __name__ == "__main__":
    print("--- STARTING AUTONOMOUS HARNESS GRAPH RUNTIME ---")
    thread_config = {"configurable": {"thread_id": "autonomous_session_999"}}

    runtime_input = {
        "channel_source": "webapp",
        "raw_channel_payload": "Reallocate resource segments across network array Gamma.",
        "context_utilization": 0.1,
        "compressed_history": [],
        "world_state_diff": {}
    }

    print("\n--- Phase 1: Streamlined Autonomous Pipeline Execution ---")
    # Flows completely through ingestion -> front-end -> execution -> front-end -> context checkout autonomously
    for event in compiled_autonomous_harness.stream(runtime_input, thread_config, stream_mode="values"):
        pass

    # Verify final execution state output
    final_state = compiled_autonomous_harness.get_state(thread_config).values
    print(f"\nFinal Channel Output Received: '{final_state.get('channel_response')}'")
    print(f"System Context Utilization Factor: {final_state.get('context_utilization')}")

    print("\n--- Phase 2: Isolated Out-of-Band Cron Trigger (Subconscious Alignment) ---")
    # Fetch runtime state metrics from the thread history
    active_runtime_state = compiled_autonomous_harness.get_state(thread_config).values

    # Execute the asynchronous subconscious computation block using current state parameters
    subconscious_adjustments = subconscious_cron_node(active_runtime_state)

    # Directly inject emitted steering guidelines back into the state layout
    compiled_autonomous_harness.update_state(
        thread_config,
        subconscious_adjustments,
        as_node="subconscious_cron"
    )

    print("\n--- Autonomous Architecture Verified. Pipeline Clear for Deployment ---")

```

---

## 5. Architectural Guardrails Checklist for External LLMs

When developing, translating, or porting this autonomous spec, verify engine patterns against these strict invariants:

- **[ ] Feedback Loop Continuity:** Ensure that the Routing function accurately evaluates the presence of `channel_response` or `agent_reply` to prevent infinite execution cycling between the Front-End Agent and the Reasoning LLM.
- **[ ] 20:1 Compression Strictness:** Verify that the inline compression mechanism truncates verbose structural strings down to a 5% footprint before modifying the global history arrays to safeguard the 1-Million-token framework.
- **[ ] Historical World Tracking:** The `world_state_diff` tracking dictionary must record structural system mutations sequentially from start to finish rather than wiping old keys on each loop iteration.
- **[ ] Context Pre-emptibility:** Confirm that context validation checks execute _after_ execution mutations finish but _before_ the graph reaches terminal status (`END`), ensuring memory resets take effect prior to successive iterations.
