---
icon: brain
---

# How It Works

OpenHuman is built on three technology pillars: a memory engine called Neocortex, a subconscious system inspired by human neuroscience, and a privacy-preserving architecture that keeps raw data on your device. Understanding how these work together is understanding how OpenHuman thinks.

### The problem OpenHuman solves

Current AI systems fall into the category of Artificial Narrow Intelligence (ANI). They perform well within bounded domains but they depend on carefully designed architectures and human-defined operational boundaries. They lack three things needed to get closer to general intelligence: persistent memory at scale, a form of consciousness or instinct, and the ability to ingest data without becoming inaccurate or expensive.

Research from Google's Titans project demonstrates the core issue clearly. As LLMs try to absorb more data, they become less accurate. Larger context windows do not solve this. They make the models slower, more expensive, and more prone to errors. This means the path to superhuman AI requires solving for context that is high accuracy, high speed, and low cost.

<figure><img src="../.gitbook/assets/V04_The_ANI_Problem@2x.png" alt=""><figcaption></figcaption></figure>

OpenHuman solves this in three steps.

#### **Step 1: A local-first desktop app**

Before anything reaches the cloud, OpenHuman runs AI models directly on your machine. The desktop app uses Gemma 3 for chat, vision analysis, speech-to-text, and text-to-speech, all running locally on your device's hardware.

Two capabilities define the desktop experience:

**Screen Intelligence** captures screenshots of your screen approximately every 5 seconds and processes them locally using the on-device vision model. Each capture is summarized into structured context: what application you were using, what content was visible, what actions you were taking. Raw screenshots are not stored long-term or sent to any server. You control which apps are included through per-app permissions, so you can exclude sensitive applications like banking or medical tools.

**Auto-complete** uses your accumulated memory context combined with the local model to generate relevant text completions on any input surface across your system. Because it draws on your Neocortex memory, the suggestions reflect your actual work context, terminology, and patterns rather than generic predictions.

Both features run entirely on-device. No raw screen data or keystroke data touches any server.

#### Step 2: Neocortex, the memory engine

Neocortex is a human-like AI memory system that can accurately work with over 1 billion tokens. It is the foundation everything else is built on.

<figure><img src="../.gitbook/assets/V05 — Performance Benchmarks@2x (1).png" alt=""><figcaption></figcaption></figure>

**Speed.** Neocortex indexes 10 million tokens in under 10 seconds. That is roughly 1,000x faster than other memory solutions. Everything that happens in your digital life can be processed in near real-time.

**Accuracy.** Neocortex goes far beyond vector databases. Vector databases retrieve whatever is semantically similar, but similarity alone does not indicate importance. Neocortex understands time and entities, ranking memories by recency, interaction patterns, and contextual relevance. It scores extremely high on RAG benchmarks (open-sourced on GitHub).

**Cost.** Neocortex does not use any LLMs to manage its intelligence. It runs on the CPU of a MacBook Air and costs $1 to index 5 million tokens. That is roughly 10x cheaper than other solutions. If an AI system is going to consume massive amounts of data, the economics have to work. They do.

**Human-like recall for consciousness.** This is the critical piece. Neocortex recalls memories and ranks them based on time, interactions, and randomness. This recall capability is what makes the subconscious system possible.

**Neocortex uses a tiered memory architecture:**

**HOT** memory holds the most recent and actively relevant context. **WARM** memory contains important information that has cooled slightly in relevance. **COOL** memory stores context that is less immediately useful but still retrievable. **COLD** memory holds the oldest and least-accessed information, kept but deprioritized.

<figure><img src="../.gitbook/assets/V11 — Tiered Memory Deep Dive@2x.png" alt=""><figcaption></figcaption></figure>

This mirrors how the human brain works. A research paper from Carnegie Mellon University puts it concisely: intentionally forgetting some things helps us remember others by freeing up working memory resources. Further research published in Nature Reviews Neuroscience (Ryan & Frankland, 2022) confirms that forgetting is an active process, not passive decay. The brain has dedicated molecular machinery for it. Memories become temporarily inaccessible rather than erased. Environmental cues can reactivate them.

This is exactly what Neocortex does. Memories move from HOT to COLD tiers, but they are never deleted. Context cues bring them back. AI systems that try to remember everything, the vector database dump approach, are architecturally wrong. The brain does not work that way.

Additional capabilities of Neocortex:

**Semantic deduplication** strips redundant noise. In group chats, 80%+ of messages are repetitive. Neocortex removes this before it ever reaches the knowledge graph.

**Cross-source entity resolution** means a mention of "the Q4 deck" in Slack, a Notion page, and an email thread are understood as the same artifact. One entity, not three disconnected references.

**Temporal context weighting** uses time-decay functions so recent events carry more weight than older ones, matching how human attention naturally works.

#### Step 3: The personalized subconscious

With good memory and good recall, OpenHuman can do something no other AI agent does: maintain a personalized subconscious.

In the human brain, there is a specialized neuron called the Purkinje cell. It is mainly responsible for random thoughts and plays a significant role in consciousness. The human brain has both a conscious and subconscious mind that work together to build intelligence. Most agentic systems follow the conscious mind model only. They wait for instructions and execute them. They cannot think on their own.

<figure><img src="../.gitbook/assets/V07_Purkinje_Cell_to_Subconscious@2x.png" alt=""><figcaption></figcaption></figure>

Inspired by the Purkinje cell, OpenHuman uses Neocortex to periodically trigger global memory recalls. These recalls are fed into a subconscious loop that produces actions, confirmations, or new connections. Memory recalls are cheap, fast, and happen over 10,000 times a day for less than $1.

The result is an AI that does not just respond to your prompts. It thinks about your data in the background. It makes connections you did not ask for. It surfaces patterns and risks proactively.

The success metric for the subconscious is what the team calls the "mirror test": when you read the subconscious output, it should feel like looking at yourself in a mirror. Not a generic AI output. A reflection of your own thinking, priorities, and context.

### How the pieces connect

When you use OpenHuman, here is what happens:

**Your desktop app runs locally.** Screen Intelligence captures your screen context. Auto-complete assists your typing. Local models handle chat, vision, and voice. All of this runs on your device with no server dependency.

**Connect your data sources.** Telegram, Slack, Gmail, Notion, blockchain wallets, and more. Each connection expands your knowledge graph.

**Neocortex compresses.** Millions of tokens of organizational history, including both your screen activity summaries and your messaging data, become structured intelligence: entities, relationships, timelines, sentiment. Noise is stripped. Signal is preserved.

**The subconscious runs.** Thousands of recall loops per day surface proactive insights, track evolving patterns, and update the knowledge graph.

**You interact naturally.** Ask anything about your life, your team, your projects. Not general knowledge. Your knowledge. What did your team decide? What is the status? What did you miss? What was on that dashboard you were looking at this morning? OpenHuman already knows.

**Privacy is maintained.** Raw data stays on device, encrypted with AES-256-GCM. Encryption keys never leave the device. Screen captures are processed locally and discarded. Only compressed metadata and summaries are processed server-side.

<figure><img src="../.gitbook/assets/2. How It Works@2x.png" alt=""><figcaption></figcaption></figure>

### The architecture: OpenClaw foundation

OpenHuman is built on top of the OpenClaw architecture and is open-sourced under the GNU GPL3 license. The full codebase is publicly available on GitHub.

The architecture uses multi-agent orchestration, where specialized agents are dynamically spawned for specific tasks rather than relying on one monolithic model. An orchestrator agent manages routing, personality, and context distribution. Specialist agents handle specific domains: communication analysis, document synthesis, task management, trading. These agents execute in parallel, not sequentially, for real-time responsiveness.

All agents share a common compressed context provided by Neocortex. This effectively creates a virtual context window far larger than any single model can support.

### Limitations

OpenHuman works with probabilistic models. It may occasionally miss nuance, misinterpret sarcasm, or over-prioritize certain messages. This is more likely in highly informal conversations, fast-moving threads, or contexts with limited data.

Screen Intelligence processes visual information through a local vision model, which means accuracy depends on screen clarity, text size, and application complexity. Highly dynamic or visually dense interfaces may produce less precise summaries.

OpenHuman is not AGI. It is a meaningful step closer, with better memory and better orchestration for memory, taking inspiration from the human brain. But human judgment still matters. OpenHuman helps you think faster. It does not think for you.

Everything is in early alpha. Feedback and contributions are welcomed.
