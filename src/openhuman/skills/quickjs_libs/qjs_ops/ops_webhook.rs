//! Webhook native operations: register, unregister, and list tunnel-to-skill mappings.
//!
//! All operations are scoped to the calling skill — the `skill_id` is baked
//! into the closure context at startup and cannot be overridden from JavaScript.

use rquickjs::{Ctx, Function, Object};

use super::types::{js_err, SkillContext};

/// Registers webhook operations onto the provided JavaScript object.
pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // Webhook Registration
    // Registers a tunnel UUID to be routed to this skill.
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set(
            "webhook_register",
            Function::new(
                ctx.clone(),
                move |tunnel_uuid: String,
                      tunnel_name: rquickjs::Value<'_>,
                      backend_tunnel_id: rquickjs::Value<'_>|
                      -> rquickjs::Result<()> {
                    let router = sc
                        .webhook_router
                        .as_ref()
                        .ok_or_else(|| js_err("Webhook router not available"))?;

                    let name = tunnel_name.as_string().and_then(|s| s.to_string().ok());
                    let backend_id = backend_tunnel_id
                        .as_string()
                        .and_then(|s| s.to_string().ok());

                    router
                        .register(&tunnel_uuid, &sc.skill_id, name, backend_id)
                        .map_err(js_err)
                },
            ),
        )?;
    }

    // ========================================================================
    // Webhook Unregistration
    // Removes a tunnel registration for this skill.
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set(
            "webhook_unregister",
            Function::new(
                ctx.clone(),
                move |tunnel_uuid: String| -> rquickjs::Result<()> {
                    let router = sc
                        .webhook_router
                        .as_ref()
                        .ok_or_else(|| js_err("Webhook router not available"))?;

                    router
                        .unregister(&tunnel_uuid, &sc.skill_id)
                        .map_err(js_err)
                },
            ),
        )?;
    }

    // ========================================================================
    // Webhook Listing
    // Returns a JSON array of all tunnel registrations for this skill.
    // ========================================================================

    {
        let sc = skill_context;
        ops.set(
            "webhook_list",
            Function::new(ctx.clone(), move || -> rquickjs::Result<String> {
                let router = sc
                    .webhook_router
                    .as_ref()
                    .ok_or_else(|| js_err("Webhook router not available"))?;

                let registrations = router.list_for_skill(&sc.skill_id);
                serde_json::to_string(&registrations).map_err(|e| js_err(e.to_string()))
            }),
        )?;
    }

    Ok(())
}
