//! A turn on a caller-built runtime's orchestrator refuses attachments
//! rather than run without them: `inference.agent_chat` carries a string.

mod common;
use std::sync::Arc;

use common::{offline_config, runtime};
use openhuman_embed::{
    Core, CoreBuilder, CoreError, DomainSet, HostAttachment, HostKind, ServiceSet,
};

#[test]
fn the_orchestrator_refuses_attachments_it_cannot_carry() {
    runtime().block_on(async {
        tokio::spawn(async {
            let built = CoreBuilder::new(HostKind::Library)
                .config(offline_config())
                .services(ServiceSet::none())
                .domains(DomainSet::embedded())
                .build()
                .await
                .unwrap();
            let core = Core::from_runtime(Arc::new(built));
            let refused = core
                .agent()
                .turn("look")
                .attach(HostAttachment::new(
                    b"\x89PNG\r\n\x1a\n".to_vec(),
                    "image/png",
                ))
                .send()
                .await
                .unwrap_err();
            assert!(
                matches!(
                    &refused,
                    CoreError::Domain { kind: Some(kind), .. } if kind == "attachments_unsupported"
                ),
                "{refused:?}"
            );
        })
        .await
        .unwrap();
    });
}
