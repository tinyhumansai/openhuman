//! Generic, id-keyed extension slots.
//!
//! A [`HookSlot`] is a small ordered table that an outside crate fills at
//! startup and a core domain consults at a fixed point (the tool catalogue, a
//! voice factory, the inference factory, ...). It knows nothing about what it
//! holds: the value type `V` and the id are the domain's, and any host can
//! register any service under any id. With nothing registered every consult
//! point falls through to the built-in behaviour, byte for byte.
//!
//! # Scope
//!
//! Registrations are **process-global** by default ([`HookSlot::register`]).
//! A host can also bind registrations to one [`CoreContext`]
//! ([`HookSlot::register_in`] on an [`Extensions`] handed to
//! [`CoreBuilder::extensions`](crate::core::runtime::CoreBuilder::extensions)),
//! so a multi-tenant host can give each context its own services. Lookups try the ambient
//! context first and the process table second: a context entry replaces a
//! global one with the same id, and context-only ids are appended after the
//! global ones.
//!
//! # Order
//!
//! [`HookSlot::entries`] is registration order (re-registering an id keeps its
//! original position), so a consult point that appends several sources has a
//! deterministic catalogue order.
//!
//! # Tests
//!
//! In `cfg(test)` builds the process table lives in a thread-local, so a test
//! that registers a fake never leaks it into a sibling test running on another
//! thread. Consult points that hop threads inside a test must register through
//! [`HookSlot::register_in`] instead.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// An ordered `id -> value` table.
pub struct Registry<V> {
    entries: RwLock<Vec<(String, V)>>,
}

impl<V> Registry<V> {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }
}

impl<V> Default for Registry<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> Registry<V> {
    /// Inserts `value` under `id`; an existing id keeps its position.
    pub fn insert(&self, id: String, value: V) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match entries.iter_mut().find(|(existing, _)| *existing == id) {
            Some(slot) => slot.1 = value,
            None => entries.push((id, value)),
        }
    }

    /// Removes `id`; whether it was present.
    pub fn remove(&self, id: &str) -> bool {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = entries.len();
        entries.retain(|(existing, _)| existing != id);
        entries.len() != before
    }

    /// Drops every entry.
    pub fn clear(&self) {
        self.entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// The value under `id`.
    pub fn get(&self, id: &str) -> Option<V> {
        self.entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(existing, _)| existing == id)
            .map(|(_, value)| value.clone())
    }

    /// Every entry in registration order.
    pub fn entries(&self) -> Vec<(String, V)> {
        self.entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

type Table = Arc<dyn Any + Send + Sync>;

/// The per-context registrations of every [`HookSlot`], keyed by slot name.
///
/// Cheap to clone: clones share one table set, so a handle given to the
/// builder and the context built from it see the same registrations.
#[derive(Clone, Default)]
pub struct Extensions {
    tables: Arc<RwLock<HashMap<&'static str, Table>>>,
}

impl Extensions {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The table `name` holds for values of type `V`, created on first use.
    ///
    /// A name reused for a different value type gets a fresh, detached table
    /// rather than a panic; slot names are unique per slot so this only guards
    /// against a programming error.
    fn table<V: Clone + Send + Sync + 'static>(&self, name: &'static str) -> Arc<Registry<V>> {
        let mut tables = self
            .tables
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = tables
            .entry(name)
            .or_insert_with(|| Arc::new(Registry::<V>::new()) as Table);
        match Arc::clone(table).downcast::<Registry<V>>() {
            Ok(typed) => typed,
            Err(_) => {
                log::warn!("[slots] slot `{name}` reused with a different value type");
                Arc::new(Registry::new())
            }
        }
    }

    fn existing<V: Clone + Send + Sync + 'static>(
        &self,
        name: &'static str,
    ) -> Option<Arc<Registry<V>>> {
        let tables = self
            .tables
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tables
            .get(name)
            .and_then(|table| Arc::clone(table).downcast::<Registry<V>>().ok())
    }

    /// Copies `other`'s tables into this set (a table already here is kept).
    pub fn merge_from(&self, other: &Extensions) {
        if Arc::ptr_eq(&self.tables, &other.tables) {
            return;
        }
        let source = other
            .tables
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut tables = self
            .tables
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, table) in source {
            tables.entry(name).or_insert(table);
        }
    }

    /// How many slots hold context registrations.
    pub fn len(&self) -> usize {
        self.tables
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Whether no slot holds a context registration.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extensions")
            .field("slots", &self.len())
            .finish()
    }
}

#[cfg(test)]
thread_local! {
    static TEST_GLOBALS: std::cell::RefCell<HashMap<&'static str, Box<dyn Any>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// A named, typed extension point. Declare one as a `static` next to the
/// consult point, with a unique `name`.
pub struct HookSlot<V> {
    name: &'static str,
    #[cfg(not(test))]
    global: Registry<V>,
    #[cfg(test)]
    _marker: std::marker::PhantomData<fn() -> V>,
}

impl<V: Clone + Send + Sync + 'static> HookSlot<V> {
    /// A slot called `name` (unique across the process; used in logs and as the
    /// per-context table key).
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            #[cfg(not(test))]
            global: Registry::new(),
            #[cfg(test)]
            _marker: std::marker::PhantomData,
        }
    }

    /// The slot's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    #[cfg(not(test))]
    fn with_global<R>(&self, f: impl FnOnce(&Registry<V>) -> R) -> R {
        f(&self.global)
    }

    #[cfg(test)]
    fn with_global<R>(&self, f: impl FnOnce(&Registry<V>) -> R) -> R {
        TEST_GLOBALS.with(|globals| {
            let mut globals = globals.borrow_mut();
            let table = globals
                .entry(self.name)
                .or_insert_with(|| Box::new(Registry::<V>::new()));
            match table.downcast_ref::<Registry<V>>() {
                Some(typed) => f(typed),
                None => f(&Registry::new()),
            }
        })
    }

    /// Registers `value` under `id` for the whole process.
    pub fn register(&self, id: impl Into<String>, value: V) {
        let id = id.into();
        log::debug!("[slots] register slot={} id={id} scope=process", self.name);
        self.with_global(|table| table.insert(id, value));
    }

    /// Registers `value` under `id` for the context built from `extensions`.
    pub fn register_in(&self, extensions: &Extensions, id: impl Into<String>, value: V) {
        let id = id.into();
        log::debug!("[slots] register slot={} id={id} scope=context", self.name);
        extensions.table::<V>(self.name).insert(id, value);
    }

    /// Removes the process-wide registration for `id`.
    pub fn unregister(&self, id: &str) -> bool {
        log::debug!("[slots] unregister slot={} id={id}", self.name);
        self.with_global(|table| table.remove(id))
    }

    /// Removes every process-wide registration.
    pub fn clear(&self) {
        log::debug!("[slots] clear slot={}", self.name);
        self.with_global(Registry::clear);
    }

    fn context_table(&self) -> Option<Arc<Registry<V>>> {
        crate::core::runtime::context::CoreContext::current_extensions()
            .and_then(|extensions| extensions.existing::<V>(self.name))
    }

    /// The value for `id`: the ambient context's, else the process-wide one.
    pub fn get(&self, id: &str) -> Option<V> {
        self.context_table()
            .and_then(|table| table.get(id))
            .or_else(|| self.with_global(|table| table.get(id)))
    }

    /// The value for `id` in an explicitly selected context, then process
    /// registration. `None` deliberately means process-only and never consults
    /// an unrelated ambient task context.
    pub fn get_for(&self, extensions: Option<&Extensions>, id: &str) -> Option<V> {
        extensions
            .and_then(|extensions| extensions.existing::<V>(self.name))
            .and_then(|table| table.get(id))
            .or_else(|| self.with_global(|table| table.get(id)))
    }

    /// Every registration in order: process-wide first, a context entry
    /// replacing a process entry with the same id, context-only ids last.
    pub fn entries(&self) -> Vec<(String, V)> {
        let mut entries = self.with_global(Registry::entries);
        if let Some(context) = self.context_table() {
            for (id, value) in context.entries() {
                match entries.iter_mut().find(|(existing, _)| *existing == id) {
                    Some(slot) => slot.1 = value,
                    None => entries.push((id, value)),
                }
            }
        }
        entries
    }

    /// Every registration, context entries first.
    ///
    /// For consult points that pick one winner (`first`, "the first factory
    /// that answers"): a context registration must take precedence over a
    /// process-wide one, as it does in [`get`](Self::get). The ambient context
    /// is the task's own scope under SaaS, so a tenant's registration wins
    /// there too. Process entries follow in registration order, minus any id
    /// the context replaced.
    pub fn entries_context_first(&self) -> Vec<(String, V)> {
        let mut entries = self
            .context_table()
            .map(|context| context.entries())
            .unwrap_or_default();
        for (id, value) in self.with_global(Registry::entries) {
            if !entries.iter().any(|(existing, _)| *existing == id) {
                entries.push((id, value));
            }
        }
        entries
    }

    /// The winning registration: the first of
    /// [`entries_context_first`](Self::entries_context_first).
    pub fn first(&self) -> Option<V> {
        self.entries_context_first()
            .into_iter()
            .next()
            .map(|(_, value)| value)
    }

    /// Whether nothing is registered (process-wide or in the ambient context).
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }
}

#[cfg(test)]
#[path = "slots_tests.rs"]
mod tests;
