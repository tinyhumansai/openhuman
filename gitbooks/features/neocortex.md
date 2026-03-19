---
icon: brain
---

# Neocortex

Neocortex is OpenHuman's memory engine. It is a human-like AI memory system designed to work accurately with over 1 billion tokens of data while supporting the computational demands of a subconscious system.

<figure><img src="../.gitbook/assets/V09 — Neocortex Hero@2x.png" alt=""><figcaption></figcaption></figure>

Neocortex goes far beyond vector databases that simply store embeddings and retrieve whatever is semantically similar. It understands time, entities, and relationships. It builds knowledge graphs. It forgets strategically. It models memory the way the human brain does.

#### Why existing memory solutions fall short

Traditional AI memory tries to remember everything. It retrieves whatever is similar, but similarity alone says nothing about importance. A message from six months ago that happens to share keywords with your current query adds noise, not signal.

The deeper problem is architectural. Research from Google's Titans project shows that as LLMs absorb more data, they become less accurate. Larger context windows do not fix this. They make things worse: slower, more expensive, more error-prone.

Existing solutions like SuperMemory, Mem0, HydraDB, and MemGPT are not capable of supporting a conscious system, nor can they process data accurately at a scale of over 10 million tokens. This is likely why attempts at AI super-intelligence today remain slow, expensive, and frequently inaccurate.

Neocortex was built to solve all of these problems simultaneously.

#### Performance

Neocortex achieves performance that is orders of magnitude ahead of alternatives:

**Indexing speed:** 10 million tokens in under 10 seconds. Roughly 1,000x faster than other solutions. This means every single thing that happens in your digital life can be processed and made available to your agent in near real-time.

**Scale:** Over 1 billion tokens supported. This is the Big Data moment for AI memory. No other system operates at this scale with comparable accuracy.

**Cost:** $1 to index 5 million tokens. Roughly 10x cheaper than other decent AI memory solutions. Neocortex achieves this because it does not use any LLMs to manage its intelligence. The memory layer itself has zero language model dependency.

**Hardware:** Runs on the CPU of a MacBook Air. No GPU required. This makes Neocortex deployable anywhere, not just on expensive cloud infrastructure.

Benchmark data is open-sourced on [GitHub](https://github.com/tinyhumansai/neocortex/tree/main/benchmarks).

#### Architecture: tiered memory

Neocortex uses a tiered memory architecture modeled on how the human brain manages information:

**HOT** tier holds the most recent and actively relevant context. This is what your agent draws on for immediate queries and real-time interactions.

**WARM** tier contains important information that has cooled in immediate relevance but remains readily accessible. Context from the past few days or weeks that is still useful.

**COOL** tier stores older context that is less immediately useful but still retrievable when the right cues appear. Think of project details from last month that you are not actively working on.

**COLD** tier holds the oldest and least-accessed information. Kept but heavily deprioritized. Reactivated only when specific context cues surface.

Intelligent pruning moves memories between tiers based on recency, frequency of access, and contextual relevance. Nothing is ever permanently deleted. Memories can always be reactivated.

<figure><img src="../.gitbook/assets/V11 — Tiered Memory Deep Dive@2x (2).png" alt=""><figcaption></figcaption></figure>

#### The neuroscience of forgetting

The tiered architecture is grounded in neuroscience research.

<figure><img src="../.gitbook/assets/V12_Neuroscience_of_Forgetting@2x.png" alt=""><figcaption></figcaption></figure>

A Carnegie Mellon University study demonstrated that intentionally forgetting some things helps us remember others by freeing up working memory resources. Memory formation depletes a limited working memory resource that recovers over time.

Further research by Ryan and Frankland, published in Nature Reviews Neuroscience (2022), established that forgetting is an active biological process with dedicated molecular machinery. Memories become inaccessible rather than erased; the engram cells persist. Environmental cues can reactivate "lost" memories.

A TIME article on the science of forgetting reinforced this: neurons have a completely separate set of mechanisms dedicated to active forgetting. Culling information is as essential for cognition as gathering it.

This validates the core design principle of Neocortex. AI systems that try to remember everything are architecturally wrong. The brain does not work that way. Neocortex compresses, forgets strategically, and builds structured representations instead.

#### Knowledge graph, not flat text

Neocortex does not store flat text. It builds knowledge graphs: structured representations of entities, relationships, and temporal chains.

<figure><img src="../.gitbook/assets/V10_Knowledge_Graph_Visualization@2x.png" alt=""><figcaption></figcaption></figure>

**Semantic deduplication** strips redundant noise before it enters the graph. In group chats, 80%+ of messages are repetitive. Neocortex removes this automatically.

**Cross-source entity resolution** means that the same concept mentioned across different platforms is understood as one entity. A "Q4 deck" referenced in Slack, a Notion page, and an email thread becomes a single node in the graph, not three disconnected fragments.

**Temporal context weighting** applies time-decay functions so recent events carry more weight than older ones. This mirrors how human attention naturally prioritizes recency.

The result: millions of tokens of organizational noise compressed into a structured, queryable knowledge graph that any AI model can reason over in real time.

#### Neocortex and the subconscious

Neocortex serves as the foundation for OpenHuman's subconscious, going well beyond retrieval.

Good memory recall is the prerequisite for consciousness. Neocortex provides this by recalling memories ranked on three factors: time, interactions, and randomness. The randomness element is critical. It is what enables the subconscious system to make unexpected connections and surface emergent insights.

More on the subconscious system in [The Subconscious](the-subconscious.md).
