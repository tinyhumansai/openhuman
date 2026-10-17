# Agent goals

TinyAgents owns goal types, lifecycle, persistence, budgets, and prompt
rendering. This directory contains OpenHuman runtime adapters for explicit
thread selection, tool registration, and turn accounting.

Goals are controlled by the agent tools and runtime. There is no thread-goal
frontend or `openhuman.thread_goals_*` JSON-RPC API.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Goals and todos](../../../../../docs/gitbooks/en/features/goals-and-todos.md)
