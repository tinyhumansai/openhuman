use super::handlers::*;
use super::*;

pub(super) fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("create"),
            handler: handle_create,
        },
        RegisteredController {
            schema: schemas("duplicate"),
            handler: handle_duplicate,
        },
        RegisteredController {
            schema: schemas("validate"),
            handler: handle_validate,
        },
        RegisteredController {
            schema: schemas("import"),
            handler: handle_import,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("update"),
            handler: handle_update,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_delete,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
        RegisteredController {
            schema: schemas("run_detached"),
            handler: handle_run_detached,
        },
        RegisteredController {
            schema: schemas("resume"),
            handler: handle_resume,
        },
        RegisteredController {
            schema: schemas("cancel_run"),
            handler: handle_cancel_run,
        },
        RegisteredController {
            schema: schemas("list_runs"),
            handler: handle_list_runs,
        },
        RegisteredController {
            schema: schemas("list_all_runs"),
            handler: handle_list_all_runs,
        },
        RegisteredController {
            schema: schemas("get_run"),
            handler: handle_get_run,
        },
        RegisteredController {
            schema: schemas("prune_runs"),
            handler: handle_prune_runs,
        },
        RegisteredController {
            schema: schemas("build"),
            handler: handle_build,
        },
        RegisteredController {
            schema: schemas("build_cancel"),
            handler: handle_build_cancel,
        },
        RegisteredController {
            schema: schemas("discover"),
            handler: handle_discover,
        },
        RegisteredController {
            schema: schemas("list_suggestions"),
            handler: handle_list_suggestions,
        },
        RegisteredController {
            schema: schemas("dismiss_suggestion"),
            handler: handle_dismiss_suggestion,
        },
        RegisteredController {
            schema: schemas("mark_suggestion_built"),
            handler: handle_mark_suggestion_built,
        },
        RegisteredController {
            schema: schemas("draft_create"),
            handler: handle_draft_create,
        },
        RegisteredController {
            schema: schemas("draft_get"),
            handler: handle_draft_get,
        },
        RegisteredController {
            schema: schemas("draft_update"),
            handler: handle_draft_update,
        },
        RegisteredController {
            schema: schemas("draft_list"),
            handler: handle_draft_list,
        },
        RegisteredController {
            schema: schemas("draft_delete"),
            handler: handle_draft_delete,
        },
        RegisteredController {
            schema: schemas("draft_promote"),
            handler: handle_draft_promote,
        },
        RegisteredController {
            schema: schemas("get_history"),
            handler: handle_get_history,
        },
        RegisteredController {
            schema: schemas("rollback"),
            handler: handle_rollback,
        },
        RegisteredController {
            schema: schemas("search_tool_catalog"),
            handler: handle_search_tool_catalog,
        },
        RegisteredController {
            schema: schemas("get_tool_contract"),
            handler: handle_get_tool_contract,
        },
        RegisteredController {
            schema: schemas("required_connections"),
            handler: handle_required_connections,
        },
        RegisteredController {
            schema: schemas("approval_manifest"),
            handler: handle_approval_manifest,
        },
    ]
}
