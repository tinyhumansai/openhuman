//! Media helpers — MIME mapping, byte-level image dimension parsing,
//! download with size cap, and TIM `msg_body` builders.
//!
//! Tencent COS upload lives in [`super::cos`] to keep both files under
//! the 500-line ceiling.

use super::errors::YuanbaoError;
use super::types::MsgBodyElement;

// ─── MIME / image-format mapping ───────────────────────────────────

pub fn guess_mime_type(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" => "text/plain",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

pub fn is_image(filename: &str, mime_type: &str) -> bool {
    if mime_type.starts_with("image/") {
        return true;
    }
    guess_mime_type(filename).starts_with("image/")
}

/// Map a MIME type to the TIM `image_format` enum.
pub fn image_format_code(mime: &str) -> u32 {
    match mime {
        "image/jpeg" | "image/jpg" => 1,
        "image/gif" => 2,
        "image/png" => 3,
        "image/bmp" => 4,
        "image/webp" | "image/heic" | "image/tiff" => 255,
        _ => 255,
    }
}

// ─── Pure-bytes image dimension parsing ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDims {
    pub width: u32,
    pub height: u32,
}

pub fn parse_image_size(data: &[u8]) -> Option<ImageDims> {
    parse_png(data)
        .or_else(|| parse_jpeg(data))
        .or_else(|| parse_gif(data))
        .or_else(|| parse_webp(data))
}

fn parse_png(buf: &[u8]) -> Option<ImageDims> {
    if buf.len() < 24 || &buf[..4] != b"\x89PNG" {
        return None;
    }
    let w = u32::from_be_bytes(buf[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(buf[20..24].try_into().ok()?);
    Some(ImageDims {
        width: w,
        height: h,
    })
}

fn parse_jpeg(buf: &[u8]) -> Option<ImageDims> {
    if buf.len() < 4 || buf[0] != 0xFF || buf[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 9 < buf.len() {
        if buf[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = buf[i + 1];
        if marker == 0xC0 || marker == 0xC2 {
            let h = u16::from_be_bytes(buf[i + 5..i + 7].try_into().ok()?);
            let w = u16::from_be_bytes(buf[i + 7..i + 9].try_into().ok()?);
            return Some(ImageDims {
                width: w as u32,
                height: h as u32,
            });
        }
        if i + 3 >= buf.len() {
            break;
        }
        let seg_len = u16::from_be_bytes(buf[i + 2..i + 4].try_into().ok()?) as usize;
        i += 2 + seg_len;
    }
    None
}

fn parse_gif(buf: &[u8]) -> Option<ImageDims> {
    if buf.len() < 10 {
        return None;
    }
    let sig = &buf[..6];
    if sig != b"GIF87a" && sig != b"GIF89a" {
        return None;
    }
    let w = u16::from_le_bytes(buf[6..8].try_into().ok()?);
    let h = u16::from_le_bytes(buf[8..10].try_into().ok()?);
    Some(ImageDims {
        width: w as u32,
        height: h as u32,
    })
}

fn parse_webp(buf: &[u8]) -> Option<ImageDims> {
    if buf.len() < 16 || &buf[..4] != b"RIFF" || &buf[8..12] != b"WEBP" {
        return None;
    }
    let chunk = &buf[12..16];
    if chunk == b"VP8 " {
        if buf.len() >= 30 && buf[23] == 0x9D && buf[24] == 0x01 && buf[25] == 0x2A {
            let w = u16::from_le_bytes(buf[26..28].try_into().ok()?) & 0x3FFF;
            let h = u16::from_le_bytes(buf[28..30].try_into().ok()?) & 0x3FFF;
            return Some(ImageDims {
                width: w as u32,
                height: h as u32,
            });
        }
    } else if chunk == b"VP8L" {
        if buf.len() >= 25 && buf[20] == 0x2F {
            let bits = u32::from_le_bytes(buf[21..25].try_into().ok()?);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            return Some(ImageDims {
                width: w,
                height: h,
            });
        }
    } else if chunk == b"VP8X" && buf.len() >= 30 {
        let w = (buf[24] as u32 | ((buf[25] as u32) << 8) | ((buf[26] as u32) << 16)) + 1;
        let h = (buf[27] as u32 | ((buf[28] as u32) << 8) | ((buf[29] as u32) << 16)) + 1;
        return Some(ImageDims {
            width: w,
            height: h,
        });
    }
    None
}

// ─── HTTP download with size cap ────────────────────────────────────

pub async fn download_url(
    http: &reqwest::Client,
    url: &str,
    max_size_mb: u64,
) -> Result<(Vec<u8>, String), YuanbaoError> {
    let limit = max_size_mb.saturating_mul(1024 * 1024);

    if let Ok(head) = http.head(url).send().await {
        if let Some(len) = head
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            if len > limit {
                return Err(YuanbaoError::Media(format!(
                    "remote file too large: {len} > limit {limit}"
                )));
            }
        }
    }

    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| YuanbaoError::Connection(format!("download {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(YuanbaoError::Media(format!(
            "download HTTP {} for {url}",
            resp.status()
        )));
    }
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| YuanbaoError::Media(format!("read body: {e}")))?;
    if bytes.len() as u64 > limit {
        return Err(YuanbaoError::Media(format!(
            "downloaded file exceeds limit: {} > {}",
            bytes.len(),
            limit
        )));
    }
    Ok((bytes.to_vec(), ct))
}

// ─── TIM msg_body builders ──────────────────────────────────────────

/// Build a TIM `TIMImageElem` `msg_body` ready to send.
pub fn build_image_msg_body(
    url: &str,
    uuid: Option<&str>,
    filename: Option<&str>,
    size: u32,
    width: u32,
    height: u32,
    mime_type: &str,
) -> Vec<MsgBodyElement> {
    use super::types::{ImageInfo, MsgContent};
    let uuid_str = uuid
        .map(|s| s.to_string())
        .or_else(|| filename.map(|s| s.to_string()))
        .unwrap_or_else(|| "image".to_string());
    let format = if mime_type.is_empty() {
        255
    } else {
        image_format_code(mime_type)
    };
    vec![MsgBodyElement {
        msg_type: "TIMImageElem".into(),
        msg_content: MsgContent {
            uuid: Some(uuid_str),
            image_format: Some(format),
            image_info_array: vec![ImageInfo {
                image_type: 1,
                size,
                width,
                height,
                url: url.to_string(),
            }],
            ..Default::default()
        },
    }]
}

/// Build a TIM `TIMFileElem` `msg_body` ready to send.
pub fn build_file_msg_body(
    url: &str,
    filename: &str,
    uuid: Option<&str>,
    size: u32,
) -> Vec<MsgBodyElement> {
    use super::types::MsgContent;
    let uuid_str = uuid
        .map(|s| s.to_string())
        .unwrap_or_else(|| filename.to_string());
    vec![MsgBodyElement {
        msg_type: "TIMFileElem".into(),
        msg_content: MsgContent {
            uuid: Some(uuid_str),
            file_name: Some(filename.to_string()),
            file_size: Some(size),
            url: Some(url.to_string()),
            ..Default::default()
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_dims_parse() {
        let png = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
        ];
        let d = parse_image_size(&png).expect("png parse");
        assert_eq!(d.width, 1);
        assert_eq!(d.height, 1);
    }

    #[test]
    fn gif_dims_parse() {
        let gif = b"GIF89a\x40\x01\xF0\x00rest";
        let d = parse_image_size(gif).expect("gif parse");
        assert_eq!(d.width, 320);
        assert_eq!(d.height, 240);
    }

    #[test]
    fn guess_mime_basic() {
        assert_eq!(guess_mime_type("foo.png"), "image/png");
        assert_eq!(guess_mime_type("doc.pdf"), "application/pdf");
        assert_eq!(guess_mime_type("blob"), "application/octet-stream");
    }

    #[test]
    fn is_image_works() {
        assert!(is_image("a.jpeg", ""));
        assert!(is_image("noext", "image/png"));
        assert!(!is_image("a.pdf", ""));
    }

    #[test]
    fn image_format_code_matrix() {
        assert_eq!(image_format_code("image/png"), 3);
        assert_eq!(image_format_code("image/jpeg"), 1);
        assert_eq!(image_format_code("image/gif"), 2);
        assert_eq!(image_format_code("image/bmp"), 4);
        assert_eq!(image_format_code("image/webp"), 255);
        assert_eq!(image_format_code("application/pdf"), 255);
    }

    // ─── extended MIME / image-format tests ─────────────────────

    #[test]
    fn guess_mime_handles_uppercase_extension() {
        assert_eq!(guess_mime_type("PHOTO.JPG"), "image/jpeg");
        assert_eq!(guess_mime_type("Doc.PDF"), "application/pdf");
    }

    #[test]
    fn guess_mime_covers_office_audio_video_archive_types() {
        assert_eq!(guess_mime_type("file.jpg"), "image/jpeg");
        assert_eq!(guess_mime_type("file.jpeg"), "image/jpeg");
        assert_eq!(guess_mime_type("file.gif"), "image/gif");
        assert_eq!(guess_mime_type("file.webp"), "image/webp");
        assert_eq!(guess_mime_type("file.bmp"), "image/bmp");
        assert_eq!(guess_mime_type("file.heic"), "image/heic");
        assert_eq!(guess_mime_type("file.tiff"), "image/tiff");
        assert_eq!(guess_mime_type("file.ico"), "image/x-icon");
        assert_eq!(guess_mime_type("file.doc"), "application/msword");
        assert_eq!(
            guess_mime_type("file.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(guess_mime_type("file.xls"), "application/vnd.ms-excel");
        assert_eq!(
            guess_mime_type("file.xlsx"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(guess_mime_type("file.ppt"), "application/vnd.ms-powerpoint");
        assert_eq!(
            guess_mime_type("file.pptx"),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert_eq!(guess_mime_type("file.txt"), "text/plain");
        assert_eq!(guess_mime_type("file.zip"), "application/zip");
        assert_eq!(guess_mime_type("file.tar"), "application/x-tar");
        assert_eq!(guess_mime_type("file.gz"), "application/gzip");
        assert_eq!(guess_mime_type("file.mp3"), "audio/mpeg");
        assert_eq!(guess_mime_type("file.mp4"), "video/mp4");
        assert_eq!(guess_mime_type("file.wav"), "audio/wav");
        assert_eq!(guess_mime_type("file.ogg"), "audio/ogg");
        assert_eq!(guess_mime_type("file.webm"), "video/webm");
    }

    #[test]
    fn image_format_code_jpg_alias_and_heic_tiff() {
        assert_eq!(image_format_code("image/jpg"), 1);
        assert_eq!(image_format_code("image/heic"), 255);
        assert_eq!(image_format_code("image/tiff"), 255);
        assert_eq!(image_format_code(""), 255);
    }

    // ─── parse_image_size — JPEG / WEBP / negative paths ────────

    #[test]
    fn jpeg_dims_from_sof0_marker() {
        // SOI + filler APP0 segment + SOF0 marker carrying h=2, w=3.
        // parse_jpeg's read uses buf[i+5..i+9] and gates on `i + 9 < buf.len()`,
        // so trailing pad bytes are required (one tail byte makes 17 < 18 true).
        let mut jpeg = vec![0xFF, 0xD8];
        // APP0 (0xFFE0) with len=4 → 2 bytes payload
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        // SOF0: 0xFF 0xC0  len(11)  precision(8)  height(2 BE)  width(3 BE)
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x03]);
        // trailing pad so the `i + 9 < buf.len()` loop guard accepts the SOF0
        // entry on the second iteration.
        jpeg.push(0xFF);
        let d = parse_image_size(&jpeg).expect("jpeg parse");
        assert_eq!(d.width, 3);
        assert_eq!(d.height, 2);
    }

    #[test]
    fn jpeg_too_short_returns_none() {
        assert!(parse_image_size(&[0xFF, 0xD8]).is_none());
    }

    #[test]
    fn jpeg_wrong_magic_returns_none() {
        let buf = [0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 0, 0, 0];
        assert!(parse_image_size(&buf).is_none());
    }

    #[test]
    fn webp_vp8x_dims_parse() {
        // RIFF size WEBP VP8X flags(4) padding(3) w-1(LE24) h-1(LE24)
        // w=320 → 319 little-endian = [0x3F, 0x01, 0x00]; h=240 → 239 = [0xEF, 0x00, 0x00]
        let mut buf = b"RIFF\x00\x00\x00\x00WEBPVP8X".to_vec();
        buf.extend_from_slice(&[0u8; 8]); // flags + reserved
        buf.extend_from_slice(&[0x3F, 0x01, 0x00, 0xEF, 0x00, 0x00]);
        let d = parse_image_size(&buf).expect("webp vp8x parse");
        assert_eq!(d.width, 320);
        assert_eq!(d.height, 240);
    }

    #[test]
    fn webp_too_short_returns_none() {
        let buf = b"RIFF\0\0\0\0WEBPVP8";
        assert!(parse_image_size(buf).is_none());
    }

    #[test]
    fn webp_unsupported_chunk_returns_none() {
        let mut buf = b"RIFF\0\0\0\0WEBPXXXX".to_vec();
        buf.extend_from_slice(&[0u8; 30]);
        assert!(parse_image_size(&buf).is_none());
    }

    #[test]
    fn png_short_or_wrong_magic_returns_none() {
        assert!(parse_image_size(&[0x89, 0x50, 0x4E, 0x47]).is_none()); // too short
        let mut buf = vec![0xFF; 24];
        buf[..4].copy_from_slice(&[0x89, 0x50, 0x4F, 0x47]); // wrong magic
        assert!(parse_image_size(&buf).is_none());
    }

    #[test]
    fn gif_too_short_or_wrong_sig_returns_none() {
        assert!(parse_image_size(b"GIF87").is_none());
        assert!(parse_image_size(b"NOTGIFEXT").is_none());
    }

    #[test]
    fn parse_image_size_empty_returns_none() {
        assert!(parse_image_size(&[]).is_none());
    }

    // ─── msg_body builders ──────────────────────────────────────

    #[test]
    fn build_image_msg_body_uses_uuid_when_present() {
        let body = build_image_msg_body(
            "https://x/cat.png",
            Some("uuid-1"),
            Some("cat.png"),
            1024,
            800,
            600,
            "image/png",
        );
        assert_eq!(body.len(), 1);
        let el = &body[0];
        assert_eq!(el.msg_type, "TIMImageElem");
        assert_eq!(el.msg_content.uuid.as_deref(), Some("uuid-1"));
        assert_eq!(el.msg_content.image_format, Some(3)); // png
        assert_eq!(el.msg_content.image_info_array.len(), 1);
        let info = &el.msg_content.image_info_array[0];
        assert_eq!(info.image_type, 1);
        assert_eq!(info.size, 1024);
        assert_eq!(info.width, 800);
        assert_eq!(info.height, 600);
        assert_eq!(info.url, "https://x/cat.png");
    }

    #[test]
    fn build_image_msg_body_falls_back_to_filename_then_default_uuid() {
        let with_filename = build_image_msg_body(
            "https://x/",
            None,
            Some("only-name.png"),
            0,
            0,
            0,
            "image/png",
        );
        assert_eq!(
            with_filename[0].msg_content.uuid.as_deref(),
            Some("only-name.png")
        );

        let default_id = build_image_msg_body("https://x/", None, None, 0, 0, 0, "image/png");
        assert_eq!(default_id[0].msg_content.uuid.as_deref(), Some("image"));
    }

    #[test]
    fn build_image_msg_body_treats_empty_mime_as_format_255() {
        let body = build_image_msg_body("https://x/cat.jpg", None, None, 0, 0, 0, "");
        assert_eq!(body[0].msg_content.image_format, Some(255));
    }

    #[test]
    fn build_file_msg_body_uses_filename_when_uuid_missing() {
        let body = build_file_msg_body("https://x/doc.pdf", "doc.pdf", None, 2048);
        assert_eq!(body.len(), 1);
        let el = &body[0];
        assert_eq!(el.msg_type, "TIMFileElem");
        assert_eq!(el.msg_content.uuid.as_deref(), Some("doc.pdf"));
        assert_eq!(el.msg_content.file_name.as_deref(), Some("doc.pdf"));
        assert_eq!(el.msg_content.file_size, Some(2048));
        assert_eq!(el.msg_content.url.as_deref(), Some("https://x/doc.pdf"));
    }

    #[test]
    fn build_file_msg_body_prefers_explicit_uuid() {
        let body = build_file_msg_body("https://x/y.pdf", "y.pdf", Some("uuid-y"), 0);
        assert_eq!(body[0].msg_content.uuid.as_deref(), Some("uuid-y"));
    }

    // ─── download_url (wiremock) ────────────────────────────────

    #[tokio::test]
    async fn download_url_returns_bytes_and_content_type() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .respond_with(wiremock::ResponseTemplate::new(200).insert_header("Content-Length", "3"))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/png; charset=binary")
                    .set_body_bytes(vec![1u8, 2, 3]),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let (bytes, mime) = download_url(&http, &server.uri(), 10).await.unwrap();
        assert_eq!(bytes, vec![1, 2, 3]);
        assert_eq!(mime, "image/png");
    }

    #[tokio::test]
    async fn download_url_rejects_oversize_from_head_content_length() {
        let server = wiremock::MockServer::start().await;
        // HEAD reports a very large file → reject BEFORE GET.
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .respond_with(wiremock::ResponseTemplate::new(200).insert_header(
                "Content-Length",
                (10u64 * 1024 * 1024 + 1).to_string().as_str(),
            ))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = download_url(&http, &server.uri(), 10).await.unwrap_err();
        match err {
            YuanbaoError::Media(m) => assert!(m.contains("too large"), "got {m}"),
            other => panic!("expected Media error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn download_url_rejects_when_body_exceeds_limit() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2 * 1024 * 1024]), // 2 MiB
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = download_url(&http, &server.uri(), 1).await.unwrap_err();
        match err {
            YuanbaoError::Media(m) => assert!(m.contains("exceeds limit"), "got {m}"),
            other => panic!("expected Media error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn download_url_surfaces_http_error_status() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = download_url(&http, &server.uri(), 10).await.unwrap_err();
        match err {
            YuanbaoError::Media(m) => assert!(m.contains("HTTP 404"), "got {m}"),
            other => panic!("expected Media error, got {other:?}"),
        }
    }
}
