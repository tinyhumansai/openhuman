use super::{disabled_component, resolver};
use crate::config::Config;
use crate::modules::registry;
use crate::modules::source;
use crate::modules::types::{ModuleRecord, ModuleState, ModuleStatus};
use tinybus::module::Availability;

/// Status of every module this build knows about.
#[must_use]
pub fn list(config: &Config) -> Vec<ModuleStatus> {
    registry::ALL
        .iter()
        .map(|record| status_of(config, record))
        .collect()
}

/// Status of one module.
fn status_of(config: &Config, record: &ModuleRecord) -> ModuleStatus {
    // Configuration is authoritative for the current core instance. A module
    // may remain loaded in this process after a prior request, but callers
    // whose configuration disables modules must still be told it is unusable.
    let (state, detail) = match disabled_component(config, record.id) {
        Some(component) => (
            ModuleState::Disabled,
            Some(format!(
                "COMPONENT_DISABLED:{component} (set [components] {component} = true to enable)"
            )),
        ),
        None => {
            let (availability, detail) = resolver(config).status_of(&record.release());
            (
                match availability {
                    Availability::Available => ModuleState::Available,
                    Availability::Loading => ModuleState::Loading,
                    Availability::Ready => ModuleState::Ready,
                    Availability::Failed => ModuleState::Failed,
                    Availability::Unsupported => ModuleState::Unsupported,
                    Availability::Disabled => ModuleState::Disabled,
                },
                detail,
            )
        }
    };
    let plan = source::plan(config, record.id);
    let module_source =
        (!matches!(state, ModuleState::Unsupported | ModuleState::Disabled)).then_some(plan.served);
    let detail = detail.or_else(|| plan.fallback.map(str::to_string));
    ModuleStatus {
        id: record.id.to_string(),
        description: record.description.to_string(),
        version: record.version.to_string(),
        bus_name: record.bus_name.to_string(),
        state,
        source: module_source,
        detail,
    }
}
