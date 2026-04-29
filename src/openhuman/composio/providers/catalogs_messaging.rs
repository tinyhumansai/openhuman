//! Curated catalogs — messaging toolkits: Slack, Discord, Telegram,
//! WhatsApp, Microsoft Teams.

use super::tool_scope::{CuratedTool, ToolScope};

// ── slack ───────────────────────────────────────────────────────────
pub const SLACK_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "SLACK_FIND_CHANNELS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_FIND_USERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_FETCH_CONVERSATION_HISTORY",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_FETCH_MESSAGE_THREAD_FROM_A_CONVERSATION",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_LIST_ALL_CHANNELS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_LIST_ALL_USERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_LIST_CONVERSATIONS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_FETCH_TEAM_INFO",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_GET_USER_PRESENCE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_ASSISTANT_SEARCH_CONTEXT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SLACK_SEND_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_POST_MESSAGE_TO_CHANNEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_SEND_MESSAGE_TO_CHANNEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_CREATE_CHANNEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_INVITE_USERS_TO_A_SLACK_CHANNEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_ADD_REACTION_TO_AN_ITEM",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_UPLOAD_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_CREATE_A_REMINDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_CREATE_USER_GROUP",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SLACK_DELETE_CHANNEL",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_ARCHIVE_CONVERSATION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_DELETE_FILE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_DELETES_A_MESSAGE_FROM_A_CHAT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_DELETE_REMINDER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_LEAVE_CONVERSATION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_INVITE_USER_TO_WORKSPACE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SLACK_CONVERT_CHANNEL_TO_PRIVATE",
        scope: ToolScope::Admin,
    },
];

// ── discord ─────────────────────────────────────────────────────────
pub const DISCORD_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "DISCORD_GET_MY_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_GET_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_LIST_MY_GUILDS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_GET_MY_GUILD_MEMBER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_INVITE_RESOLVE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_GET_GUILD_WIDGET",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DISCORD_LIST_MY_CONNECTIONS",
        scope: ToolScope::Read,
    },
];

// ── telegram ────────────────────────────────────────────────────────
pub const TELEGRAM_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "TELEGRAM_GET_UPDATES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_CHAT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_CHAT_HISTORY",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_CHAT_MEMBER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_CHAT_MEMBERS_COUNT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_CHAT_ADMINISTRATORS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_GET_ME",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TELEGRAM_SEND_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_SEND_PHOTO",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_SEND_DOCUMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_SEND_LOCATION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_SEND_POLL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_FORWARD_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_EDIT_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_ANSWER_CALLBACK_QUERY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TELEGRAM_DELETE_MESSAGE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TELEGRAM_CREATE_CHAT_INVITE_LINK",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TELEGRAM_SET_MY_COMMANDS",
        scope: ToolScope::Admin,
    },
];

// ── whatsapp ────────────────────────────────────────────────────────
pub const WHATSAPP_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "WHATSAPP_GET_PHONE_NUMBERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_GET_MESSAGE_TEMPLATES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_GET_PHONE_NUMBER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_GET_BUSINESS_PROFILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_GET_TEMPLATE_STATUS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_GET_MEDIA_INFO",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_TEMPLATE_MESSAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_MEDIA",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_MEDIA_BY_ID",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_INTERACTIVE_BUTTONS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_SEND_INTERACTIVE_LIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_UPLOAD_MEDIA",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "WHATSAPP_CREATE_MESSAGE_TEMPLATE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "WHATSAPP_DELETE_MESSAGE_TEMPLATE",
        scope: ToolScope::Admin,
    },
];

// ── microsoft_teams ─────────────────────────────────────────────────
pub const MICROSOFT_TEAMS_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_CHAT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_CHANNEL",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_TEAM_FROM_GROUP",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CHATS_GET_ALL_CHATS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_PRESENCE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_ONLINE_MEETING",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_GET_SCHEDULE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CREATE_CHANNEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CREATE_TEAM",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CREATE_MEETING",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_ADD_TEAM_MEMBER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_ADD_CHAT_MEMBER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CREATE_SHIFT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_CREATE_TIME_OFF_REQUEST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_DELETE_TEAM",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_DELETE_CHANNEL",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_ARCHIVE_TEAM",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_ARCHIVE_CHANNEL",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_DELETE_TAB",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "MICROSOFT_TEAMS_DELETE_TIME_OFF",
        scope: ToolScope::Admin,
    },
];
