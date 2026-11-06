//! TS-E2E X5: a caller on the core's module bus that is not the module a host
//! callback exists for.
//!
//! tinybus offers no way to attach a second, untrusted peer to the module
//! runtime (its broker is `new_restricted_module_control` and keeps no
//! transport; TS-E2E finding X5-GAP). So the caller is the runtime's own
//! module-controller connection. A member that answers it does not check its
//! sender, and would answer another loaded module the same way; a member that
//! refuses it refuses everything but its bound module.

use serde::{de::DeserializeOwned, Serialize};

/// How the call is delivered.
#[derive(Clone, Copy, Debug)]
pub enum Delivery {
    Plain,
    Confidential,
}

/// Call `interface.member` on the object `bus_name` serves at `path`, from the
/// module bus's controller connection, on the bus's own executor.
pub async fn call<Request, Reply>(
    bus_name: &str,
    path: &str,
    interface: &str,
    member: &str,
    request: Request,
    delivery: Delivery,
) -> tinybus::Result<Reply>
where
    Request: Serialize + Send + 'static,
    Reply: DeserializeOwned + Send + 'static,
{
    let runtime = openhuman_core::modules::runtime::runtime()
        .await
        .expect("the module runtime and its host callbacks must start");
    let (bus_name, path, interface, member) = (
        bus_name.to_owned(),
        path.to_owned(),
        interface.to_owned(),
        member.to_owned(),
    );
    runtime
        .spawn(async move {
            let proxy = runtime.connection().proxy(&bus_name, &path, &interface)?;
            match delivery {
                Delivery::Plain => proxy.call(&member, (request,)).await,
                Delivery::Confidential => proxy.call_confidential(&member, (request,)).await,
            }
        })
        .await
        .expect("the module bus task")
}

/// Whether `bus_name` has an owner on the module bus, so a refusal is the
/// member's answer and not a missing object.
pub async fn is_served(bus_name: &str, path: &str, interface: &str) -> bool {
    let runtime = openhuman_core::modules::runtime::runtime()
        .await
        .expect("the module runtime and its host callbacks must start");
    let (bus_name, path, interface) = (bus_name.to_owned(), path.to_owned(), interface.to_owned());
    runtime
        .spawn(async move {
            runtime
                .connection()
                .proxy(&bus_name, &path, &interface)
                .expect("a valid proxy address")
                .is_available()
                .await
                .unwrap_or(false)
        })
        .await
        .expect("the module bus task")
}
