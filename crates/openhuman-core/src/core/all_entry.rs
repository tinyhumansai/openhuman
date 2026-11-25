//! Controller entry types shared by the global registry and domain modules.

use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::{Map, Value};

use crate::core::runtime::DispatchCtx;
use crate::core::ControllerSchema;

/// A pinned, boxed future returned by a controller handler.
pub type ControllerFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'static>>;

/// A boxed future used by typed controller parsers and gates.
pub type ControllerTask<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;

/// A function pointer type for controller handlers.
///
/// Handlers take the call's explicit [`DispatchCtx`] and a map of parameters,
/// and return a [`ControllerFuture`]. The transport that accepted the call
/// builds the context; the registry passes it through untouched.
pub type ControllerHandler = fn(DispatchCtx, Map<String, Value>) -> ControllerFuture;

/// A function pointer type for domain-specific CLI handlers.
pub type CliHandler = fn(&[String]) -> anyhow::Result<()>;

/// A registered standalone CLI adapter for a domain.
#[derive(Clone)]
pub struct RegisteredCliAdapter {
    pub namespace: &'static str,
    pub handler: CliHandler,
}

/// A registered controller combining its schema and handler function.
#[derive(Clone)]
pub struct RegisteredController {
    /// The schema defining the controller's identity and parameters.
    pub schema: ControllerSchema,
    /// The actual function that executes the controller's logic.
    pub handler: ControllerHandler,
}

impl RegisteredController {
    /// Returns the canonical RPC method name for this controller (e.g., `openhuman.memory_doc_put`).
    pub fn rpc_method_name(&self) -> String {
        crate::core::all::rpc_method_name(&self.schema)
    }
}

/// A controller whose runtime path and validation seam share one typed parser.
type BoundInvoke = dyn Fn(DispatchCtx, Map<String, Value>) -> ControllerFuture + Send + Sync;
type BoundDecode = dyn Fn(Map<String, Value>) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub struct BoundRegisteredController {
    pub schema: ControllerSchema,
    invoke: Arc<BoundInvoke>,
    decode: Arc<BoundDecode>,
}

impl BoundRegisteredController {
    /// Bind decoding and execution to the same parser function.
    pub fn typed<P: Send + 'static>(
        schema: ControllerSchema,
        parse: fn(Map<String, Value>) -> Result<P, String>,
        run: fn(DispatchCtx, P) -> ControllerFuture,
    ) -> Self {
        let decode = Arc::new(move |params| parse(params).map(|_| ())) as Arc<BoundDecode>;
        let invoke = Arc::new(move |ctx: DispatchCtx, params: Map<String, Value>| {
            Box::pin(async move { run(ctx, parse(params)?).await }) as ControllerFuture
        }) as Arc<BoundInvoke>;
        Self {
            schema,
            invoke,
            decode,
        }
    }

    /// Preserve a manual pre-parse gate while sharing the parser with validation.
    pub fn after_gate<G: Send + 'static, P: Send + 'static>(
        schema: ControllerSchema,
        gate: fn(DispatchCtx) -> ControllerTask<G>,
        parse: fn(Map<String, Value>) -> Result<P, String>,
        run: fn(DispatchCtx, G, P) -> ControllerFuture,
    ) -> Self {
        let decode = Arc::new(move |params| parse(params).map(|_| ())) as Arc<BoundDecode>;
        let invoke = Arc::new(move |ctx: DispatchCtx, params: Map<String, Value>| {
            Box::pin(async move {
                let gated = gate(ctx.clone()).await?;
                run(ctx, gated, parse(params)?).await
            }) as ControllerFuture
        }) as Arc<BoundInvoke>;
        Self {
            schema,
            invoke,
            decode,
        }
    }

    /// Decode params through the parser used by runtime invocation.
    pub fn decode_params(&self, params: Map<String, Value>) -> Result<(), String> {
        (self.decode)(params)
    }

    pub fn invoke(&self, ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
        (self.invoke)(ctx, params)
    }
}

/// Runtime registry entry, retaining the compatible legacy controller type.
#[derive(Clone)]
pub struct ControllerEntry {
    pub schema: ControllerSchema,
    handler: EntryHandler,
}

#[derive(Clone)]
enum EntryHandler {
    Legacy(ControllerHandler),
    Bound(BoundRegisteredController),
}

impl ControllerEntry {
    pub(crate) fn legacy(controller: RegisteredController) -> Self {
        Self {
            schema: controller.schema,
            handler: EntryHandler::Legacy(controller.handler),
        }
    }

    pub(crate) fn bound(controller: BoundRegisteredController) -> Self {
        Self {
            schema: controller.schema.clone(),
            handler: EntryHandler::Bound(controller),
        }
    }

    /// Returns the canonical RPC method name for this controller.
    pub fn rpc_method_name(&self) -> String {
        crate::core::all::rpc_method_name(&self.schema)
    }

    /// Decode params with a bound runtime parser, when this entry has one.
    pub fn decode_params(&self, params: Map<String, Value>) -> Result<(), String> {
        match &self.handler {
            EntryHandler::Legacy(_) => {
                Err(format!("{} has no bound parser", self.schema.method_name()))
            }
            EntryHandler::Bound(handler) => handler.decode_params(params),
        }
    }

    #[cfg(test)]
    pub(super) fn is_bound(&self) -> bool {
        matches!(self.handler, EntryHandler::Bound(_))
    }

    pub fn invoke(&self, ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
        match &self.handler {
            EntryHandler::Legacy(handler) => handler(ctx, params),
            EntryHandler::Bound(handler) => handler.invoke(ctx, params),
        }
    }
}
