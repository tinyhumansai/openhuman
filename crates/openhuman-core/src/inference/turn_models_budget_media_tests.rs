use super::*;
use std::sync::Mutex;

use tinyagents_harness::cost::ModelBudget;
use tinyinference_llm::message::{ImageRef, MediaRef, Message, UserMessage};
use tinyinference_llm::model::budget::{Budget, CallBudget, SpendLimits};
use tinyinference_llm::model::{InputModality, InputSource, ModelProfile};
use tinyinference_llm::usage::{ChargedAmount, Usage};

const CHAT_CALL_MICROS: u64 = 100;
const READOUT_CHARGE_MICROS: i64 = 7;

struct RecordingModel {
    profile: ModelProfile,
    image_transport: bool,
    reply: &'static str,
    usage: Option<Usage>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl RecordingModel {
    fn chat(image_in: bool) -> Self {
        let mut profile = ModelProfile::default();
        profile.modalities.image_in = image_in;
        Self {
            profile,
            image_transport: image_in,
            reply: "main reply",
            usage: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn vision() -> Self {
        let mut profile = ModelProfile::default();
        profile.provider = Some("injected".into());
        profile.modalities.image_in = true;
        Self {
            profile,
            image_transport: true,
            reply: "a cat holding a sign",
            usage: Some(Usage {
                input_tokens: 40,
                output_tokens: 6,
                total_tokens: 46,
                charged_amount: Some(ChargedAmount::usd_micros(READOUT_CHARGE_MICROS)),
                ..Default::default()
            }),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatModel<()> for RecordingModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    fn supports_input(&self, modality: InputModality, mime: &str, source: InputSource) -> bool {
        self.image_transport
            && modality == InputModality::Image
            && source == InputSource::Base64
            && matches!(mime, "image/jpeg" | "image/webp")
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.requests.lock().unwrap().push(request);
        let mut response = ModelResponse::assistant(self.reply);
        response.usage = self.usage;
        Ok(response)
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    _context: Arc<crate::core::runtime::CoreContext>,
    chat: Arc<RecordingModel>,
    vision: Arc<RecordingModel>,
    budget: ModelBudget,
    models: TurnModels,
}

async fn budgeted_turn(chat_image_in: bool, input_tokens: u64) -> Fixture {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::core::types::HostKind;

    let tmp = tempfile::tempdir().expect("isolated workspace");
    tokio::fs::write(tmp.path().join("photo.jpg"), b"\xff\xd8\xff\xe0photo bytes")
        .await
        .unwrap();
    tokio::fs::write(
        tmp.path().join("sticker.webp"),
        b"RIFF\x10\x00\x00\x00WEBPVP8 sticker",
    )
    .await
    .unwrap();
    tokio::fs::write(tmp.path().join("report.pdf"), b"%PDF-1.4 report bytes")
        .await
        .unwrap();
    let mut config = crate::config::Config::default();
    config.action_dir = tmp.path().into();
    config.workspace_dir = tmp.path().join("internal");
    config.config_path = config.workspace_dir.join("config.toml");
    config.modules.enabled = false;
    config.modules.allow_download = false;
    config.chat_provider = Some("openhuman".into());

    let chat = Arc::new(RecordingModel::chat(chat_image_in));
    let vision = Arc::new(RecordingModel::vision());
    let mut overrides = HostOverrides::default();
    overrides.model = Some(chat.clone());
    overrides
        .role_models
        .insert("vision".into(), vision.clone() as Arc<dyn ChatModel<()>>);
    let root = CoreContext::for_test_with_host_kind(HostKind::Library, DomainSet::full(), None);
    let context = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
            .host_overrides(Arc::new(overrides)),
    );
    let ctx = DispatchCtx::new(context.clone());
    let budget = ModelBudget::new(
        Budget::new(SpendLimits {
            tokens: None,
            cost_micros: Some(1_000_000),
        }),
        CallBudget {
            input_tokens,
            output_tokens: 1_024,
            cost_micros: CHAT_CALL_MICROS,
        },
    );
    let models = TurnModelSource::new_crate_native(&ctx, "chat", Arc::new(config))
        .build("hint:chat", 0.0, None, None)
        .expect("budgeted turn models build")
        .with_host_controls(Some(&budget), None);
    Fixture {
        _tmp: tmp,
        _context: context,
        chat,
        vision,
        budget,
        models,
    }
}

fn user(blocks: Vec<ContentBlock>) -> Message {
    Message::User(UserMessage { content: blocks })
}

fn image(path: &str, mime: &str) -> ContentBlock {
    ContentBlock::Image(ImageRef {
        url: path.into(),
        mime_type: Some(mime.into()),
    })
}

fn is_text_only(request: &ModelRequest) -> bool {
    request.messages.iter().all(|message| match message {
        Message::User(user) => user
            .content
            .iter()
            .all(|block| matches!(block, ContentBlock::Text(_))),
        _ => true,
    })
}

fn spent_micros(budget: &ModelBudget) -> u64 {
    budget.ledger.snapshot().spent.cost_micros
}

#[tokio::test]
async fn budgeted_non_vision_turn_admits_rendered_image_and_charges_readout_and_chat() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let fixture = budgeted_turn(false, 200_000).await;
    let photo_turn = user(vec![
        ContentBlock::Text("what is in this photo?".into()),
        image("photo.jpg", "image/jpeg"),
    ]);

    let response = fixture
        .models
        .primary
        .invoke(&(), ModelRequest::new(vec![photo_turn.clone()]))
        .await
        .expect("photo turn is admitted after rendering");

    assert_eq!(Message::Assistant(response.message).text(), "main reply");
    let vision_calls = fixture.vision.requests();
    assert_eq!(vision_calls.len(), 1);
    let chat_calls = fixture.chat.requests();
    assert_eq!(chat_calls.len(), 1);
    assert!(is_text_only(&chat_calls[0]));
    assert!(chat_calls[0].messages[0]
        .text()
        .contains("Image readout:\na cat holding a sign"));
    assert_eq!(
        spent_micros(&fixture.budget),
        READOUT_CHARGE_MICROS as u64 + CHAT_CALL_MICROS
    );

    let follow_up = ModelRequest::new(vec![
        photo_turn,
        Message::assistant("main reply"),
        Message::user("and what colour is it?"),
    ]);
    fixture
        .models
        .primary
        .invoke(&(), follow_up)
        .await
        .expect("a later text turn over the same history is admitted");
    let chat_calls = fixture.chat.requests();
    assert_eq!(chat_calls.len(), 2);
    assert!(is_text_only(&chat_calls[1]));
    assert!(spent_micros(&fixture.budget) > READOUT_CHARGE_MICROS as u64 + CHAT_CALL_MICROS);
}

#[tokio::test]
async fn budgeted_non_vision_turn_admits_webp_sticker_and_pdf_document() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let fixture = budgeted_turn(false, 200_000).await;

    fixture
        .models
        .primary
        .invoke(
            &(),
            ModelRequest::new(vec![user(vec![image("sticker.webp", "image/webp")])]),
        )
        .await
        .expect("sticker turn is admitted after rendering");
    assert_eq!(fixture.vision.requests().len(), 1);
    assert_eq!(
        spent_micros(&fixture.budget),
        READOUT_CHARGE_MICROS as u64 + CHAT_CALL_MICROS
    );

    fixture
        .models
        .primary
        .invoke(
            &(),
            ModelRequest::new(vec![user(vec![
                ContentBlock::Text("summarise this".into()),
                ContentBlock::Document(MediaRef::Path {
                    path: "report.pdf".into(),
                    media_type: Some("application/pdf".into()),
                }),
            ])]),
        )
        .await
        .expect("document turn is admitted after rendering");
    let chat_calls = fixture.chat.requests();
    assert_eq!(chat_calls.len(), 2);
    assert!(chat_calls.iter().all(is_text_only));
    assert!(chat_calls[1].messages[0].text().contains("application/pdf"));
    assert_eq!(
        spent_micros(&fixture.budget),
        READOUT_CHARGE_MICROS as u64 + 2 * CHAT_CALL_MICROS
    );
}

#[tokio::test]
async fn budgeted_vision_turn_sends_the_readout_instead_of_native_image() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let fixture = budgeted_turn(true, 200_000).await;

    fixture
        .models
        .primary
        .invoke(
            &(),
            ModelRequest::new(vec![user(vec![image("photo.jpg", "image/jpeg")])]),
        )
        .await
        .expect("vision turn is admitted under a budget");

    assert_eq!(fixture.vision.requests().len(), 1);
    let chat_calls = fixture.chat.requests();
    assert_eq!(chat_calls.len(), 1);
    assert!(is_text_only(&chat_calls[0]));
    assert_eq!(
        spent_micros(&fixture.budget),
        READOUT_CHARGE_MICROS as u64 + CHAT_CALL_MICROS
    );
}

#[tokio::test]
async fn budgeted_turn_still_refuses_a_request_over_the_input_bound() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let fixture = budgeted_turn(false, 64).await;

    let error = fixture
        .models
        .primary
        .invoke(&(), ModelRequest::new(vec![Message::user("x".repeat(512))]))
        .await
        .expect_err("an oversized request is refused before dispatch");

    assert!(
        error.to_string().contains("conservative input-token bound"),
        "{error}"
    );
    assert!(fixture.chat.requests().is_empty());
    assert_eq!(spent_micros(&fixture.budget), 0);
}
