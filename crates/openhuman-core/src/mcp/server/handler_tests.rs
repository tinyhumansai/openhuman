use super::*;

#[test]
fn host_tool_errors_keep_their_category_and_diagnostic() {
    assert_eq!(
        reply_error(ToolCallError::InvalidParams(
            "argument was empty".to_owned()
        )),
        ServerHostReply::InvalidParams {
            message: "argument was empty".to_owned()
        }
    );
    assert_eq!(
        reply_error(ToolCallError::Internal("provider setup failed".to_owned())),
        ServerHostReply::Internal {
            message: "provider setup failed".to_owned()
        }
    );
    assert_eq!(
        reply_error(ToolCallError::ResourceNotFound("unknown uri".to_owned())),
        ServerHostReply::ResourceNotFound {
            message: "unknown uri".to_owned()
        }
    );
}
