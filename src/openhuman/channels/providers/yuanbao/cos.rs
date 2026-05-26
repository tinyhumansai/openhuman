//! Tencent COS upload — HMAC-SHA1 signing and `genUploadInfo` flow.
//!
//! Split out of `media.rs` to stay under the 500-line per-file ceiling.
//! Reference: <https://cloud.tencent.com/document/product/436/7778>.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};
use tracing::{debug, info};

use super::errors::YuanbaoError;
use super::media::{guess_mime_type, is_image, parse_image_size, ImageDims};

const UPLOAD_INFO_PATH: &str = "/api/resource/genUploadInfo";
const COS_USE_ACCELERATE: bool = true;

type HmacSha1 = Hmac<Sha1>;

fn hmac_sha1_hex(key: &[u8], msg: &[u8]) -> String {
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    hex::encode(mac.finalize().into_bytes())
}

fn sha1_hex(msg: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(msg);
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone)]
pub struct CosSignInput<'a> {
    pub method: &'a str,
    /// URL-encoded path with leading `/`.
    pub path: &'a str,
    pub params: &'a [(&'a str, &'a str)],
    pub headers: &'a [(&'a str, &'a str)],
    pub secret_id: &'a str,
    pub secret_key: &'a str,
    pub start_time: u64,
    pub expire_seconds: u64,
}

/// Build the COS `Authorization` header value.
pub fn cos_sign(input: &CosSignInput<'_>) -> String {
    let q_sign_time = format!(
        "{};{}",
        input.start_time,
        input.start_time + input.expire_seconds
    );

    // Step 1 — SignKey = HMAC-SHA1(SecretKey, q-sign-time).
    let sign_key = hmac_sha1_hex(input.secret_key.as_bytes(), q_sign_time.as_bytes());

    // Step 2 — HttpString. Names lower-cased, values URL-encoded.
    let mut params: Vec<(String, String)> = input
        .params
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), url_encode(v)))
        .collect();
    params.sort();
    let mut headers: Vec<(String, String)> = input
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), url_encode(v)))
        .collect();
    headers.sort();

    let url_param_list = params
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let url_params = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let header_list = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let header_str = headers
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let http_string = format!(
        "{}\n{}\n{}\n{}\n",
        input.method.to_ascii_lowercase(),
        input.path,
        url_params,
        header_str
    );

    // Step 3 — StringToSign.
    let sha1_of_http = sha1_hex(http_string.as_bytes());
    let string_to_sign = format!("sha1\n{q_sign_time}\n{sha1_of_http}\n");

    // Step 4 — Signature.
    let signature = hmac_sha1_hex(sign_key.as_bytes(), string_to_sign.as_bytes());

    format!(
        "q-sign-algorithm=sha1&q-ak={sid}&q-sign-time={t}&q-key-time={t}\
         &q-header-list={hl}&q-url-param-list={pl}&q-signature={sig}",
        sid = input.secret_id,
        t = q_sign_time,
        hl = header_list,
        pl = url_param_list,
        sig = signature
    )
}

fn url_encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn encode_cos_key(key: &str) -> String {
    key.split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Clone, Default)]
pub struct CosCredentials {
    pub bucket: String,
    pub region: String,
    pub location: String,
    pub secret_id: String,
    pub secret_key: String,
    pub session_token: String,
    pub start_time: u64,
    pub expired_time: u64,
    pub resource_url: String,
}

#[derive(Debug, Clone)]
pub struct UploadResult {
    pub url: String,
    pub uuid: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

/// Fetch COS upload credentials from the yuanbao gateway.
pub async fn get_cos_credentials(
    http: &reqwest::Client,
    api_domain: &str,
    app_key: &str,
    bot_id: &str,
    token: &str,
    route_env: &str,
    filename: &str,
) -> Result<CosCredentials, YuanbaoError> {
    let upload_url = format!(
        "{}/{}",
        api_domain.trim_end_matches('/'),
        UPLOAD_INFO_PATH.trim_start_matches('/')
    );
    let body = serde_json::json!({
        "fileName": filename,
        "fileId": uuid::Uuid::new_v4().simple().to_string(),
        "docFrom": "localDoc",
        "docOpenId": "",
    });
    let mut req = http
        .post(&upload_url)
        .header("Content-Type", "application/json")
        .header("X-Token", token)
        .header("X-ID", if bot_id.is_empty() { app_key } else { bot_id })
        .header("X-Source", "web");
    if !route_env.is_empty() {
        req = req.header("X-Route-Env", route_env);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| YuanbaoError::Connection(format!("genUploadInfo: {e}")))?;
    if !resp.status().is_success() {
        return Err(YuanbaoError::Media(format!(
            "genUploadInfo HTTP {}",
            resp.status()
        )));
    }
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| YuanbaoError::Media(format!("genUploadInfo body parse: {e}")))?;
    if let Some(code) = payload.get("code").and_then(|c| c.as_i64()) {
        if code != 0 {
            return Err(YuanbaoError::Media(format!(
                "genUploadInfo code={code}, msg={}",
                payload.get("msg").and_then(|m| m.as_str()).unwrap_or("")
            )));
        }
    }
    let data = payload.get("data").unwrap_or(&payload);
    let get_str = |k: &str| -> String {
        data.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_u64 = |k: &str| -> u64 { data.get(k).and_then(|v| v.as_u64()).unwrap_or(0) };

    Ok(CosCredentials {
        bucket: get_str("bucketName"),
        region: get_str("region"),
        location: get_str("location"),
        secret_id: get_str("encryptTmpSecretId"),
        secret_key: get_str("encryptTmpSecretKey"),
        session_token: get_str("encryptToken"),
        start_time: get_u64("startTime"),
        expired_time: get_u64("expiredTime"),
        resource_url: get_str("resourceUrl"),
    })
}

/// PUT a file to COS using credentials returned by `get_cos_credentials`.
pub async fn upload_to_cos(
    http: &reqwest::Client,
    creds: &CosCredentials,
    data: &[u8],
    filename: &str,
    mut content_type: String,
) -> Result<UploadResult, YuanbaoError> {
    if creds.secret_id.is_empty() || creds.secret_key.is_empty() || creds.location.is_empty() {
        return Err(YuanbaoError::Media(
            "COS credentials missing secret_id / secret_key / location".into(),
        ));
    }
    if content_type.is_empty() || content_type == "application/octet-stream" {
        content_type = if is_image(filename, "") {
            guess_mime_type(filename).to_string()
        } else {
            "application/octet-stream".into()
        };
    }

    let cos_host = if COS_USE_ACCELERATE {
        format!("{}.cos.accelerate.myqcloud.com", creds.bucket)
    } else {
        format!("{}.cos.{}.myqcloud.com", creds.bucket, creds.region)
    };
    let encoded_key = encode_cos_key(&creds.location);
    let cos_url = format!("https://{cos_host}/{}", encoded_key.trim_start_matches('/'));

    let now = unix_now();
    let start = if creds.start_time != 0 {
        creds.start_time
    } else {
        now
    };
    let expire = if creds.expired_time > now {
        creds.expired_time - now
    } else {
        3600
    };

    let headers_for_sign: Vec<(&str, &str)> = vec![
        ("host", cos_host.as_str()),
        ("content-type", content_type.as_str()),
        ("x-cos-security-token", creds.session_token.as_str()),
    ];
    let path = format!("/{}", encoded_key.trim_start_matches('/'));
    let sig = cos_sign(&CosSignInput {
        method: "put",
        path: &path,
        params: &[],
        headers: &headers_for_sign,
        secret_id: &creds.secret_id,
        secret_key: &creds.secret_key,
        start_time: start,
        expire_seconds: expire,
    });

    info!(
        "[yuanbao] COS PUT bucket={} key={} size={}",
        creds.bucket,
        creds.location,
        data.len()
    );
    let resp = http
        .put(&cos_url)
        .header("Authorization", sig)
        .header("Content-Type", content_type.as_str())
        .header("x-cos-security-token", &creds.session_token)
        .body(data.to_vec())
        .send()
        .await
        .map_err(|e| YuanbaoError::Connection(format!("COS PUT: {e}")))?;
    if !resp.status().is_success() {
        return Err(YuanbaoError::Media(format!(
            "COS PUT HTTP {}",
            resp.status()
        )));
    }

    let dims = if content_type.starts_with("image/") {
        parse_image_size(data).unwrap_or(ImageDims {
            width: 0,
            height: 0,
        })
    } else {
        ImageDims {
            width: 0,
            height: 0,
        }
    };

    let uuid = {
        let mut h = Sha1::new();
        h.update(data);
        hex::encode(h.finalize())
    };
    let url = if creds.resource_url.is_empty() {
        cos_url
    } else {
        creds.resource_url.clone()
    };
    debug!("[yuanbao] COS upload ok url={url}");
    Ok(UploadResult {
        url,
        uuid,
        size: data.len() as u64,
        width: dims.width,
        height: dims.height,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_sign_is_deterministic() {
        let s = cos_sign(&CosSignInput {
            method: "put",
            path: "/test/file.bin",
            params: &[],
            headers: &[("host", "bucket.cos.example.com")],
            secret_id: "AKID",
            secret_key: "SK",
            start_time: 1_700_000_000,
            expire_seconds: 3600,
        });
        let s2 = cos_sign(&CosSignInput {
            method: "put",
            path: "/test/file.bin",
            params: &[],
            headers: &[("host", "bucket.cos.example.com")],
            secret_id: "AKID",
            secret_key: "SK",
            start_time: 1_700_000_000,
            expire_seconds: 3600,
        });
        assert_eq!(s, s2);
        assert!(s.starts_with("q-sign-algorithm=sha1"));
        assert!(s.contains("q-ak=AKID"));
        assert!(s.contains("q-sign-time=1700000000;1700003600"));
    }

    #[test]
    fn cos_sign_changes_with_path() {
        let base = CosSignInput {
            method: "put",
            path: "/a",
            params: &[],
            headers: &[("host", "h")],
            secret_id: "AKID",
            secret_key: "SK",
            start_time: 1_700_000_000,
            expire_seconds: 3600,
        };
        let s1 = cos_sign(&base);
        let s2 = cos_sign(&CosSignInput { path: "/b", ..base });
        assert_ne!(s1, s2);
    }

    #[test]
    fn cos_sign_lowercases_method_and_includes_url_params() {
        let s = cos_sign(&CosSignInput {
            method: "PUT", // mixed case → should be lowercased into sig
            path: "/k",
            params: &[("Foo", "Bar Baz")], // url-encoded value
            headers: &[("Host", "h")],
            secret_id: "AKID",
            secret_key: "SK",
            start_time: 1_700_000_000,
            expire_seconds: 600,
        });
        assert!(s.contains("q-url-param-list=foo"));
        // header list also lowercased
        assert!(s.contains("q-header-list=host"));
    }

    fn ok_credentials_body(bucket: &str, location: &str) -> serde_json::Value {
        serde_json::json!({
            "code": 0,
            "data": {
                "bucketName": bucket,
                "region": "ap-shanghai",
                "location": location,
                "encryptTmpSecretId": "AKID",
                "encryptTmpSecretKey": "SECRET",
                "encryptToken": "session-token",
                "startTime": 1_700_000_000u64,
                "expiredTime": 1_700_003_600u64,
                "resourceUrl": "https://cdn.example/r",
            }
        })
    }

    #[tokio::test]
    async fn get_cos_credentials_parses_data_block() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(UPLOAD_INFO_PATH))
            .and(wiremock::matchers::header("X-Token", "tok"))
            .and(wiremock::matchers::header("X-Source", "web"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(ok_credentials_body("bkt-1", "k/v/file.png")),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let creds = get_cos_credentials(&http, &server.uri(), "appk", "bot", "tok", "", "file.png")
            .await
            .unwrap();
        assert_eq!(creds.bucket, "bkt-1");
        assert_eq!(creds.region, "ap-shanghai");
        assert_eq!(creds.location, "k/v/file.png");
        assert_eq!(creds.secret_id, "AKID");
        assert_eq!(creds.secret_key, "SECRET");
        assert_eq!(creds.session_token, "session-token");
        assert_eq!(creds.resource_url, "https://cdn.example/r");
        assert_eq!(creds.start_time, 1_700_000_000);
        assert_eq!(creds.expired_time, 1_700_003_600);
    }

    #[tokio::test]
    async fn get_cos_credentials_falls_back_to_app_key_for_xid_when_bot_id_empty() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(UPLOAD_INFO_PATH))
            .and(wiremock::matchers::header("X-ID", "appk"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(ok_credentials_body("bkt", "loc")),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let creds = get_cos_credentials(&http, &server.uri(), "appk", "", "tok", "", "f")
            .await
            .unwrap();
        assert_eq!(creds.bucket, "bkt");
    }

    #[tokio::test]
    async fn get_cos_credentials_sends_route_env_header_when_non_empty() {
        let server = wiremock::MockServer::start().await;
        // Bind the matcher to both the upload-info path AND the header so
        // this test fails if a future refactor routes the call elsewhere
        // but happens to still attach `X-Route-Env: canary` somewhere.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(UPLOAD_INFO_PATH))
            .and(wiremock::matchers::header("X-Route-Env", "canary"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(ok_credentials_body("bkt", "loc")),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        get_cos_credentials(&http, &server.uri(), "appk", "bot", "tok", "canary", "f")
            .await
            .expect("should send canary header");
    }

    #[tokio::test]
    async fn get_cos_credentials_surfaces_http_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = get_cos_credentials(&http, &server.uri(), "appk", "bot", "tok", "", "f")
            .await
            .unwrap_err();
        match err {
            YuanbaoError::Media(m) => assert!(m.contains("HTTP 500"), "got {m}"),
            other => panic!("expected Media error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_cos_credentials_surfaces_non_zero_business_code() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": 4001,
                    "msg": "quota",
                })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = get_cos_credentials(&http, &server.uri(), "appk", "bot", "tok", "", "f")
            .await
            .unwrap_err();
        match err {
            YuanbaoError::Media(m) => {
                assert!(m.contains("code=4001"), "got {m}");
                assert!(m.contains("quota"), "got {m}");
            }
            other => panic!("expected Media error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upload_to_cos_rejects_missing_credentials() {
        let http = reqwest::Client::new();
        // empty credentials → fail without making any HTTP call
        let bad = CosCredentials::default();
        let err = upload_to_cos(
            &http,
            &bad,
            b"data",
            "f.bin",
            "application/octet-stream".into(),
        )
        .await
        .unwrap_err();
        match err {
            YuanbaoError::Media(m) => assert!(m.contains("credentials missing"), "got {m}"),
            other => panic!("expected Media error, got {other:?}"),
        }
    }

    // NOTE: upload_to_cos always targets `<bucket>.cos.accelerate.myqcloud.com`
    // which we cannot redirect at the reqwest layer without DNS hacks, so we
    // only cover the guard branch (missing creds) above. The PUT body itself
    // is exercised by integration tests, not unit tests.

    #[test]
    fn encode_cos_key_keeps_slashes_but_escapes_segments() {
        assert_eq!(encode_cos_key("plain/file.png"), "plain/file.png");
        assert_eq!(encode_cos_key("a b/c d.png"), "a%20b/c%20d.png");
    }
}
