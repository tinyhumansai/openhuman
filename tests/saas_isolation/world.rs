//! The adversarial driver's state: three users, their canaries, the ids
//! harvested from every response (tagged with the user they came from), the
//! `/events` frames each user received, and the seeded RNG that picks the
//! next call.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use super::*;

/// The provisioned users. No name is a prefix of another, so a canary names
/// exactly one owner.
pub const USERS: [&str; 3] = ["alice", "bob", "carol"];

/// The user whose turn stays live through the whole run, so the others can
/// aim cancel, queue and steer calls at it.
pub const VICTIM: usize = 0;

/// A string only `user` ever writes.
pub fn canary(user: usize, tag: &str) -> String {
    format!("CANARY-{}-{tag}", USERS[user])
}

/// `user`'s backend credential: a canary too, so it may never surface
/// anywhere another user can see.
pub fn credential(user: usize) -> String {
    canary(user, "credential")
}

/// The users whose canaries `text` contains.
pub fn owners_in(text: &str) -> Vec<usize> {
    (0..USERS.len())
        .filter(|&u| text.contains(&format!("CANARY-{}-", USERS[u])))
        .collect()
}

/// The users other than `owner` whose canaries `text` contains.
pub fn foreign_in(text: &str, owner: usize) -> Vec<&'static str> {
    owners_in(text)
        .into_iter()
        .filter(|&u| u != owner)
        .map(|u| USERS[u])
        .collect()
}

/// SplitMix64: small, seedable and the same on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// True with probability `percent` / 100.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// An id seen in `owner`'s responses, with the thread it belongs to.
#[derive(Debug, Clone)]
pub struct Owned {
    pub owner: usize,
    pub value: String,
    pub thread: Option<String>,
}

#[derive(Default)]
pub struct Pools {
    pub threads: Vec<Owned>,
    pub requests: Vec<Owned>,
    pub messages: Vec<Owned>,
    pub clients: Vec<Owned>,
    pub queue_items: Vec<Owned>,
    pub memory_ids: Vec<Owned>,
    /// Relayed messages, by owner: `(channel, chat_id, sender_id, message_id)`.
    pub relays: Vec<(usize, [String; 4])>,
}

impl Pools {
    fn push(list: &mut Vec<Owned>, owner: usize, value: &str, thread: Option<&str>) {
        Self::push_unless(list, owner, value, thread, &HashSet::new());
    }

    /// [`Self::push`], skipping a value the caller sent itself: an id a
    /// response only echoes back is not the caller's.
    fn push_unless(
        list: &mut Vec<Owned>,
        owner: usize,
        value: &str,
        thread: Option<&str>,
        sent: &HashSet<String>,
    ) {
        if sent.contains(value) {
            return;
        }
        if value.is_empty() || value.len() > 200 || value.contains("CANARY") {
            return;
        }
        if list.iter().any(|o| o.owner == owner && o.value == value) {
            return;
        }
        list.push(Owned {
            owner,
            value: value.to_string(),
            thread: thread.map(str::to_owned),
        });
    }
}

/// `/events` frames as they arrive: `(listening user, client id, data)`.
pub type Frames = Arc<Mutex<Vec<(usize, String, String)>>>;

pub struct World {
    pub node: Node,
    pub rng: Rng,
    pub seed: u64,
    pub step: usize,
    counter: u64,
    pub pools: Pools,
    /// Thread ids each owner keeps away from their own writes, so their state
    /// at the end shows only what other users did to them.
    pub protected: [HashSet<String>; 3],
    /// Requests each owner keeps away from their own cancels.
    pub protected_requests: [HashSet<String>; 3],
    /// Relay keys each user sent themselves.
    pub relayed: [HashSet<String>; 3],
    pub frames: Frames,
    pub streams: usize,
    pub trace: Vec<String>,
    /// Per method: (answered with a result, answered with an error, other status).
    pub stats: BTreeMap<String, (u32, u32, u32)>,
    /// The first error each method answered, for the run summary.
    pub first_error: BTreeMap<String, String>,
}

impl World {
    pub fn new(node: Node, seed: u64) -> Self {
        Self {
            node,
            rng: Rng::new(seed),
            seed,
            step: 0,
            counter: 0,
            pools: Pools::default(),
            protected: Default::default(),
            protected_requests: Default::default(),
            relayed: Default::default(),
            frames: Arc::new(Mutex::new(Vec::new())),
            streams: 0,
            trace: Vec::new(),
            stats: BTreeMap::new(),
            first_error: BTreeMap::new(),
        }
    }

    pub fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// A fresh canary for `user`.
    pub fn canary(&mut self, user: usize) -> String {
        let n = self.next_id();
        canary(user, &n.to_string())
    }

    /// Panic with everything needed to replay the run.
    pub fn fail(&self, what: &str) -> ! {
        let tail = &self.trace[..];
        panic!(
            "ISOLATION FAILURE (seed {seed}, step {step}): {what}\n\
             replay: OH_FUZZ_SEED={seed} OH_FUZZ_STEPS={steps} cargo test -p openhuman-cli \
             --test saas_profile_isolation_e2e\nlast calls:\n{}\ncore log:\n{}",
            tail.join("\n"),
            self.node.log_tail(),
            seed = self.seed,
            step = self.step,
            steps = steps_from_env(),
        );
    }

    fn pick_from(&mut self, list: Vec<Owned>) -> Option<Owned> {
        if list.is_empty() {
            None
        } else {
            let i = self.rng.below(list.len());
            Some(list[i].clone())
        }
    }

    fn of(list: &[Owned], keep: impl Fn(&Owned) -> bool) -> Vec<Owned> {
        list.iter().filter(|o| keep(o)).cloned().collect()
    }

    /// A thread id for `actor`, mostly one another user named. A `writing`
    /// call never gets one of the actor's own protected threads.
    pub fn thread_for(&mut self, actor: usize, writing: bool) -> String {
        for _ in 0..8 {
            let roll = self.rng.below(10);
            let pick = match roll {
                0..=4 => {
                    let others = Self::of(&self.pools.threads, |o| o.owner != actor);
                    self.pick_from(others).map(|o| o.value)
                }
                5..=7 => {
                    let own = Self::of(&self.pools.threads, |o| o.owner == actor);
                    self.pick_from(own).map(|o| o.value)
                }
                8 => {
                    let victim = (actor + 1 + self.rng.below(USERS.len() - 1)) % USERS.len();
                    let mut ids: Vec<String> = self.protected[victim].iter().cloned().collect();
                    ids.sort();
                    (!ids.is_empty()).then(|| ids[self.rng.below(ids.len())].clone())
                }
                _ => None,
            };
            let id = pick.unwrap_or_else(|| format!("t-{}", self.next_id()));
            if writing && self.protected[actor].contains(&id) {
                continue;
            }
            return id;
        }
        format!("t-{}", self.next_id())
    }

    /// An id from `list`, preferring another user's; sometimes made up.
    fn foreign_first(&mut self, actor: usize, which: fn(&Pools) -> &Vec<Owned>) -> String {
        let list = which(&self.pools).clone();
        let roll = self.rng.below(10);
        let pick = if roll < 7 {
            self.pick_from(Self::of(&list, |o| o.owner != actor))
        } else if roll < 9 {
            self.pick_from(Self::of(&list, |o| o.owner == actor))
        } else {
            None
        };
        pick.map(|o| o.value)
            .unwrap_or_else(|| format!("made-up-{}", self.next_id()))
    }

    pub fn request_for(&mut self, actor: usize) -> String {
        loop {
            let id = self.foreign_first(actor, |p| &p.requests);
            if !self.protected_requests[actor].contains(&id) {
                return id;
            }
        }
    }

    /// A request id and, when known, the thread it ran on: a cancel aimed
    /// at exactly another user's `(thread, request)`.
    pub fn request_on_thread(&mut self, actor: usize) -> (String, String) {
        let request = self.request_for(actor);
        let thread = self
            .pools
            .requests
            .iter()
            .find(|o| o.value == request)
            .and_then(|o| o.thread.clone())
            .filter(|t| !self.protected[actor].contains(t) && self.rng.chance(70));
        let thread = thread.unwrap_or_else(|| self.thread_for(actor, true));
        (request, thread)
    }

    pub fn message_for(&mut self, actor: usize) -> String {
        self.foreign_first(actor, |p| &p.messages)
    }

    pub fn queue_item_for(&mut self, actor: usize) -> String {
        self.foreign_first(actor, |p| &p.queue_items)
    }

    pub fn memory_id_for(&mut self, actor: usize) -> String {
        self.foreign_first(actor, |p| &p.memory_ids)
    }

    pub fn client_for(&mut self, actor: usize) -> String {
        if self.rng.chance(30) {
            return ["c1", "c2", "channel-relay"][self.rng.below(3)].to_string();
        }
        self.foreign_first(actor, |p| &p.clients)
    }

    /// Record every id in `body` (a response to `actor`'s `method` call)
    /// under `actor`.
    pub fn harvest(&mut self, actor: usize, method: &str, params: &Value, body: &Value) {
        let thread = params["thread_id"].as_str().map(str::to_owned);
        if let Some(client) = params["client_id"].as_str() {
            Pools::push(&mut self.pools.clients, actor, client, None);
        }
        // Every string anywhere in the params, however deeply nested.
        let mut sent: HashSet<String> = HashSet::new();
        let mut pending = vec![params];
        while let Some(value) = pending.pop() {
            match value {
                Value::String(s) => {
                    sent.insert(s.clone());
                }
                Value::Array(items) => pending.extend(items),
                Value::Object(map) => pending.extend(map.values()),
                _ => {}
            }
        }
        let mut stack = vec![(body.clone(), thread)];
        while let Some((value, thread)) = stack.pop() {
            match value {
                Value::Array(items) => {
                    stack.extend(items.into_iter().map(|v| (v, thread.clone())));
                }
                Value::Object(map) => {
                    let here = map
                        .get("thread_id")
                        .or_else(|| map.get("threadId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(thread);
                    for (key, v) in &map {
                        let Some(s) = v.as_str() else {
                            stack.push((v.clone(), here.clone()));
                            continue;
                        };
                        let t = here.as_deref();
                        let pools = &mut self.pools;
                        match key.as_str() {
                            "thread_id" | "threadId" => {
                                Pools::push(&mut pools.threads, actor, s, None)
                            }
                            "request_id" | "requestId" => {
                                Pools::push_unless(&mut pools.requests, actor, s, t, &sent)
                            }
                            "client_id" | "clientId" => {
                                Pools::push(&mut pools.clients, actor, s, None)
                            }
                            "item_id" | "itemId" => {
                                Pools::push_unless(&mut pools.queue_items, actor, s, t, &sent)
                            }
                            "message_id" | "messageId" => {
                                Pools::push_unless(&mut pools.messages, actor, s, t, &sent)
                            }
                            "id" => match method {
                                m if m.contains("threads_list")
                                    || m.contains("threads_create_new")
                                    || m.contains("threads_upsert") =>
                                {
                                    Pools::push(&mut pools.threads, actor, s, None)
                                }
                                m if m.contains("messages_list")
                                    || m.contains("message_append") =>
                                {
                                    Pools::push(&mut pools.messages, actor, s, t)
                                }
                                m if m.contains("memory_") => {
                                    Pools::push_unless(&mut pools.memory_ids, actor, s, None, &sent)
                                }
                                m if m.contains("queue_status") => {
                                    Pools::push_unless(&mut pools.queue_items, actor, s, t, &sent)
                                }
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Listen on `/events?client_id=` as `user`; frames land in [`Self::frames`].
    pub fn listen(&mut self, user: usize, client_id: &str) {
        let rx = user_events(&self.node.base, USERS[user], client_id);
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(first) if first == "status:200" => {}
            other => self.fail(&format!(
                "{}'s /events?client_id={client_id} did not open: {other:?}",
                USERS[user]
            )),
        }
        let frames = self.frames.clone();
        let client = client_id.to_string();
        std::thread::spawn(move || {
            while let Ok(data) = rx.recv() {
                frames.lock().unwrap().push((user, client.clone(), data));
            }
        });
        self.streams += 1;
    }
}

/// `OH_FUZZ_SEED`, or a fixed default so CI is reproducible.
pub fn seed_from_env() -> u64 {
    std::env::var("OH_FUZZ_SEED")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0x5AA5_150C)
}

/// `OH_FUZZ_STEPS`, default 200.
pub fn steps_from_env() -> usize {
    std::env::var("OH_FUZZ_STEPS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(200)
}
