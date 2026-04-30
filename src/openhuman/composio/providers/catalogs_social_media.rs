//! Curated catalogs — social media / entertainment toolkits: Twitter,
//! Spotify, YouTube.

use super::tool_scope::{CuratedTool, ToolScope};

// ── twitter ─────────────────────────────────────────────────────────
pub const TWITTER_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "TWITTER_RECENT_SEARCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_GET_USER_BY_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_POST_LOOKUP_BY_POST_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_FOLLOWERS_BY_USER_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_FOLLOWING_BY_USER_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_BOOKMARKS_BY_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_GET_LIST_MEMBERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_FULL_ARCHIVE_SEARCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TWITTER_CREATION_OF_A_POST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_RETWEET_POST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_ADD_POST_TO_BOOKMARKS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_FOLLOW_USER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_MUTE_USER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_CREATE_DM_CONVERSATION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_CREATE_LIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_ADD_LIST_MEMBER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TWITTER_POST_DELETE_BY_POST_ID",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TWITTER_DELETE_LIST",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TWITTER_REMOVE_LIST_MEMBER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TWITTER_DELETE_DM",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TWITTER_REMOVE_POST_FROM_BOOKMARKS",
        scope: ToolScope::Admin,
    },
];

// ── spotify ─────────────────────────────────────────────────────────
pub const SPOTIFY_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "SPOTIFY_GET_CURRENT_USER_S_PROFILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_USER_S_TOP_TRACKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_PLAYLIST",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_PLAYLIST_ITEMS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_RECENTLY_PLAYED_TRACKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_USER_S_SAVED_TRACKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_SEARCH_FOR_ITEM",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_GET_AVAILABLE_DEVICES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SPOTIFY_ADD_ITEMS_TO_PLAYLIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_CREATE_PLAYLIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_SAVE_TRACKS_FOR_CURRENT_USER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_PAUSE_PLAYBACK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_ADD_ITEM_TO_PLAYBACK_QUEUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_CHANGE_PLAYLIST_DETAILS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SPOTIFY_REMOVE_PLAYLIST_ITEMS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SPOTIFY_REMOVE_USER_S_SAVED_TRACKS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SPOTIFY_UNFOLLOW_ARTISTS_OR_USERS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SPOTIFY_REMOVE_USER_S_SAVED_ALBUMS",
        scope: ToolScope::Admin,
    },
];

// ── youtube ─────────────────────────────────────────────────────────
pub const YOUTUBE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "YOUTUBE_SEARCH_YOU_TUBE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_LIST_CHANNEL_VIDEOS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_GET_CHANNEL_STATISTICS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_LIST_COMMENT_THREADS2",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_LIST_COMMENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_GET_VIDEO_DETAILS_BATCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_LIST_USER_PLAYLISTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_LIST_PLAYLIST_ITEMS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "YOUTUBE_UPLOAD_VIDEO",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_UPDATE_VIDEO",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_CREATE_PLAYLIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_ADD_VIDEO_TO_PLAYLIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_POST_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_RATE_VIDEO",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_UPDATE_PLAYLIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "YOUTUBE_DELETE_VIDEO",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "YOUTUBE_DELETE_PLAYLIST",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "YOUTUBE_DELETE_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "YOUTUBE_DELETE_PLAYLIST_ITEM",
        scope: ToolScope::Admin,
    },
];
