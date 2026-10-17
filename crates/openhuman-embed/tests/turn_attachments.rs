//! Files a host attaches to a turn as bytes reach the model with the
//! message, ahead of it, and a turn over the configured limits never runs.
//! The orchestrator's refusal is in `orchestrator_attachments.rs`: a
//! caller-built runtime needs a process of its own.

mod common;
use common::{chat_requests, offline_config, provider, route, runtime, stub_backend};
use openhuman_embed::{AgentSpec, HostAttachment, Runtime, Workspace};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRpixels";
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";

#[test]
fn attached_files_reach_the_model_ahead_of_the_message() {
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let model = provider("seen").await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .unwrap();
            let agent = runtime
                .agent(AgentSpec::new("viewer").provider(route(&model, "fixture")))
                .unwrap();

            let outcome = agent
                .turn("what are these?")
                .session("with-files")
                .attach(HostAttachment::new(PNG, "image/png").named("photo.png"))
                .attachments([HostAttachment::new(PDF, "application/pdf").named("q3.pdf")])
                .send()
                .await
                .unwrap();
            assert_eq!(outcome.reply, "seen");

            // Both files are named to the model, in order and ahead of the
            // caption; their bytes never ride the reference text.
            let requests = chat_requests(&model).await;
            let body = String::from_utf8_lossy(&requests.last().unwrap().body).into_owned();
            let photo = body.find("photo.png").expect("the image reached the model");
            let report = body.find("q3.pdf").expect("the document reached the model");
            let caption = body.find("what are these?").expect("the caption too");
            assert!(photo < report && report < caption, "{body}");
            assert!(body.contains("image/png") && body.contains("application/pdf"));
            assert!(
                !body.contains("IHDRpixels"),
                "raw bytes in the transcript text"
            );

            // Over the configured limit: the turn fails before it runs.
            let before = chat_requests(&model).await.len();
            let crowd = (0..17).map(|_| HostAttachment::new(PNG, "image/png"));
            let refused = agent
                .turn("too many")
                .session("with-files")
                .attachments(crowd)
                .send()
                .await
                .unwrap_err();
            assert!(refused.to_string().contains("count exceeds"), "{refused}");
            assert_eq!(chat_requests(&model).await.len(), before);
        })
        .await
        .unwrap();
    });
}
