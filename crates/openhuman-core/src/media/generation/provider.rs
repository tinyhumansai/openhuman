//! The image and video generators the media tools run over.
//!
//! TinyInference owns the wire contract; a host supplies the generators (the
//! endpoint, credential and policy that go with them) and hands them to the
//! tools built in [`super::tools`]. The core ships no generators of its own: a
//! host that has a media service registers a tool source that builds
//! [`MediaGenerators`] and calls
//! [`media_tools_for_config`](super::tools::media_tools_for_config).

use std::sync::Arc;

use tinyagents_harness::tinyinference_image::ImageGenerator;
use tinyagents_harness::tinyinference_video::VideoGenerator;

use crate::security::approval::ApprovalChatContext;

/// Rebuilds the generators so their egress disclosures reach `chat`.
pub type GeneratorRouter = Arc<
    dyn Fn(Option<ApprovalChatContext>) -> (Arc<dyn ImageGenerator>, Arc<dyn VideoGenerator>)
        + Send
        + Sync,
>;

/// The image and video generators for this process.
pub struct MediaGenerators {
    /// Image generation.
    pub image: Arc<dyn ImageGenerator>,
    /// Video generation.
    pub video: Arc<dyn VideoGenerator>,
    /// Binds the generators to a turn's chat so their egress disclosures reach
    /// it. `None` for generators with no disclosures to route (mocks).
    router: Option<GeneratorRouter>,
}

impl MediaGenerators {
    /// Generators whose egress disclosures can be bound to a turn's chat:
    /// `router` rebuilds the pair around a given chat.
    pub fn with_router(
        image: Arc<dyn ImageGenerator>,
        video: Arc<dyn VideoGenerator>,
        router: GeneratorRouter,
    ) -> Self {
        Self {
            image,
            video,
            router: Some(router),
        }
    }

    /// Generators with no egress routing.
    pub fn new(image: Arc<dyn ImageGenerator>, video: Arc<dyn VideoGenerator>) -> Self {
        Self {
            image,
            video,
            router: None,
        }
    }

    /// The generators for one call of a turn whose chat routing is `chat`: the
    /// egress disclosures they raise are surfaced to that chat. An explicit
    /// value, so a call made off the chat task still discloses to its own chat.
    pub fn routed(&self, chat: Option<&ApprovalChatContext>) -> MediaGenerators {
        match &self.router {
            Some(router) => {
                let (image, video) = router(chat.cloned());
                MediaGenerators {
                    image,
                    video,
                    router: self.router.clone(),
                }
            }
            None => MediaGenerators {
                image: Arc::clone(&self.image),
                video: Arc::clone(&self.video),
                router: None,
            },
        }
    }
}
