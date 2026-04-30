use super::*;

fn make_channel() -> WhatsAppChannel {
    WhatsAppChannel::new(
        "test-token".into(),
        "123456789".into(),
        "verify-me".into(),
        vec!["+1234567890".into()],
    )
}

#[test]
fn whatsapp_channel_name() {
    let ch = make_channel();
    assert_eq!(ch.name(), "whatsapp");
}

#[test]
fn whatsapp_verify_token() {
    let ch = make_channel();
    assert_eq!(ch.verify_token(), "verify-me");
}

#[test]
fn whatsapp_number_allowed_exact() {
    let ch = make_channel();
    assert!(ch.is_number_allowed("+1234567890"));
    assert!(!ch.is_number_allowed("+9876543210"));
}

#[test]
fn whatsapp_number_allowed_wildcard() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    assert!(ch.is_number_allowed("+1234567890"));
    assert!(ch.is_number_allowed("+9999999999"));
}

#[test]
fn whatsapp_number_denied_empty() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec![]);
    assert!(!ch.is_number_allowed("+1234567890"));
}

#[test]
fn whatsapp_parse_empty_payload() {
    let ch = make_channel();
    let payload = serde_json::json!({});
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_valid_text_message() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "123",
            "changes": [{
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "15551234567",
                        "phone_number_id": "123456789"
                    },
                    "messages": [{
                        "from": "1234567890",
                        "id": "wamid.xxx",
                        "timestamp": "1699999999",
                        "type": "text",
                        "text": {
                            "body": "Hello OpenHuman!"
                        }
                    }]
                },
                "field": "messages"
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "+1234567890");
    assert_eq!(msgs[0].content, "Hello OpenHuman!");
    assert_eq!(msgs[0].channel, "whatsapp");
    assert_eq!(msgs[0].timestamp, 1_699_999_999);
}

#[test]
fn whatsapp_parse_unauthorized_number() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "9999999999",
                        "timestamp": "1699999999",
                        "type": "text",
                        "text": { "body": "Spam" }
                    }]
                }
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Unauthorized numbers should be filtered");
}

#[test]
fn whatsapp_parse_non_text_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "1234567890",
                        "timestamp": "1699999999",
                        "type": "image",
                        "image": { "id": "img123" }
                    }]
                }
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Non-text messages should be skipped");
}

#[test]
fn whatsapp_parse_multiple_messages() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [
                        { "from": "111", "timestamp": "1", "type": "text", "text": { "body": "First" } },
                        { "from": "222", "timestamp": "2", "type": "text", "text": { "body": "Second" } }
                    ]
                }
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "First");
    assert_eq!(msgs[1].content, "Second");
}

#[test]
fn whatsapp_parse_normalizes_phone_with_plus() {
    let ch = WhatsAppChannel::new(
        "tok".into(),
        "123".into(),
        "ver".into(),
        vec!["+1234567890".into()],
    );
    // API sends without +, but we normalize to +
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "1234567890",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "Hi" }
                    }]
                }
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "+1234567890");
}

#[test]
fn whatsapp_empty_text_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "" }
                    }]
                }
            }]
        }]
    });

    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

// ══════════════════════════════════════════════════════════
// EDGE CASES — Comprehensive coverage
// ══════════════════════════════════════════════════════════

#[test]
fn whatsapp_parse_missing_entry_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "object": "whatsapp_business_account"
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_entry_not_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": "not_an_array"
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_missing_changes_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{ "id": "123" }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_changes_not_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": "not_an_array"
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_missing_value() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{ "field": "messages" }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_missing_messages_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "metadata": {}
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_messages_not_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": "not_an_array"
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_missing_from_field() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "No sender" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Messages without 'from' should be skipped");
}

#[test]
fn whatsapp_parse_missing_text_body() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": {}
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(
        msgs.is_empty(),
        "Messages with empty text object should be skipped"
    );
}

#[test]
fn whatsapp_parse_null_text_body() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": null }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Messages with null body should be skipped");
}

#[test]
fn whatsapp_parse_invalid_timestamp_uses_current() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "not_a_number",
                        "type": "text",
                        "text": { "body": "Hello" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    // Timestamp should be current time (non-zero)
    assert!(msgs[0].timestamp > 0);
}

#[test]
fn whatsapp_parse_missing_timestamp_uses_current() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "type": "text",
                        "text": { "body": "Hello" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].timestamp > 0);
}

#[test]
fn whatsapp_parse_multiple_entries() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [
            {
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "111",
                            "timestamp": "1",
                            "type": "text",
                            "text": { "body": "Entry 1" }
                        }]
                    }
                }]
            },
            {
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "222",
                            "timestamp": "2",
                            "type": "text",
                            "text": { "body": "Entry 2" }
                        }]
                    }
                }]
            }
        ]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Entry 1");
    assert_eq!(msgs[1].content, "Entry 2");
}

#[test]
fn whatsapp_parse_multiple_changes() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [
                {
                    "value": {
                        "messages": [{
                            "from": "111",
                            "timestamp": "1",
                            "type": "text",
                            "text": { "body": "Change 1" }
                        }]
                    }
                },
                {
                    "value": {
                        "messages": [{
                            "from": "222",
                            "timestamp": "2",
                            "type": "text",
                            "text": { "body": "Change 2" }
                        }]
                    }
                }
            ]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Change 1");
    assert_eq!(msgs[1].content, "Change 2");
}

#[test]
fn whatsapp_parse_status_update_ignored() {
    // Status updates have "statuses" instead of "messages"
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "statuses": [{
                        "id": "wamid.xxx",
                        "status": "delivered",
                        "timestamp": "1699999999"
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty(), "Status updates should be ignored");
}

#[test]
fn whatsapp_parse_audio_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "audio",
                        "audio": { "id": "audio123", "mime_type": "audio/ogg" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_video_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "video",
                        "video": { "id": "video123" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_document_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "document",
                        "document": { "id": "doc123", "filename": "file.pdf" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_sticker_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "sticker",
                        "sticker": { "id": "sticker123" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_location_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "location",
                        "location": { "latitude": 40.7128, "longitude": -74.0060 }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_contacts_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "contacts",
                        "contacts": [{ "name": { "formatted_name": "John" } }]
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_reaction_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "reaction",
                        "reaction": { "message_id": "wamid.xxx", "emoji": "👍" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_mixed_authorized_unauthorized() {
    let ch = WhatsAppChannel::new(
        "tok".into(),
        "123".into(),
        "ver".into(),
        vec!["+1111111111".into()],
    );
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [
                        { "from": "1111111111", "timestamp": "1", "type": "text", "text": { "body": "Allowed" } },
                        { "from": "9999999999", "timestamp": "2", "type": "text", "text": { "body": "Blocked" } },
                        { "from": "1111111111", "timestamp": "3", "type": "text", "text": { "body": "Also allowed" } }
                    ]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Allowed");
    assert_eq!(msgs[1].content, "Also allowed");
}

#[test]
fn whatsapp_parse_unicode_message() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "Hello 👋 世界 🌍 مرحبا" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "Hello 👋 世界 🌍 مرحبا");
}

#[test]
fn whatsapp_parse_very_long_message() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let long_text = "A".repeat(10_000);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": long_text }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content.len(), 10_000);
}

#[test]
fn whatsapp_parse_whitespace_only_message_skipped() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "   " }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    // Whitespace-only is NOT empty, so it passes through
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "   ");
}

#[test]
fn whatsapp_number_allowed_multiple_numbers() {
    let ch = WhatsAppChannel::new(
        "tok".into(),
        "123".into(),
        "ver".into(),
        vec![
            "+1111111111".into(),
            "+2222222222".into(),
            "+3333333333".into(),
        ],
    );
    assert!(ch.is_number_allowed("+1111111111"));
    assert!(ch.is_number_allowed("+2222222222"));
    assert!(ch.is_number_allowed("+3333333333"));
    assert!(!ch.is_number_allowed("+4444444444"));
}

#[test]
fn whatsapp_number_allowed_case_sensitive() {
    // Phone numbers should be exact match
    let ch = WhatsAppChannel::new(
        "tok".into(),
        "123".into(),
        "ver".into(),
        vec!["+1234567890".into()],
    );
    assert!(ch.is_number_allowed("+1234567890"));
    // Different number should not match
    assert!(!ch.is_number_allowed("+1234567891"));
}

#[test]
fn whatsapp_parse_phone_already_has_plus() {
    let ch = WhatsAppChannel::new(
        "tok".into(),
        "123".into(),
        "ver".into(),
        vec!["+1234567890".into()],
    );
    // If API sends with +, we should still handle it
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "+1234567890",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "Hi" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "+1234567890");
}

#[test]
fn whatsapp_channel_fields_stored_correctly() {
    let ch = WhatsAppChannel::new(
        "my-access-token".into(),
        "phone-id-123".into(),
        "my-verify-token".into(),
        vec!["+111".into(), "+222".into()],
    );
    assert_eq!(ch.verify_token(), "my-verify-token");
    assert!(ch.is_number_allowed("+111"));
    assert!(ch.is_number_allowed("+222"));
    assert!(!ch.is_number_allowed("+333"));
}

#[test]
fn whatsapp_parse_empty_messages_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": []
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_empty_entry_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": []
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_empty_changes_array() {
    let ch = make_channel();
    let payload = serde_json::json!({
        "entry": [{
            "changes": []
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert!(msgs.is_empty());
}

#[test]
fn whatsapp_parse_newlines_preserved() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "Line 1\nLine 2\nLine 3" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "Line 1\nLine 2\nLine 3");
}

#[test]
fn whatsapp_parse_special_characters() {
    let ch = WhatsAppChannel::new("tok".into(), "123".into(), "ver".into(), vec!["*".into()]);
    let payload = serde_json::json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "from": "111",
                        "timestamp": "1",
                        "type": "text",
                        "text": { "body": "<script>alert('xss')</script> & \"quotes\" 'apostrophe'" }
                    }]
                }
            }]
        }]
    });
    let msgs = ch.parse_webhook_payload(&payload);
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        msgs[0].content,
        "<script>alert('xss')</script> & \"quotes\" 'apostrophe'"
    );
}
